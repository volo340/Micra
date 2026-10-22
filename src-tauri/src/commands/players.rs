//! Jugadores: lista blanca, operadores, baneos y expulsiones.
//!
//! Con el servidor encendido se usan sus comandos de consola (así el propio
//! servidor guarda los cambios); apagado, se editan sus archivos JSON.

use micra_core::server::players::{self, KnownPlayer, PlayerAction, PlayerLists};
use micra_core::server::properties;
use tauri::State;

use super::CmdResult;
use crate::state::Shared;
use crate::supervisor::server_dir;

#[tauri::command]
pub async fn player_lists(state: State<'_, Shared>, id: String) -> CmdResult<PlayerLists> {
    let dir = server_dir(&state, &id)?;
    Ok(players::read_lists(&dir).await)
}

#[tauri::command]
pub async fn player_action(
    state: State<'_, Shared>,
    id: String,
    action: PlayerAction,
    target: String,
) -> CmdResult<()> {
    let dir = server_dir(&state, &id)?;
    let target = target.trim();
    if let Some(server) = state.server(&id).await.filter(|s| s.handle.is_alive()) {
        let command = players::command_for(&action, target)?;
        return server.handle.send_command(&command).await;
    }
    let needs_profile = matches!(
        action,
        PlayerAction::WhitelistAdd | PlayerAction::Op | PlayerAction::Ban { .. }
    );
    let profile = if needs_profile {
        Some(resolve_profile(&state, &dir, target).await?)
    } else {
        None
    };
    players::apply_offline(&dir, &action, target, profile.as_ref()).await
}

/// UUID del jugador: el oficial de Mojang si el servidor comprueba las
/// cuentas, o el que calcula Minecraft en modo sin conexión.
async fn resolve_profile(
    state: &Shared,
    dir: &std::path::Path,
    name: &str,
) -> CmdResult<KnownPlayer> {
    let name = players::validate_name(name)?;
    let props = properties::read_properties(dir).await?;
    if properties::flag(&props, "online-mode", true) {
        players::lookup_profile(&state.http, name).await
    } else {
        Ok(KnownPlayer {
            name: name.to_string(),
            uuid: players::offline_uuid(name),
        })
    }
}
