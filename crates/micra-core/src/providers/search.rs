//! Un solo buscador para Modrinth y CurseForge: pide la página a las dos
//! fuentes a la vez y mezcla los resultados. Un proyecto que está en las dos
//! sale una vez, el de Modrinth (sus archivos llegan con SHA-512 y siempre se
//! pueden descargar desde la app).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::providers::curseforge::{CfClass, CfMod, CurseForgeClient, Pagination};
use crate::providers::modrinth::{ModrinthClient, ProjectKind, SearchHit, SearchQuery};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Modrinth,
    Curseforge,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ContentKind {
    Mod,
    Modpack,
}

impl ContentKind {
    /// Resultados que se piden a cada fuente por página.
    fn page_size(self) -> u32 {
        match self {
            Self::Mod => 20,
            Self::Modpack => 12,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentQuery {
    pub query: String,
    pub kind: ContentKind,
    pub game_version: Option<String>,
    /// Loaders que valen (nombres de Modrinth): basta con que el proyecto
    /// tenga uno.
    #[serde(default)]
    pub loaders: Vec<String>,
    /// `None`: la primera página. Para seguir, el `next` de la anterior.
    #[serde(default)]
    pub cursor: Option<Cursor>,
}

/// Por dónde sigue cada fuente; `None` si ya no le quedan resultados.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Cursor {
    pub modrinth: Option<u32>,
    pub curseforge: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContentHit {
    pub source: Source,
    /// Id del proyecto en su fuente (en CurseForge, el número como texto).
    pub id: String,
    pub title: String,
    pub author: Option<String>,
    pub description: String,
    pub icon_url: Option<String>,
    pub downloads: u64,
    /// Versión de Minecraft más reciente que anuncia.
    pub game_version: Option<String>,
    /// El título normalizado: el mismo proyecto comparte este valor en las
    /// dos fuentes (la interfaz lo usa al juntar páginas).
    pub dedupe_key: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentPage {
    pub hits: Vec<ContentHit>,
    /// Para pedir la página siguiente; `None` si no hay más.
    pub next: Option<Cursor>,
    /// Fuentes que no respondieron (se muestran los resultados de las demás).
    pub failed: Vec<SourceFailure>,
    /// Si se pudo buscar en CurseForge (necesita una clave de API).
    pub curseforge_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFailure {
    pub source: Source,
    pub error: String,
}

/// Busca en Modrinth y, si hay clave, en CurseForge a la vez.
pub async fn search(
    modrinth: &ModrinthClient,
    curseforge: Option<&CurseForgeClient>,
    q: &ContentQuery,
) -> Result<ContentPage> {
    let cursor = q.cursor.unwrap_or(Cursor {
        modrinth: Some(0),
        curseforge: Some(0),
    });
    let curseforge_offset = cursor.curseforge.filter(|_| curseforge.is_some());
    let (from_modrinth, from_curseforge) = tokio::join!(
        async {
            match cursor.modrinth {
                Some(offset) => Some(search_modrinth(modrinth, q, offset).await),
                None => None,
            }
        },
        async {
            match (curseforge, curseforge_offset) {
                (Some(cf), Some(offset)) => Some(search_curseforge(cf, q, offset).await),
                _ => None,
            }
        }
    );

    let mut next = Cursor::default();
    let mut failed = Vec::new();
    let mut first_error = None;
    let mut take =
        |source: Source, result: Option<Result<(Vec<ContentHit>, Option<u32>)>>| match result {
            Some(Ok((hits, more))) => {
                match source {
                    Source::Modrinth => next.modrinth = more,
                    Source::Curseforge => next.curseforge = more,
                }
                Some(hits)
            }
            Some(Err(e)) => {
                failed.push(SourceFailure {
                    source,
                    error: e.to_string(),
                });
                first_error.get_or_insert(e);
                None
            }
            None => None,
        };
    let modrinth_hits = take(Source::Modrinth, from_modrinth);
    let curseforge_hits = take(Source::Curseforge, from_curseforge);
    if modrinth_hits.is_none() && curseforge_hits.is_none() {
        if let Some(e) = first_error {
            return Err(e);
        }
    }
    let hits = merge(
        modrinth_hits.unwrap_or_default(),
        curseforge_hits.unwrap_or_default(),
    );
    let has_more = next.modrinth.is_some() || next.curseforge.is_some();
    Ok(ContentPage {
        hits,
        next: has_more.then_some(next),
        failed,
        curseforge_enabled: curseforge.is_some(),
    })
}

async fn search_modrinth(
    modrinth: &ModrinthClient,
    q: &ContentQuery,
    offset: u32,
) -> Result<(Vec<ContentHit>, Option<u32>)> {
    let mut loaders = q.loaders.iter().cloned();
    let query = SearchQuery {
        query: q.query.clone(),
        kind: match q.kind {
            ContentKind::Mod => ProjectKind::Mod,
            ContentKind::Modpack => ProjectKind::Modpack,
        },
        game_version: q.game_version.clone(),
        loader: loaders.next(),
        extra_loaders: loaders.collect(),
        offset,
        limit: q.kind.page_size(),
    };
    let res = modrinth.search(&query).await?;
    let seen = offset + res.hits.len() as u32;
    let more = (!res.hits.is_empty() && seen < res.total_hits).then_some(seen);
    Ok((res.hits.into_iter().map(from_modrinth).collect(), more))
}

async fn search_curseforge(
    cf: &CurseForgeClient,
    q: &ContentQuery,
    index: u32,
) -> Result<(Vec<ContentHit>, Option<u32>)> {
    let class = match q.kind {
        ContentKind::Mod => CfClass::Mod,
        ContentKind::Modpack => CfClass::Modpack,
    };
    let loaders: Vec<&str> = q.loaders.iter().map(String::as_str).collect();
    let res = cf
        .search(
            &q.query,
            class,
            q.game_version.as_deref(),
            &loaders,
            index,
            q.kind.page_size(),
        )
        .await?;
    let more = res.pagination.as_ref().and_then(next_index);
    // Como en Modrinth, sin los mods que solo funcionan en el juego de cada
    // jugador (CurseForge lo indica en sus versiones: "Client").
    let hits = res
        .data
        .into_iter()
        .filter(|m| q.kind == ContentKind::Modpack || !m.is_client_only())
        .map(from_curseforge)
        .collect();
    Ok((hits, more))
}

/// CurseForge no deja pasar de los 10 000 primeros resultados.
const CURSEFORGE_MAX_RESULTS: u32 = 10_000;

fn next_index(p: &Pagination) -> Option<u32> {
    let seen = p.index + p.result_count;
    let limit = p.total_count.min(CURSEFORGE_MAX_RESULTS);
    (p.result_count > 0 && seen < limit).then_some(seen)
}

fn from_modrinth(hit: SearchHit) -> ContentHit {
    ContentHit {
        source: Source::Modrinth,
        dedupe_key: dedupe_key(&hit.title),
        game_version: hit.versions.last().cloned(),
        id: hit.project_id,
        title: hit.title,
        author: Some(hit.author).filter(|a| !a.is_empty()),
        description: hit.description,
        icon_url: hit.icon_url,
        downloads: hit.downloads,
    }
}

fn from_curseforge(m: CfMod) -> ContentHit {
    let newest = m.latest_files.iter().max_by_key(|f| f.id);
    ContentHit {
        source: Source::Curseforge,
        dedupe_key: dedupe_key(&m.name),
        game_version: newest
            .and_then(|f| f.minecraft_version())
            .map(str::to_string),
        id: m.id.to_string(),
        author: m.authors.first().map(|a| a.name.clone()),
        description: m.summary,
        icon_url: m.logo.map(|l| l.thumbnail_url),
        downloads: m.download_count.max(0.0) as u64,
        title: m.name,
    }
}

/// Intercala las dos listas (una de cada) y quita de CurseForge lo que ya
/// sale de Modrinth.
fn merge(modrinth: Vec<ContentHit>, curseforge: Vec<ContentHit>) -> Vec<ContentHit> {
    let on_modrinth: HashSet<String> = modrinth.iter().map(|h| h.dedupe_key.clone()).collect();
    let mut a = modrinth.into_iter();
    let mut b = curseforge
        .into_iter()
        .filter(|h| !on_modrinth.contains(&h.dedupe_key));
    let mut out = Vec::new();
    loop {
        match (a.next(), b.next()) {
            (None, None) => break,
            (x, y) => out.extend(x.into_iter().chain(y)),
        }
    }
    out
}

/// Título en minúsculas, sin lo que va entre paréntesis o corchetes y solo
/// con letras y números: «Just Enough Items (JEI)» → «justenoughitems».
pub fn dedupe_key(title: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for c in title.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            c if depth == 0 && c.is_alphanumeric() => out.extend(c.to_lowercase()),
            _ => {}
        }
    }
    if out.is_empty() {
        title.trim().to_lowercase()
    } else {
        out
    }
}

/// Error para quien busca en CurseForge sin clave.
pub fn curseforge_missing() -> Error {
    Error::InvalidConfig(crate::tr!(
        "CurseForge necesita una clave de API: añádela en Ajustes",
        "CurseForge needs an API key: add it in Settings"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(source: Source, id: &str, title: &str) -> ContentHit {
        ContentHit {
            source,
            id: id.into(),
            title: title.into(),
            author: None,
            description: String::new(),
            icon_url: None,
            downloads: 0,
            game_version: None,
            dedupe_key: dedupe_key(title),
        }
    }

    #[test]
    fn keys_of_titles() {
        assert_eq!(dedupe_key("Just Enough Items (JEI)"), "justenoughitems");
        assert_eq!(dedupe_key("Just Enough Items"), "justenoughitems");
        assert_eq!(dedupe_key("Xaero's Minimap"), "xaerosminimap");
        assert_eq!(dedupe_key("[Fabric] Sodium"), "sodium");
        assert_eq!(dedupe_key("Create: Steam 'n' Rails"), "createsteamnrails");
        assert_ne!(dedupe_key("Sodium Extra"), dedupe_key("Sodium"));
        assert_eq!(dedupe_key("(((("), "((((");
    }

    #[test]
    fn merges_both_sources_once() {
        let modrinth = vec![
            hit(Source::Modrinth, "AANobbMI", "Sodium"),
            hit(Source::Modrinth, "u6dRKJwZ", "Just Enough Items"),
            hit(Source::Modrinth, "P7dR8mSH", "Fabric API"),
        ];
        let curseforge = vec![
            hit(Source::Curseforge, "238222", "Just Enough Items (JEI)"),
            hit(Source::Curseforge, "1", "Waystones"),
            hit(Source::Curseforge, "2", "Jade"),
            hit(Source::Curseforge, "3", "Mouse Tweaks"),
        ];
        let ids: Vec<String> = merge(modrinth, curseforge)
            .into_iter()
            .map(|h| h.id)
            .collect();
        assert_eq!(
            ids,
            ["AANobbMI", "1", "u6dRKJwZ", "2", "P7dR8mSH", "3"].map(String::from)
        );
    }

    #[test]
    fn curseforge_pages() {
        let page = |index, result_count, total_count| Pagination {
            index,
            page_size: 20,
            result_count,
            total_count,
        };
        assert_eq!(next_index(&page(0, 20, 45)), Some(20));
        assert_eq!(next_index(&page(40, 5, 45)), None);
        assert_eq!(next_index(&page(0, 0, 0)), None);
        assert_eq!(next_index(&page(9980, 20, 50_000)), None);
    }

    #[test]
    fn query_shape() {
        let q: ContentQuery = serde_json::from_value(serde_json::json!({
            "query": "create",
            "kind": "mod",
            "gameVersion": "1.20.1",
            "loaders": ["forge"],
        }))
        .unwrap();
        assert!(q.cursor.is_none());
        let q: ContentQuery = serde_json::from_value(serde_json::json!({
            "query": "",
            "kind": "modpack",
            "gameVersion": null,
            "cursor": { "modrinth": null, "curseforge": 24 },
        }))
        .unwrap();
        assert_eq!(
            q.cursor,
            Some(Cursor {
                modrinth: None,
                curseforge: Some(24)
            })
        );
        assert!(q.loaders.is_empty());
    }
}
