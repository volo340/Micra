//! Estadísticas guardadas (un punto por minuto durante 31 días).

use chrono::{Duration, Utc};
use micra_core::server::stats::{self, StatPoint};
use serde::Deserialize;
use tauri::State;

use super::CmdResult;
use crate::state::Shared;
use crate::supervisor::server_dir;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StatsRange {
    Hour,
    Day,
    Week,
    Month,
}

/// Puntos que caben bien en una gráfica.
const MAX_POINTS: usize = 240;

#[tauri::command]
pub async fn server_stats(
    state: State<'_, Shared>,
    id: String,
    range: StatsRange,
) -> CmdResult<Vec<StatPoint>> {
    server_dir(&state, &id)?;
    let span = match range {
        StatsRange::Hour => Duration::hours(1),
        StatsRange::Day => Duration::days(1),
        StatsRange::Week => Duration::days(7),
        StatsRange::Month => Duration::days(30),
    };
    let dir = state.paths.stats_dir(&id);
    let to = Utc::now();
    let points = tauri::async_runtime::spawn_blocking(move || {
        stats::read_range(&dir, to - span, to, MAX_POINTS)
    })
    .await
    .unwrap_or_default();
    Ok(points)
}
