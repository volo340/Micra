//! Importar un servidor que ya existe en otra carpeta.

use std::path::Path;
use std::sync::Arc;

use micra_core::import::{self, ImportAnalysis, ImportRequest};
use micra_core::server::config::ServerConfig;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use super::CmdResult;
use crate::events::{self, Task};
use crate::state::Shared;

/// Selector de carpetas del sistema. `None` si la persona cancela.
#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> CmdResult<Option<String>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |folder| {
        let _ = tx.send(folder.and_then(|f| f.into_path().ok()));
    });
    Ok(rx.await.ok().flatten().map(|p| p.display().to_string()))
}

#[tauri::command]
pub async fn analyze_import(state: State<'_, Shared>, path: String) -> CmdResult<ImportAnalysis> {
    import::analyze(&state.paths, Path::new(&path)).await
}

#[tauri::command]
pub async fn import_server(
    app: AppHandle,
    state: State<'_, Shared>,
    request: ImportRequest,
) -> CmdResult<ServerConfig> {
    let progress = {
        let app = app.clone();
        Arc::new(move |done: u64, total: u64| events::task(&app, None, Task::Import, done, total))
    };
    import::import(
        &state.paths,
        &state.http,
        &state.java,
        request,
        Some(progress),
    )
    .await
}
