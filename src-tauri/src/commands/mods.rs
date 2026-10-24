//! Mods y modpack: lista, activar o quitar, instalar desde Modrinth o
//! CurseForge, actualizaciones con un clic, análisis de malware y qué mod
//! quitar cuando el servidor no arranca.

use std::sync::Arc;

use micra_core::providers::curseforge::CurseForgeClient;
use micra_core::providers::search::{self, ContentKind, ContentPage, ContentQuery, Cursor, Source};
use micra_core::provision::{ProvisionStep, Provisioner};
use micra_core::server::config::ServerConfig;
use micra_core::server::mods::{self, ModInfo, ModRemoteInfo, Sources};
use micra_core::server::pack::{self, PackManifest, PackUpdate, UpdateTarget};
use micra_core::server::scan::{self, ScanReport};
use micra_core::{tr, Error};
use tauri::{AppHandle, Emitter, State};

use super::{open_path, CmdResult};
use crate::events::{self, Task};
use crate::state::{AppState, Shared};
use crate::supervisor::{self, server_dir};

/// Clientes para buscar y descargar mods (CurseForge solo si hay clave).
fn sources<'a>(state: &'a AppState, curseforge: Option<&'a CurseForgeClient>) -> Sources<'a> {
    Sources {
        http: &state.http,
        modrinth: &state.modrinth,
        curseforge,
    }
}

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
pub async fn remove_mod(
    app: AppHandle,
    state: State<'_, Shared>,
    id: String,
    file_name: String,
) -> CmdResult<()> {
    let dir = server_dir(&state, &id)?;
    state.ensure_stopped(&id).await?;
    mods::remove_mod(&dir, &file_name).await?;
    // Si era uno de los que impedían arrancar, ya no hace falta avisar de él.
    if state.live(&id).crash.is_some() {
        let live = state.update_live(&id, |l| {
            if let Some(crash) = l.crash.as_mut() {
                crash.culprits.retain(|c| c.file_name != file_name);
            }
        });
        events::live(&app, &id, &live);
    }
    Ok(())
}

/// Oculta el aviso de por qué falló el último arranque.
#[tauri::command]
pub fn dismiss_crash(app: AppHandle, state: State<'_, Shared>, id: String) -> CmdResult<()> {
    let live = state.update_live(&id, |l| l.crash = None);
    events::live(&app, &id, &live);
    Ok(())
}

/// Abre el informe de error que dejó Minecraft en `crash-reports/`.
#[tauri::command]
pub fn open_crash_report(app: AppHandle, state: State<'_, Shared>, id: String) -> CmdResult<()> {
    let dir = server_dir(&state, &id)?;
    let file = state
        .live(&id)
        .crash
        .and_then(|c| c.report_file)
        .filter(|f| {
            f.strip_prefix("crash-reports/").is_some_and(|name| {
                !name.is_empty() && !name.contains(['/', '\\']) && !name.starts_with('.')
            })
        })
        .ok_or_else(|| Error::NotFound(tr!("el informe del error", "the crash report")))?;
    open_path(&app, &dir.join(file))
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
        let curseforge = state.curseforge().await;
        mods::check_updates(sources(&state, curseforge.as_deref()), &dir, &cfg, &list).await?
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
    source: Source,
    version_id: String,
) -> CmdResult<String> {
    let dir = server_dir(&state, &id)?;
    let _busy = state.begin_busy(&id)?;
    state.ensure_stopped(&id).await?;
    let cfg = ServerConfig::load(&dir).await?;
    let curseforge = state.curseforge().await;
    let sources = sources(&state, curseforge.as_deref());
    mods::update_mod(sources, &dir, &cfg, &file_name, source, &version_id).await
}

/// Busca en Modrinth y CurseForge mods que valgan para este servidor.
#[tauri::command]
pub async fn search_mods(
    state: State<'_, Shared>,
    id: String,
    query: String,
    cursor: Option<Cursor>,
) -> CmdResult<ContentPage> {
    let dir = server_dir(&state, &id)?;
    let cfg = ServerConfig::load(&dir).await?;
    let loaders = cfg.loader.modrinth_loaders(&cfg.mc_version);
    if loaders.is_empty() {
        return Err(Error::InvalidConfig(tr!(
            "un servidor vanilla no puede cargar mods",
            "a vanilla server cannot load mods"
        )));
    }
    let query = ContentQuery {
        query,
        kind: ContentKind::Mod,
        game_version: Some(cfg.mc_version.clone()),
        loaders: loaders.into_iter().map(str::to_string).collect(),
        cursor,
    };
    let curseforge = state.curseforge().await;
    search::search(&state.modrinth, curseforge.as_deref(), &query).await
}

#[tauri::command]
pub async fn install_mod(
    state: State<'_, Shared>,
    id: String,
    source: Source,
    project_id: String,
    version_id: Option<String>,
) -> CmdResult<Vec<String>> {
    let dir = server_dir(&state, &id)?;
    let _busy = state.begin_busy(&id)?;
    state.ensure_stopped(&id).await?;
    let cfg = ServerConfig::load(&dir).await?;
    let curseforge = state.curseforge().await;
    let sources = sources(&state, curseforge.as_deref());
    mods::install(
        sources,
        &dir,
        &cfg,
        source,
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
