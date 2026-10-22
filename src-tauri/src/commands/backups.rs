//! Copias de seguridad: crear (también con el servidor encendido), restaurar
//! con un clic, borrar y abrir la carpeta.

use std::sync::Arc;

use micra_core::server::backup::{self, BackupInfo, BackupKind, BackupProgress, BackupScope};
use micra_core::server::config::ServerConfig;
use micra_core::Error;
use tauri::{AppHandle, State};

use super::{open_path, CmdResult};
use crate::events::{self, Task};
use crate::state::Shared;
use crate::supervisor::server_dir;

fn progress(app: &AppHandle, id: &str, task: Task) -> BackupProgress {
    let (app, id) = (app.clone(), id.to_string());
    Arc::new(move |done, total| events::task(&app, Some(&id), task, done, total))
}

#[tauri::command]
pub async fn list_backups(state: State<'_, Shared>, id: String) -> CmdResult<Vec<BackupInfo>> {
    server_dir(&state, &id)?;
    Ok(backup::list_backups(&state.paths.backups_dir(&id)).await)
}

/// Copia manual. Con el servidor encendido solo se copia el mundo (se
/// pausa el guardado mientras tanto); apagado, también la copia completa.
#[tauri::command]
pub async fn create_backup(
    app: AppHandle,
    state: State<'_, Shared>,
    id: String,
    scope: BackupScope,
) -> CmdResult<BackupInfo> {
    let dir = server_dir(&state, &id)?;
    let _busy = state.begin_busy(&id)?;
    let backups_dir = state.paths.backups_dir(&id);
    let on_progress = Some(progress(&app, &id, Task::Backup));
    if let Some(server) = state.server(&id).await.filter(|s| s.handle.is_alive()) {
        if scope == BackupScope::Full {
            return Err(Error::MustBeStopped);
        }
        let cfg = server.config.borrow().clone();
        return backup::create_live(
            &server.handle,
            &dir,
            &backups_dir,
            &cfg,
            BackupKind::Manual,
            on_progress,
        )
        .await;
    }
    let cfg = ServerConfig::load(&dir).await?;
    backup::create_backup(
        &dir,
        &backups_dir,
        &cfg,
        BackupKind::Manual,
        scope,
        on_progress,
    )
    .await
}

#[tauri::command]
pub async fn restore_backup(
    app: AppHandle,
    state: State<'_, Shared>,
    id: String,
    name: String,
) -> CmdResult<()> {
    let dir = server_dir(&state, &id)?;
    let _busy = state.begin_busy(&id)?;
    state.ensure_stopped(&id).await?;
    let cfg = ServerConfig::load(&dir).await?;
    backup::restore_backup(
        &dir,
        &state.paths.backups_dir(&id),
        &cfg,
        &name,
        Some(progress(&app, &id, Task::Restore)),
    )
    .await
}

#[tauri::command]
pub async fn delete_backup(state: State<'_, Shared>, id: String, name: String) -> CmdResult<()> {
    server_dir(&state, &id)?;
    backup::delete_backup(&state.paths.backups_dir(&id), &name).await
}

#[tauri::command]
pub fn open_backups_folder(app: AppHandle, state: State<'_, Shared>, id: String) -> CmdResult<()> {
    server_dir(&state, &id)?;
    let dir = state.paths.backups_dir(&id);
    std::fs::create_dir_all(&dir)?;
    open_path(&app, &dir)
}
