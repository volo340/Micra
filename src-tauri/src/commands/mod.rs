//! Comandos que la interfaz llama con `invoke(...)`. Los eventos que el
//! backend emite están en `events.rs`.

pub mod app;
pub mod backups;
pub mod content;
pub mod import;
pub mod map;
pub mod mods;
pub mod network;
pub mod players;
pub mod properties;
pub mod servers;
pub mod stats;

use std::path::Path;

use micra_core::{tr, Error};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

pub type CmdResult<T> = Result<T, Error>;

fn open_path(app: &AppHandle, path: &Path) -> CmdResult<()> {
    app.opener()
        .open_path(path.display().to_string(), None::<&str>)
        .map_err(|e| {
            Error::Api(tr!(
                "no se pudo abrir la carpeta: {e}",
                "could not open the folder: {e}"
            ))
        })
}

fn open_url(app: &AppHandle, url: &str) -> CmdResult<()> {
    app.opener().open_url(url, None::<&str>).map_err(|e| {
        Error::Api(tr!(
            "no se pudo abrir el navegador: {e}",
            "could not open the browser: {e}"
        ))
    })
}
