//! Cliente de la API v2 de Modrinth (https://docs.modrinth.com/api/).
//! No requiere clave, pero sí un User-Agent identificable (ver `http.rs`).

use serde::{Deserialize, Serialize};

use crate::Result;

const API: &str = "https://api.modrinth.com/v2";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProjectKind {
    Modpack,
    Mod,
    Plugin,
}

impl ProjectKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Modpack => "modpack",
            Self::Mod => "mod",
            Self::Plugin => "plugin",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub query: String,
    pub kind: ProjectKind,
    pub game_version: Option<String>,
    /// fabric, forge, neoforge, quilt, paper...
    pub loader: Option<String>,
    #[serde(default)]
    pub offset: u32,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_limit() -> u32 {
    20
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    pub offset: u32,
    pub limit: u32,
    pub total_hits: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub project_type: String,
    pub author: String,
    pub downloads: u64,
    pub icon_url: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub versions: Vec<String>,
    pub server_side: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Version {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    pub version_type: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub date_published: String,
    pub files: Vec<VersionFile>,
}

impl Version {
    pub fn primary_file(&self) -> Option<&VersionFile> {
        self.files.iter().find(|f| f.primary).or(self.files.first())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionFile {
    pub url: String,
    pub filename: String,
    pub primary: bool,
    pub size: u64,
    pub hashes: FileHashes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileHashes {
    pub sha1: Option<String>,
    pub sha512: Option<String>,
}

#[derive(Clone)]
pub struct ModrinthClient {
    http: reqwest::Client,
}

impl ModrinthClient {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }

    /// Busca proyectos que se puedan usar en un servidor
    /// (`server_side` requerido u opcional).
    pub async fn search(&self, q: &SearchQuery) -> Result<SearchResponse> {
        let facets = build_facets(q);
        let limit = q.limit.clamp(1, 100).to_string();
        let offset = q.offset.to_string();
        let res = self
            .http
            .get(format!("{API}/search"))
            .query(&[
                ("query", q.query.as_str()),
                ("facets", facets.as_str()),
                ("index", "relevance"),
                ("limit", limit.as_str()),
                ("offset", offset.as_str()),
            ])
            .send()
            .await?
            .error_for_status()?;
        Ok(res.json().await?)
    }

    /// Versiones de un proyecto, opcionalmente filtradas por versión del
    /// juego y loader. Modrinth las devuelve de la más nueva a la más antigua.
    pub async fn project_versions(
        &self,
        id_or_slug: &str,
        game_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<Version>> {
        let mut req = self.http.get(format!("{API}/project/{id_or_slug}/version"));
        if let Some(gv) = game_version {
            req = req.query(&[("game_versions", serde_json::to_string(&[gv])?)]);
        }
        if let Some(l) = loader {
            req = req.query(&[("loaders", serde_json::to_string(&[l])?)]);
        }
        Ok(req.send().await?.error_for_status()?.json().await?)
    }

    pub async fn version(&self, version_id: &str) -> Result<Version> {
        Ok(self
            .http
            .get(format!("{API}/version/{version_id}"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
}

/// Facets de Modrinth: lista de listas; OR dentro de cada lista, AND entre listas.
fn build_facets(q: &SearchQuery) -> String {
    let mut facets: Vec<Vec<String>> = vec![
        vec![format!("project_type:{}", q.kind.as_str())],
        vec!["server_side:required".into(), "server_side:optional".into()],
    ];
    if let Some(gv) = &q.game_version {
        facets.push(vec![format!("versions:{gv}")]);
    }
    if let Some(loader) = &q.loader {
        facets.push(vec![format!("categories:{loader}")]);
    }
    serde_json::to_string(&facets).expect("los facets siempre son serializables")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facets_shape() {
        let q = SearchQuery {
            query: "create".into(),
            kind: ProjectKind::Modpack,
            game_version: Some("1.20.1".into()),
            loader: Some("forge".into()),
            offset: 0,
            limit: 20,
        };
        assert_eq!(
            build_facets(&q),
            r#"[["project_type:modpack"],["server_side:required","server_side:optional"],["versions:1.20.1"],["categories:forge"]]"#
        );
    }
}
