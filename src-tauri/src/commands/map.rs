//! Mapa web con BlueMap.

use micra_core::server::config::ServerConfig;
use micra_core::server::map;
use tauri::{AppHandle, State};

use super::{open_url, CmdResult};
use crate::events;
use crate::state::Shared;
use crate::supervisor::server_dir;

/// Instala BlueMap. `accept_download`: la persona acepta que descargue los
/// recursos del juego de Mojang (lo exige su licencia).
#[tauri::command]
pub async fn enable_map(
    state: State<'_, Shared>,
    id: String,
    accept_download: bool,
) -> CmdResult<ServerConfig> {
    let dir = server_dir(&state, &id)?;
    let _busy = state.begin_busy(&id)?;
    state.ensure_stopped(&id).await?;
    let mut cfg = ServerConfig::load(&dir).await?;
    map::install(&state.http, &state.modrinth, &dir, &cfg).await?;
    cfg.map.enabled = true;
    cfg.map.accepted_download = accept_download;
    cfg.save(&dir).await?;
    map::apply_config(&dir, &cfg).await?;
    Ok(cfg)
}

#[tauri::command]
pub async fn disable_map(state: State<'_, Shared>, id: String) -> CmdResult<ServerConfig> {
    let dir = server_dir(&state, &id)?;
    state.ensure_stopped(&id).await?;
    let mut cfg = ServerConfig::load(&dir).await?;
    map::disable(&dir).await?;
    cfg.map.enabled = false;
    cfg.save(&dir).await?;
    Ok(cfg)
}

/// Acepta la descarga de BlueMap; con el servidor encendido se aplica ya.
#[tauri::command]
pub async fn accept_map_download(
    app: AppHandle,
    state: State<'_, Shared>,
    id: String,
) -> CmdResult<ServerConfig> {
    let dir = server_dir(&state, &id)?;
    let mut cfg = ServerConfig::load(&dir).await?;
    cfg.map.accepted_download = true;
    cfg.save(&dir).await?;
    if let Some(server) = state.server(&id).await.filter(|s| s.handle.is_alive()) {
        server
            .config
            .send_modify(|c| c.map.accepted_download = true);
        if map::apply_config(&dir, &cfg).await? {
            server.handle.send_command("bluemap reload").await?;
        }
    }
    let live = state.update_live(&id, |l| l.map_needs_accept = false);
    events::live(&app, &id, &live);
    Ok(cfg)
}

#[tauri::command]
pub async fn open_map(app: AppHandle, state: State<'_, Shared>, id: String) -> CmdResult<()> {
    let dir = server_dir(&state, &id)?;
    let cfg = ServerConfig::load(&dir).await?;
    open_url(&app, &map::url(&cfg))
}
