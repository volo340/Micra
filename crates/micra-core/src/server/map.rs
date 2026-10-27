//! Mapa web del mundo con BlueMap (https://bluemap.bluecolored.de).
//!
//! BlueMap es un mod (Fabric, Forge o NeoForge) que dibuja el mundo en 3D y
//! lo sirve en una web local (`http://localhost:<puerto>`). Necesita
//! descargar recursos del juego de Mojang, y su licencia pide que la persona
//! lo acepte (`accept-download`): Micra solo lo activa con su permiso.

use std::path::Path;

use crate::providers::modrinth::ModrinthClient;
use crate::server::config::ServerConfig;
use crate::server::mods::{self, ModInfo};
use crate::{tr, Error, Result};

pub const MODRINTH_PROJECT: &str = "bluemap";
const CORE_CONF: &str = "config/bluemap/core.conf";
const WEBSERVER_CONF: &str = "config/bluemap/webserver.conf";

pub fn supported(cfg: &ServerConfig) -> bool {
    cfg.loader.supports_mods()
}

/// El jar de BlueMap entre los mods instalados.
pub fn find_installed(list: &[ModInfo]) -> Option<&ModInfo> {
    list.iter().find(|m| {
        m.mod_id.as_deref() == Some("bluemap")
            || m.file_name.to_ascii_lowercase().starts_with("bluemap")
    })
}

pub async fn install(
    http: &reqwest::Client,
    modrinth: &ModrinthClient,
    server_dir: &Path,
    cfg: &ServerConfig,
) -> Result<Vec<String>> {
    if !supported(cfg) {
        return Err(Error::InvalidConfig(tr!(
            "el mapa necesita un servidor con Fabric, Forge o NeoForge",
            "the map needs a server with Fabric, Forge or NeoForge"
        )));
    }
    let list = mods::list_mods(server_dir).await?;
    if let Some(existing) = find_installed(&list) {
        if !existing.enabled {
            mods::set_enabled(server_dir, &existing.file_name, true).await?;
        }
        return Ok(Vec::new());
    }
    mods::install_from_modrinth(http, modrinth, server_dir, cfg, MODRINTH_PROJECT, None).await
}

/// Desactiva BlueMap (sin borrar el mapa ya dibujado).
pub async fn disable(server_dir: &Path) -> Result<()> {
    let list = mods::list_mods(server_dir).await?;
    if let Some(existing) = find_installed(&list) {
        if existing.enabled {
            mods::set_enabled(server_dir, &existing.file_name, false).await?;
        }
    }
    Ok(())
}

/// Ajusta la configuración que BlueMap crea en su primer arranque: acepta
/// la descarga (si la persona lo aceptó) y fija el puerto. Devuelve `true`
/// si cambió algo (entonces hay que mandar `bluemap reload`).
pub async fn apply_config(server_dir: &Path, cfg: &ServerConfig) -> Result<bool> {
    let core = server_dir.join(CORE_CONF);
    let web = server_dir.join(WEBSERVER_CONF);
    for path in [&core, &web] {
        crate::fsutil::check_no_links(server_dir, path)?;
    }
    let mut changed = false;
    if cfg.map.accepted_download {
        changed |= patch_file(&core, "accept-download", "true").await?;
    }
    changed |= patch_file(&web, "port", &cfg.map.port.to_string()).await?;
    Ok(changed)
}

/// Pone `key: value` en un archivo HOCON de BlueMap conservando el resto.
/// Si el archivo no existe todavía no hace nada.
async fn patch_file(path: &Path, key: &str, value: &str) -> Result<bool> {
    let Ok(content) = tokio::fs::read_to_string(path).await else {
        return Ok(false);
    };
    let patched = patch_hocon(&content, key, value);
    if patched == content {
        return Ok(false);
    }
    crate::fsutil::write_atomic(path, patched.as_bytes()).await?;
    Ok(true)
}

/// `true` si la línea asigna `key` (no un comentario ni otra clave que
/// empiece igual).
fn is_key_line(line: &str, key: &str) -> bool {
    line.trim_start()
        .strip_prefix(key)
        .is_some_and(|rest| rest.trim_start().starts_with([':', '=']))
}

fn patch_hocon(content: &str, key: &str, value: &str) -> String {
    let mut found = false;
    let mut out: Vec<String> = content
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            let is_key = is_key_line(line, key);
            if is_key && !found {
                found = true;
                let indent = &line[..line.len() - trimmed.len()];
                format!("{indent}{key}: {value}")
            } else {
                line.to_string()
            }
        })
        .collect();
    if !found {
        out.push(format!("{key}: {value}"));
    }
    let mut text = out.join("\n");
    if content.ends_with('\n') || !found {
        text.push('\n');
    }
    text
}

pub fn url(cfg: &ServerConfig) -> String {
    format!("http://localhost:{}", cfg.map.port)
}

/// BlueMap pide aceptar la descarga en la consola.
pub fn is_accept_prompt(msg: &str) -> bool {
    msg.contains("accept-download") || msg.contains("you need to accept")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_keys_and_keeps_comments() {
        let core = "# BlueMap core\n# accept-download: (docs)\naccept-download: false\nrender-thread-count: 1\n";
        let out = patch_hocon(core, "accept-download", "true");
        assert_eq!(
            out,
            "# BlueMap core\n# accept-download: (docs)\naccept-download: true\nrender-thread-count: 1\n"
        );
        let web = "enabled: true\n  port: 8100\n";
        assert_eq!(
            patch_hocon(web, "port", "8101"),
            "enabled: true\n  port: 8101\n"
        );
        // `portal-stuff` no es `port`.
        assert_eq!(
            patch_hocon("portal-stuff: 1\n", "port", "8101"),
            "portal-stuff: 1\nport: 8101\n"
        );
    }

    #[test]
    fn missing_files_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut cfg: ServerConfig = serde_json::from_value(serde_json::json!({
            "id": "m", "name": "M", "mcVersion": "1.21.1",
            "loader": {"kind": "fabric", "loaderVersion": "0.16.9"},
            "javaMajor": 21, "memory": {"minMb": 1024, "maxMb": 2048}, "port": 25565,
            "eulaAccepted": true, "launch": {"kind": "jar", "path": "x.jar"}
        }))
        .unwrap();
        cfg.map.accepted_download = true;
        assert!(!rt.block_on(apply_config(dir.path(), &cfg)).unwrap());
        let conf = dir.path().join(CORE_CONF);
        std::fs::create_dir_all(conf.parent().unwrap()).unwrap();
        std::fs::write(&conf, "accept-download: false\n").unwrap();
        assert!(rt.block_on(apply_config(dir.path(), &cfg)).unwrap());
        assert_eq!(
            std::fs::read_to_string(&conf).unwrap(),
            "accept-download: true\n"
        );
        assert!(!rt.block_on(apply_config(dir.path(), &cfg)).unwrap());
    }
}
