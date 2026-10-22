//! Mods y modpack: lista, activar o quitar, instalar desde Modrinth,
//! actualizaciones con un clic y análisis de malware.

use std::sync::Arc;

use micra_core::providers::modrinth::{ProjectKind, SearchQuery, SearchResponse};
use micra_core::provision::{ProvisionStep, Provisioner};
use micra_core::server::config::ServerConfig;
use micra_core::server::mods::{self, ModInfo, ModRemoteInfo};
use micra_core::server::pack::{self, PackManifest, PackUpdate, UpdateTarget};
use micra_core::server::scan::{self, ScanReport};
use micra_core::{tr, Error};
use tauri::{AppHandle, Emitter, State};

use super::CmdResult;
use crate::events::{self, Task};
use crate::state::Shared;
use crate::supervisor::{self, server_dir};

#[tauri::command]
pub async fn list_mods(state: State<'_, Shared>, id: String) -> CmdResult<Vec<ModInfo>> {
    let dir = server_dir(&state, &id)?;
    mods::list_mods(&dir).await
}

#[tauri::command]
pub async fn set_mod_enabled(
    state: State<'_, Shared>,
    id: String,
    file_name: String,
    enabled: bool,
) -> CmdResult<String> {
    let dir = server_dir(&state, &id)?;
    state.ensure_stopped(&id).await?;
    mods::set_enabled(&dir, &file_name, enabled).await
}

#[tauri::command]
pub async fn remove_mod(state: State<'_, Shared>, id: String, file_name: String) -> CmdResult<()> {
    let dir = server_dir(&state, &id)?;
    state.ensure_stopped(&id).await?;
    mods::remove_mod(&dir, &file_name).await
}

/// Qué mods tienen versión nueva (y actualiza el aviso del panel).
#[tauri::command]
pub async fn check_mod_updates(
    app: AppHandle,
    state: State<'_, Shared>,
    id: String,
) -> CmdResult<Vec<ModRemoteInfo>> {
    let dir = server_dir(&state, &id)?;
    let cfg = ServerConfig::load(&dir).await?;
    let list = mods::list_mods(&dir).await?;
    let remote = if list.is_empty() {
        Vec::new()
    } else {
        mods::check_updates(&state.modrinth, &cfg, &list).await?
    };
    let count = remote.iter().filter(|m| m.update.is_some()).count();
    let live = state.update_live(&id, |l| l.mod_updates = count);
    events::live(&app, &id, &live);
    Ok(remote)
}

#[tauri::command]
pub async fn update_mod(
    state: State<'_, Shared>,
    id: String,
    file_name: String,
    version_id: String,
) -> CmdResult<String> {
    let dir = server_dir(&state, &id)?;
    let _busy = state.begin_busy(&id)?;
    state.ensure_stopped(&id).await?;
    let cfg = ServerConfig::load(&dir).await?;
    mods::update_mod(
        &state.http,
        &state.modrinth,
        &dir,
        &cfg,
        &file_name,
        &version_id,
    )
    .await
}

/// Busca mods en Modrinth que valgan para este servidor.
#[tauri::command]
pub async fn search_mods(
    state: State<'_, Shared>,
    id: String,
    query: String,
    offset: Option<u32>,
) -> CmdResult<SearchResponse> {
    let dir = server_dir(&state, &id)?;
    let cfg = ServerConfig::load(&dir).await?;
    let mut loaders = cfg.loader.modrinth_loaders(&cfg.mc_version).into_iter();
    let Some(loader) = loaders.next() else {
        return Err(Error::InvalidConfig(tr!(
            "un servidor vanilla no puede cargar mods",
            "a vanilla server cannot load mods"
        )));
    };
    let query = SearchQuery {
        query,
        kind: ProjectKind::Mod,
        game_version: Some(cfg.mc_version.clone()),
        loader: Some(loader.to_string()),
        extra_loaders: loaders.map(str::to_string).collect(),
        offset: offset.unwrap_or(0),
        limit: 20,
    };
    state.modrinth.search(&query).await
}

#[tauri::command]
pub async fn install_mod(
    state: State<'_, Shared>,
    id: String,
    project_id: String,
    version_id: Option<String>,
) -> CmdResult<Vec<String>> {
    let dir = server_dir(&state, &id)?;
    let _busy = state.begin_busy(&id)?;
    state.ensure_stopped(&id).await?;
    let cfg = ServerConfig::load(&dir).await?;
    mods::install_from_modrinth(
        &state.http,
        &state.modrinth,
        &dir,
        &cfg,
        &project_id,
        version_id.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn scan_mods(
    app: AppHandle,
    state: State<'_, Shared>,
    id: String,
) -> CmdResult<ScanReport> {
    let dir = server_dir(&state, &id)?;
    let progress = {
        let (app, id) = (app.clone(), id.clone());
        Arc::new(move |done: usize, total: usize| {
            events::task(&app, Some(&id), Task::Scan, done as u64, total as u64)
        })
    };
    scan::scan_server(&dir, Some(progress)).await
}

#[tauri::command]
pub async fn quarantine_mod(state: State<'_, Shared>, id: String, path: String) -> CmdResult<()> {
    let dir = server_dir(&state, &id)?;
    state.ensure_stopped(&id).await?;
    scan::quarantine(&dir, &path).await
}

#[tauri::command]
pub async fn check_pack_update(
    app: AppHandle,
    state: State<'_, Shared>,
    id: String,
) -> CmdResult<Option<PackUpdate>> {
    let dir = server_dir(&state, &id)?;
    let cfg = ServerConfig::load(&dir).await?;
    let manifest = PackManifest::load(&dir).await;
    let curseforge = state.curseforge().await;
    let update = pack::check_update(
        &state.modrinth,
        curseforge.as_deref(),
        &cfg,
        manifest.as_ref(),
    )
    .await?;
    let live = state.update_live(&id, |l| l.pack_update = update.clone());
    events::live(&app, &id, &live);
    Ok(update)
}

/// Actualiza el modpack (antes hace una copia completa para poder volver).
#[tauri::command]
pub async fn update_pack(
    app: AppHandle,
    state: State<'_, Shared>,
    id: String,
    target: UpdateTarget,
) -> CmdResult<ServerConfig> {
    let dir = server_dir(&state, &id)?;
    let _busy = state.begin_busy(&id)?;
    state.ensure_stopped(&id).await?;
    let mut cfg = ServerConfig::load(&dir).await?;
    let curseforge = state.curseforge().await;
    let provisioner = Provisioner {
        http: &state.http,
        paths: &state.paths,
        java: &state.java,
        modrinth: &state.modrinth,
        curseforge: curseforge.as_deref(),
    };
    let on_step = {
        let app = app.clone();
        move |step: ProvisionStep| {
            let _ = app.emit("provision://progress", step);
        }
    };
    provisioner
        .update_pack(&dir, &mut cfg, &target, &on_step)
        .await?;
    supervisor::check_updates(&app, &state, &dir, &cfg).await;
    Ok(cfg)
}
