//! Editor de `server.properties` con las reglas de seguridad de Micra:
//! RCON siempre con contraseña fuerte (generada) y puertos sin choques.
//! El puerto del juego se cambia en los ajustes de Micra, no aquí.
//!
//! Algunas opciones viven en el mundo y no en el archivo: el modo extremo
//! de un mundo ya creado y, desde Minecraft 1.21.9, las propiedades que se
//! convirtieron en reglas de juego (ver `server::gamerules`).

use std::collections::BTreeMap;
use std::path::Path;

use micra_core::server::config::ServerConfig;
use micra_core::server::process::ServerStatus;
use micra_core::server::{gamerules, properties, world};
use micra_core::{tr, Error};
use serde::Serialize;
use tauri::State;

use super::CmdResult;
use crate::state::Shared;
use crate::supervisor::{save_world_tweaks, server_dir, set_rule};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertiesView {
    /// Lo del archivo, con los valores que guarda el mundo donde mandan.
    values: BTreeMap<String, String>,
    /// El servidor aún no ha arrancado nunca (el archivo se crea entonces).
    exists: bool,
    /// Propiedad → regla de juego que la sustituye en esta versión.
    rules: BTreeMap<String, String>,
    /// Propiedades que esta versión ya no tiene.
    removed: Vec<String>,
    /// Cambios para el mundo que esperan al próximo arranque.
    pending: Vec<String>,
}

async fn view(state: &Shared, id: &str, dir: &Path) -> CmdResult<PropertiesView> {
    let cfg = ServerConfig::load(dir).await?;
    let mut values = properties::read_properties(dir).await?;
    let exists = dir.join(properties::FILE).exists();
    let moved = gamerules::moved(&cfg.mc_version);

    // Encendido, el mundo está en memoria y sus archivos pueden ir atrasados.
    let running = state.server(id).await.filter(|s| s.handle.is_alive());
    let (hardcore, rules) = match running {
        Some(server) => {
            let world = server.world.lock().unwrap();
            (world.hardcore, world.rules.clone())
        }
        None => {
            let world_dir = world::world_dir(dir, &values);
            let props: Vec<&str> = moved.iter().map(|(p, _)| *p).collect();
            (
                world::read_hardcore(&world_dir).await,
                world::read_rules(&world_dir, &props).await,
            )
        }
    };
    let tweaks = &cfg.world;
    for (property, _) in &moved {
        let value = tweaks
            .pending_rules
            .get(*property)
            .or_else(|| rules.get(*property))
            .copied()
            .unwrap_or(gamerules::DEFAULT);
        values.insert(property.to_string(), value.to_string());
    }
    if let Some(hardcore) = tweaks.pending_hardcore.or(hardcore) {
        values.insert("hardcore".into(), hardcore.to_string());
    }

    let mut pending: Vec<String> = tweaks.pending_rules.keys().cloned().collect();
    if tweaks.pending_hardcore.is_some() {
        pending.push("hardcore".into());
    }
    Ok(PropertiesView {
        values,
        exists,
        rules: moved
            .into_iter()
            .map(|(p, r)| (p.to_string(), r.to_string()))
            .collect(),
        removed: gamerules::removed(&cfg.mc_version)
            .into_iter()
            .map(String::from)
            .collect(),
        pending,
    })
}

#[tauri::command]
pub async fn get_properties(state: State<'_, Shared>, id: String) -> CmdResult<PropertiesView> {
    let dir = server_dir(&state, &id)?;
    view(&state, &id, &dir).await
}

#[tauri::command]
pub async fn save_properties(
    state: State<'_, Shared>,
    id: String,
    changes: BTreeMap<String, String>,
) -> CmdResult<PropertiesView> {
    let dir = server_dir(&state, &id)?;
    let cfg = ServerConfig::load(&dir).await?;
    if changes.contains_key("server-port") {
        return Err(Error::InvalidConfig(tr!(
            "el puerto del juego se cambia en Ajustes del servidor > Red",
            "change the game port in Server settings > Network"
        )));
    }
    for (key, value) in &changes {
        properties::validate(key, value)?;
    }
    let current = properties::read_properties(&dir).await?;
    let merged = |key: &str| changes.get(key).or_else(|| current.get(key));

    for key in ["rcon.port", "query.port"] {
        let Some(port) = changes.get(key).and_then(|p| p.trim().parse::<u16>().ok()) else {
            continue;
        };
        if port == cfg.port || (cfg.map.enabled && port == cfg.map.port) {
            return Err(Error::InvalidConfig(tr!(
                "«{key}» no puede usar el puerto {port}: ya lo usa el juego o el mapa",
                "\"{key}\" cannot use port {port}: the game or the map already uses it"
            )));
        }
    }

    // Las reglas de juego no van al archivo, que esta versión ya no lee
    // para ellas; las propiedades quitadas tampoco.
    let mut values: Vec<(&str, String)> = Vec::new();
    let mut rules: Vec<(&str, &'static str, bool)> = Vec::new();
    for (key, value) in &changes {
        let value = value.trim();
        if let Some(rule) = gamerules::rule_for(key, &cfg.mc_version) {
            rules.push((key.as_str(), rule, value == "true"));
        } else if !gamerules::is_removed(key, &cfg.mc_version) {
            values.push((key.as_str(), value.to_string()));
        }
    }
    // RCON nunca con una contraseña débil: se genera una al activarlo.
    let rcon_on = merged("enable-rcon").is_some_and(|v| v.trim() == "true");
    let weak = properties::is_weak_password(merged("rcon.password").map_or("", |p| p.as_str()));
    if rcon_on && weak {
        values.retain(|(k, _)| *k != "rcon.password");
        values.push(("rcon.password", properties::random_password()));
    }
    if !values.is_empty() {
        properties::set_properties(&dir, &values).await?;
    }

    // Encendido, las reglas se cambian ya; si no, al arrancar.
    let running = state
        .server(&id)
        .await
        .filter(|s| s.handle.status() == ServerStatus::Running);
    let mut applied = Vec::new();
    let mut later = Vec::new();
    for (property, rule, value) in rules {
        let now = match &running {
            Some(server) => set_rule(server, property, rule, value).await,
            None => false,
        };
        if now {
            applied.push(property);
        } else {
            later.push((property, value));
        }
    }
    // Un mundo ya creado no lee el modo extremo de server.properties: se
    // escribe en level.dat al arrancar (si aún no existe, lo crea el archivo).
    let hardcore = changes.get("hardcore").map(|v| v.trim() == "true");
    if !applied.is_empty() || !later.is_empty() || hardcore.is_some() {
        save_world_tweaks(&dir, |w| {
            for property in applied {
                w.pending_rules.remove(property);
            }
            for (property, value) in later {
                w.pending_rules.insert(property.to_string(), value);
            }
            if hardcore.is_some() {
                w.pending_hardcore = hardcore;
            }
        })
        .await?;
    }
    view(&state, &id, &dir).await
}
