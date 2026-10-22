//! Editor de `server.properties` con las reglas de seguridad de Micra:
//! RCON siempre con contraseña fuerte (generada) y puertos sin choques.
//! El puerto del juego se cambia en los ajustes de Micra, no aquí.

use std::collections::BTreeMap;

use micra_core::server::config::ServerConfig;
use micra_core::server::properties;
use micra_core::{tr, Error};
use serde::Serialize;
use tauri::State;

use super::CmdResult;
use crate::state::Shared;
use crate::supervisor::server_dir;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertiesView {
    values: BTreeMap<String, String>,
    /// El servidor aún no ha arrancado nunca (el archivo se crea entonces).
    exists: bool,
}

async fn view(dir: &std::path::Path) -> CmdResult<PropertiesView> {
    Ok(PropertiesView {
        values: properties::read_properties(dir).await?,
        exists: dir.join(properties::FILE).exists(),
    })
}

#[tauri::command]
pub async fn get_properties(state: State<'_, Shared>, id: String) -> CmdResult<PropertiesView> {
    view(&server_dir(&state, &id)?).await
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

    let mut values: Vec<(&str, String)> = changes
        .iter()
        .map(|(k, v)| (k.as_str(), v.trim().to_string()))
        .collect();
    // RCON nunca con una contraseña débil: se genera una al activarlo.
    let rcon_on = merged("enable-rcon").is_some_and(|v| v.trim() == "true");
    let weak = properties::is_weak_password(merged("rcon.password").map_or("", |p| p.as_str()));
    if rcon_on && weak {
        values.retain(|(k, _)| *k != "rcon.password");
        values.push(("rcon.password", properties::random_password()));
    }
    properties::set_properties(&dir, &values).await?;
    view(&dir).await
}
