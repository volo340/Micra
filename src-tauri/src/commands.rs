//! Comandos expuestos al frontend (`invoke(...)`) y eventos que emitimos:
//!
//! - `provision://progress`: pasos del asistente de creación.
//! - `server://event`: consola, cambios de estado y jugadores.
//! - `server://stats`: CPU/RAM cada segundo mientras el servidor vive.

use std::sync::Arc;
use std::time::Duration;

use micra_core::hardware::HardwareInfo;
use micra_core::java::JavaRuntime;
use micra_core::provision::{list_servers as list_server_dirs, NewServerRequest, ProvisionStep, Provisioner};
use micra_core::providers::curseforge::{CfClass, CfFile, CfSearchResponse};
use micra_core::providers::modrinth::{SearchQuery, SearchResponse, Version};
use micra_core::providers::mojang::{self, VersionEntry};
use micra_core::server::config::ServerConfig;
use micra_core::server::monitor::{ProcessMonitor, ResourceSample};
use micra_core::server::process::{ServerEvent, ServerHandle, ServerStatus};
use micra_core::Error;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::broadcast::error::RecvError;

use crate::state::AppState;

type CmdResult<T> = Result<T, Error>;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerEventPayload {
    server_id: String,
    event: ServerEvent,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerStatsPayload {
    server_id: String,
    sample: ResourceSample,
    player_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSummary {
    config: ServerConfig,
    status: ServerStatus,
    players: Vec<String>,
}

#[tauri::command]
pub fn hardware_info() -> HardwareInfo {
    micra_core::hardware::hardware_info()
}

/// Versiones "release" de Minecraft, de la más nueva a la más antigua.
#[tauri::command]
pub async fn minecraft_versions(state: State<'_, AppState>) -> CmdResult<Vec<VersionEntry>> {
    let manifest = mojang::fetch_manifest(&state.http).await?;
    Ok(manifest.versions.into_iter().filter(|v| v.kind == "release").collect())
}

#[tauri::command]
pub fn list_java_runtimes(state: State<'_, AppState>) -> Vec<JavaRuntime> {
    state.java.list_installed()
}

#[tauri::command]
pub async fn search_modrinth(state: State<'_, AppState>, query: SearchQuery) -> CmdResult<SearchResponse> {
    state.modrinth.search(&query).await
}

#[tauri::command]
pub async fn modrinth_versions(
    state: State<'_, AppState>,
    project_id: String,
    game_version: Option<String>,
    loader: Option<String>,
) -> CmdResult<Vec<Version>> {
    state
        .modrinth
        .project_versions(&project_id, game_version.as_deref(), loader.as_deref())
        .await
}

#[tauri::command]
pub fn curseforge_enabled(state: State<'_, AppState>) -> bool {
    state.curseforge.is_some()
}

#[tauri::command]
pub async fn search_curseforge(
    state: State<'_, AppState>,
    query: String,
    class: CfClass,
    game_version: Option<String>,
    index: Option<u32>,
) -> CmdResult<CfSearchResponse> {
    let cf = state.curseforge.as_ref().ok_or_else(curseforge_missing)?;
    cf.search(&query, class, game_version.as_deref(), index.unwrap_or(0), 20).await
}

#[tauri::command]
pub async fn curseforge_files(
    state: State<'_, AppState>,
    mod_id: u64,
    game_version: Option<String>,
) -> CmdResult<Vec<CfFile>> {
    let cf = state.curseforge.as_ref().ok_or_else(curseforge_missing)?;
    cf.files(mod_id, game_version.as_deref()).await
}

#[tauri::command]
pub async fn list_servers(state: State<'_, AppState>) -> CmdResult<Vec<ServerSummary>> {
    let running = state.running.read().await;
    Ok(list_server_dirs(&state.paths)
        .await?
        .into_iter()
        .map(|(config, _)| {
            let handle = running.get(&config.id);
            ServerSummary {
                status: handle.map_or(ServerStatus::Stopped, |h| h.status()),
                players: handle.map(|h| h.players()).unwrap_or_default(),
                config,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn create_server(
    app: AppHandle,
    state: State<'_, AppState>,
    request: NewServerRequest,
) -> CmdResult<ServerConfig> {
    let provisioner = Provisioner {
        http: &state.http,
        paths: &state.paths,
        java: &state.java,
        modrinth: &state.modrinth,
        curseforge: state.curseforge.as_ref(),
    };
    let on_step = move |step: ProvisionStep| {
        let _ = app.emit("provision://progress", step);
    };
    provisioner.create(request, &on_step).await
}

#[tauri::command]
pub async fn start_server(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    if state.running.read().await.get(&id).is_some_and(|h| h.is_alive()) {
        return Err(Error::AlreadyRunning);
    }
    let dir = state.paths.server_dir(&id);
    let config = ServerConfig::load(&dir).await?;
    let java = state.java.ensure(config.java_major, None).await?;
    let (handle, mut events) = ServerHandle::launch(&config, &dir, &java).await?;
    let handle = Arc::new(handle);

    // Reenviar consola y estado al frontend.
    {
        let app = app.clone();
        let server_id = id.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        let exited = matches!(event, ServerEvent::Exited { .. });
                        let payload = ServerEventPayload { server_id: server_id.clone(), event };
                        let _ = app.emit("server://event", payload);
                        if exited {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "la consola va más rápido que la interfaz");
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        });
    }

    // Muestreo de CPU/RAM.
    if let Some(pid) = handle.pid() {
        let handle = handle.clone();
        let server_id = id.clone();
        tauri::async_runtime::spawn(async move {
            let mut monitor = ProcessMonitor::new(pid);
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            while handle.is_alive() {
                tick.tick().await;
                if let Some(sample) = monitor.sample() {
                    let payload = ServerStatsPayload {
                        server_id: server_id.clone(),
                        sample,
                        player_count: handle.players().len(),
                    };
                    let _ = app.emit("server://stats", payload);
                }
            }
        });
    }

    state.running.write().await.insert(id, handle);
    Ok(())
}

#[tauri::command]
pub async fn stop_server(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let handle = state.running.read().await.get(&id).cloned().ok_or(Error::NotRunning)?;
    handle.stop(Duration::from_secs(60)).await
}

#[tauri::command]
pub async fn send_command(state: State<'_, AppState>, id: String, command: String) -> CmdResult<()> {
    let handle = state.running.read().await.get(&id).cloned().ok_or(Error::NotRunning)?;
    handle.send_command(&command).await
}

fn curseforge_missing() -> Error {
    Error::InvalidConfig(
        "CurseForge no está configurado: define MICRA_CURSEFORGE_API_KEY".into(),
    )
}
