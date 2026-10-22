//! Eventos que el backend envía a la interfaz:
//!
//! - `provision://progress`: pasos al crear, importar o actualizar un servidor.
//! - `server://event`: consola, cambios de estado y jugadores.
//! - `server://stats`: CPU, RAM, jugadores y TPS cada segundo.
//! - `server://live`: red, TPS, actualizaciones y avisos de cada servidor.
//! - `task://progress`: progreso de copias, restauraciones, análisis e importaciones.
//! - `app://error`: un fallo en segundo plano que la persona debe ver.

use micra_core::server::monitor::ResourceSample;
use micra_core::server::process::ServerEvent;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::state::LiveInfo;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerEventPayload<'a> {
    server_id: &'a str,
    event: &'a ServerEvent,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerStatsPayload<'a> {
    server_id: &'a str,
    sample: &'a ResourceSample,
    player_count: usize,
    tps: Option<f32>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LivePayload<'a> {
    server_id: &'a str,
    live: &'a LiveInfo,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Task {
    Backup,
    Restore,
    Scan,
    Import,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskPayload<'a> {
    server_id: Option<&'a str>,
    task: Task,
    done: u64,
    total: u64,
}

#[derive(Clone, Serialize)]
struct ErrorPayload<'a> {
    title: &'a str,
    message: &'a str,
}

pub fn server_event(app: &AppHandle, server_id: &str, event: &ServerEvent) {
    let _ = app.emit("server://event", ServerEventPayload { server_id, event });
}

pub fn stats(
    app: &AppHandle,
    server_id: &str,
    sample: &ResourceSample,
    player_count: usize,
    tps: Option<f32>,
) {
    let _ = app.emit(
        "server://stats",
        ServerStatsPayload {
            server_id,
            sample,
            player_count,
            tps,
        },
    );
}

pub fn live(app: &AppHandle, server_id: &str, live: &LiveInfo) {
    let _ = app.emit("server://live", LivePayload { server_id, live });
}

pub fn task(app: &AppHandle, server_id: Option<&str>, task: Task, done: u64, total: u64) {
    let _ = app.emit(
        "task://progress",
        TaskPayload {
            server_id,
            task,
            done,
            total,
        },
    );
}

pub fn error(app: &AppHandle, title: &str, message: &str) {
    let _ = app.emit("app://error", ErrorPayload { title, message });
}
