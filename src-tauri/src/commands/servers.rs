//! Servidores: lista, creación, arranque y parada, consola y ajustes.

use std::time::Duration;

use chrono::{DateTime, Local};
use micra_core::net::playit;
use micra_core::provision::{
    self, check_port_free_among, list_servers as list_server_dirs, used_ports, NewServerRequest,
    ProvisionStep, Provisioner,
};
use micra_core::secrets::SecretKey;
use micra_core::server::config::{
    validate_resources, BackupPolicy, CpuConfig, MemoryConfig, NetworkPolicy, SchedulePolicy,
    SecurityPolicy, ServerConfig,
};
use micra_core::server::process::ServerStatus;
use micra_core::{hardware, tr, Error};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use super::{open_path, CmdResult};
use crate::state::{LiveInfo, Shared, StopReason};
use crate::supervisor::{self, server_dir};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSummary {
    config: ServerConfig,
    status: ServerStatus,
    players: Vec<String>,
    dir: String,
    live: LiveInfo,
    busy: bool,
    /// Cuándo arrancó (solo si está encendido).
    started_at: Option<DateTime<Local>>,
}

#[tauri::command]
pub async fn list_servers(state: State<'_, Shared>) -> CmdResult<Vec<ServerSummary>> {
    let running = state.running.read().await;
    Ok(list_server_dirs(&state.paths)
        .await?
        .into_iter()
        .map(|(saved, dir)| {
            let server = running.get(&saved.id);
            let status = server.map_or(ServerStatus::Stopped, |s| s.handle.status());
            let alive = server.filter(|s| s.handle.is_alive());
            ServerSummary {
                started_at: alive.map(|s| s.started_at),
                status,
                players: server.map(|s| s.handle.players()).unwrap_or_default(),
                dir: dir.display().to_string(),
                live: state.live(&saved.id),
                busy: state.is_busy(&saved.id),
                config: saved,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn create_server(
    app: AppHandle,
    state: State<'_, Shared>,
    request: NewServerRequest,
) -> CmdResult<ServerConfig> {
    let curseforge = state.curseforge().await;
    let provisioner = Provisioner {
        http: &state.http,
        paths: &state.paths,
        java: &state.java,
        modrinth: &state.modrinth,
        curseforge: curseforge.as_deref(),
    };
    let on_step = move |step: ProvisionStep| {
        let _ = app.emit("provision://progress", step);
    };
    provisioner.create(request, &on_step).await
}

/// Borra un servidor por completo (y sus copias si se pide). Tiene que
/// estar apagado.
#[tauri::command]
pub async fn delete_server(
    state: State<'_, Shared>,
    id: String,
    delete_backups: bool,
) -> CmdResult<()> {
    let dir = server_dir(&state, &id)?;
    let _busy = state.begin_busy(&id)?;
    state.ensure_stopped(&id).await?;
    let cfg = ServerConfig::load(&dir).await?;
    // Su túnel de playit.gg ya no hará falta; si no se puede borrar ahora,
    // se queda en la cuenta sin estorbar.
    if cfg.network.tunnel {
        if let Ok(Some(secret)) = state.secrets.get(SecretKey::PlayitAgent).await {
            let deleted = tokio::time::timeout(
                Duration::from_secs(10),
                playit::delete_tunnel(&state.http, &secret, &id),
            )
            .await;
            if !matches!(deleted, Ok(Ok(()))) {
                tracing::warn!("no se pudo borrar el túnel de playit.gg de {id}");
            }
        }
    }
    provision::delete_server(&state.paths, &id, delete_backups).await?;
    state.running.write().await.remove(&id);
    state.live.lock().unwrap().remove(&id);
    Ok(())
}

#[tauri::command]
pub async fn start_server(app: AppHandle, state: State<'_, Shared>, id: String) -> CmdResult<()> {
    supervisor::start(&app, &state, &id).await
}

#[tauri::command]
pub async fn stop_server(state: State<'_, Shared>, id: String) -> CmdResult<()> {
    supervisor::stop(&state, &id, StopReason::User).await
}

/// Apaga y vuelve a encender (la vigilancia del servidor lo arranca de nuevo
/// en cuanto termina de apagarse).
#[tauri::command]
pub async fn restart_server(state: State<'_, Shared>, id: String) -> CmdResult<()> {
    supervisor::stop(&state, &id, StopReason::Restart).await
}

#[tauri::command]
pub async fn send_command(state: State<'_, Shared>, id: String, command: String) -> CmdResult<()> {
    let server = state.alive(&id).await?;
    server.handle.send_command(&command).await
}

#[tauri::command]
pub fn open_server_folder(app: AppHandle, state: State<'_, Shared>, id: String) -> CmdResult<()> {
    open_path(&app, &server_dir(&state, &id)?)
}

/// Lo que se edita en la pestaña Ajustes del servidor.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSettings {
    memory: MemoryConfig,
    cpu: CpuConfig,
    port: u16,
    use_aikar_flags: bool,
    backups: BackupPolicy,
    schedule: SchedulePolicy,
    network: NetworkPolicy,
    security: SecurityPolicy,
    map_port: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSaved {
    config: ServerConfig,
    /// Hay cambios (RAM, núcleos, puerto, aislamiento) que se aplican al
    /// reiniciar el servidor.
    restart_needed: bool,
}

#[tauri::command]
pub async fn update_server_settings(
    state: State<'_, Shared>,
    id: String,
    settings: ServerSettings,
) -> CmdResult<SettingsSaved> {
    let dir = server_dir(&state, &id)?;
    let mut cfg = ServerConfig::load(&dir).await?;
    let hw = hardware::hardware_info();
    validate_resources(
        &settings.memory,
        &settings.cpu,
        settings.port,
        hw.total_memory_mb,
        hw.logical_cpus,
    )?;
    if settings.port != cfg.port {
        check_port_free_among(settings.port, &used_ports(&state.paths, Some(&id)))?;
    }
    let other_maps: Vec<u16> = list_server_dirs(&state.paths)
        .await?
        .into_iter()
        .filter(|(c, _)| c.id != id && c.map.enabled)
        .map(|(c, _)| c.map.port)
        .collect();
    let port = settings.map_port;
    if port != cfg.map.port && other_maps.contains(&port) {
        return Err(Error::InvalidConfig(tr!(
            "otro servidor ya usa el puerto {port} para su mapa",
            "another server already uses port {port} for its map"
        )));
    }

    let restart_sensitive = |c: &ServerConfig| {
        (
            c.memory.min_mb,
            c.memory.max_mb,
            c.cpu.cores.clone(),
            c.port,
            c.use_aikar_flags,
            c.security.sandbox,
            c.security.low_integrity,
            c.map.port,
        )
    };
    let before = restart_sensitive(&cfg);
    cfg.memory = settings.memory;
    cfg.cpu = settings.cpu;
    cfg.port = settings.port;
    cfg.use_aikar_flags = settings.use_aikar_flags;
    cfg.backups = settings.backups;
    cfg.schedule = settings.schedule;
    cfg.network = settings.network;
    cfg.security = settings.security;
    cfg.map.port = settings.map_port;
    cfg.validate_policies()?;
    let changed_for_restart = before != restart_sensitive(&cfg);
    cfg.save(&dir).await?;

    // En marcha: las políticas se aplican ya; lo demás, al reiniciar.
    let mut restart_needed = false;
    if let Some(server) = state.server(&id).await.filter(|s| s.handle.is_alive()) {
        restart_needed = changed_for_restart;
        server.config.send_modify(|live| {
            live.backups = cfg.backups.clone();
            live.schedule = cfg.schedule.clone();
            live.network = cfg.network.clone();
            live.security.guard = cfg.security.guard.clone();
        });
    }
    Ok(SettingsSaved {
        config: cfg,
        restart_needed,
    })
}
