//! Versiones de Minecraft y búsqueda de modpacks en Modrinth y CurseForge.

use micra_core::providers::curseforge::{CfClass, CfFile, CfSearchResponse};
use micra_core::providers::modrinth::{SearchQuery, SearchResponse, Version};
use micra_core::providers::mojang::{self, VersionEntry};
use micra_core::{tr, Error};
use tauri::State;

use super::CmdResult;
use crate::state::Shared;

/// Versiones "release" de Minecraft, de la más nueva a la más antigua.
#[tauri::command]
pub async fn minecraft_versions(state: State<'_, Shared>) -> CmdResult<Vec<VersionEntry>> {
    let manifest = mojang::fetch_manifest(&state.http).await?;
    Ok(manifest
        .versions
        .into_iter()
        .filter(|v| v.kind == "release")
        .collect())
}

#[tauri::command]
pub async fn search_modrinth(
    state: State<'_, Shared>,
    query: SearchQuery,
) -> CmdResult<SearchResponse> {
    state.modrinth.search(&query).await
}

#[tauri::command]
pub async fn modrinth_versions(
    state: State<'_, Shared>,
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
pub async fn curseforge_enabled(state: State<'_, Shared>) -> CmdResult<bool> {
    Ok(state.curseforge().await.is_some())
}

#[tauri::command]
pub async fn search_curseforge(
    state: State<'_, Shared>,
    query: String,
    class: CfClass,
    game_version: Option<String>,
    index: Option<u32>,
) -> CmdResult<CfSearchResponse> {
    let cf = state.curseforge().await.ok_or_else(curseforge_missing)?;
    cf.search(
        &query,
        class,
        game_version.as_deref(),
        index.unwrap_or(0),
        20,
    )
    .await
}

#[tauri::command]
pub async fn curseforge_files(
    state: State<'_, Shared>,
    mod_id: u64,
    game_version: Option<String>,
) -> CmdResult<Vec<CfFile>> {
    let cf = state.curseforge().await.ok_or_else(curseforge_missing)?;
    cf.files(mod_id, game_version.as_deref()).await
}

fn curseforge_missing() -> Error {
    Error::InvalidConfig(tr!(
        "CurseForge necesita una clave de API: añádela en Ajustes",
        "CurseForge needs an API key: add it in Settings"
    ))
}
