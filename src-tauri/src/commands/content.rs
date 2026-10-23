//! Versiones de Minecraft (también las de cada loader) y búsqueda de mods y
//! modpacks en Modrinth y CurseForge.

use micra_core::providers::curseforge::{CfClass, CfFile, CfSearchResponse};
use micra_core::providers::loaders::{self, ModLoader};
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

/// Versiones de Minecraft con servidor de Fabric, Forge o NeoForge.
#[tauri::command]
pub async fn loader_game_versions(
    state: State<'_, Shared>,
    loader: ModLoader,
) -> CmdResult<Vec<String>> {
    loaders::game_versions(&state.http, loader).await
}

#[tauri::command]
pub async fn search_modrinth(
    state: State<'_, Shared>,
    query: SearchQuery,
) -> CmdResult<SearchResponse> {
    state.modrinth.search(&query).await
}

/// Versiones de un proyecto; con `loaders`, solo las de alguno de ellos.
#[tauri::command]
pub async fn modrinth_versions(
    state: State<'_, Shared>,
    project_id: String,
    game_version: Option<String>,
    loaders: Option<Vec<String>>,
) -> CmdResult<Vec<Version>> {
    let loaders: Vec<&str> = loaders.iter().flatten().map(String::as_str).collect();
    state
        .modrinth
        .project_versions_any(&project_id, game_version.as_deref(), &loaders)
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
    loaders: Option<Vec<String>>,
    index: Option<u32>,
) -> CmdResult<CfSearchResponse> {
    let cf = state.curseforge().await.ok_or_else(curseforge_missing)?;
    let loaders: Vec<&str> = loaders.iter().flatten().map(String::as_str).collect();
    cf.search(
        &query,
        class,
        game_version.as_deref(),
        &loaders,
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
