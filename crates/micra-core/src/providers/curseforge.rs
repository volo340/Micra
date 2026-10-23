//! Cliente de la API oficial de CurseForge (https://docs.curseforge.com/rest-api/).
//!
//! Requiere una API key propia (se solicita gratis en https://console.curseforge.com/).
//! Además, algunos autores desactivan la distribución por terceros: en ese caso
//! `download_url` llega vacío y hay que mandar al usuario a la web.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::download::{download_file, ExpectedHash, HashAlgo, Progress};
use crate::{Error, Result};

const API: &str = "https://api.curseforge.com/v1";
const MINECRAFT_GAME_ID: u32 = 432;

/// `modLoaderType` de CurseForge, nombre en Modrinth y etiqueta de CurseForge.
const LOADERS: &[(u8, &str, &str)] = &[
    (1, "forge", "Forge"),
    (4, "fabric", "Fabric"),
    (5, "quilt", "Quilt"),
    (6, "neoforge", "NeoForge"),
];

/// IDs de clase de CurseForge para Minecraft.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CfClass {
    Modpack,
    Mod,
    Plugin,
}

impl CfClass {
    fn id(self) -> u32 {
        match self {
            Self::Modpack => 4471,
            Self::Mod => 6,
            Self::Plugin => 5,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    data: T,
    pagination: Option<Pagination>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pagination {
    pub index: u32,
    pub page_size: u32,
    pub result_count: u32,
    pub total_count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CfSearchResponse {
    pub data: Vec<CfMod>,
    pub pagination: Option<Pagination>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfMod {
    pub id: u64,
    pub name: String,
    pub slug: String,
    pub summary: String,
    pub download_count: f64,
    pub logo: Option<CfLogo>,
    #[serde(default)]
    pub latest_files: Vec<CfFile>,
    #[serde(default)]
    pub latest_files_indexes: Vec<CfFileIndex>,
    pub allow_mod_distribution: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfFileIndex {
    #[serde(default)]
    pub mod_loader: Option<u8>,
}

impl CfMod {
    /// Loaders de sus archivos recientes, con los nombres de Modrinth.
    pub fn loaders(&self) -> Vec<&'static str> {
        let by_id = self
            .latest_files_indexes
            .iter()
            .filter_map(|i| LOADERS.iter().find(|(id, ..)| Some(*id) == i.mod_loader));
        // `game_versions` mezcla versiones del juego con loaders ("Forge").
        let by_label = self
            .latest_files
            .iter()
            .flat_map(|f| &f.game_versions)
            .filter_map(|v| {
                LOADERS
                    .iter()
                    .find(|(.., label)| label.eq_ignore_ascii_case(v))
            });
        let mut out: Vec<&'static str> = by_id.chain(by_label).map(|(_, name, _)| *name).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Si es de alguno de estos loaders. Los pocos que no dicen el suyo se
    /// quedan: su paquete de servidor trae el loader que necesitan.
    pub fn matches_loaders(&self, wanted: &[&str]) -> bool {
        let own = self.loaders();
        own.is_empty() || own.iter().any(|l| wanted.contains(l))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfLogo {
    pub thumbnail_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfFile {
    pub id: u64,
    pub mod_id: u64,
    pub display_name: String,
    pub file_name: String,
    pub download_url: Option<String>,
    #[serde(default)]
    pub game_versions: Vec<String>,
    pub server_pack_file_id: Option<u64>,
    pub is_server_pack: Option<bool>,
    #[serde(default)]
    pub hashes: Vec<CfHash>,
    pub file_length: u64,
}

impl CfFile {
    /// Versión de Minecraft del archivo. CurseForge mezcla en `game_versions`
    /// versiones del juego ("1.20.1") con loaders ("Forge", "Fabric").
    pub fn minecraft_version(&self) -> Option<&str> {
        self.game_versions
            .iter()
            .map(String::as_str)
            .find(|v| v.chars().next().is_some_and(|c| c.is_ascii_digit()))
    }

    fn sha1(&self) -> Option<&str> {
        // algo: 1 = SHA1, 2 = MD5
        self.hashes
            .iter()
            .find(|h| h.algo == 1)
            .map(|h| h.value.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfHash {
    pub value: String,
    pub algo: u8,
}

#[derive(Clone)]
pub struct CurseForgeClient {
    http: reqwest::Client,
    api_key: String,
}

impl CurseForgeClient {
    pub fn new(http: reqwest::Client, api_key: impl Into<String>) -> Self {
        Self {
            http,
            api_key: api_key.into(),
        }
    }

    /// Lee la clave de `MICRA_CURSEFORGE_API_KEY` (en ejecución o, si no,
    /// la incrustada al compilar). Sin clave, la integración queda desactivada.
    pub fn from_env(http: reqwest::Client) -> Option<Self> {
        std::env::var("MICRA_CURSEFORGE_API_KEY")
            .ok()
            .or_else(|| option_env!("MICRA_CURSEFORGE_API_KEY").map(str::to_owned))
            .filter(|k| !k.is_empty())
            .map(|k| Self::new(http, k))
    }

    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Envelope<T>> {
        let res = self
            .http
            .get(format!("{API}{path}"))
            .header("x-api-key", &self.api_key)
            .header("Accept", "application/json")
            .query(query)
            .send()
            .await?;
        if res.status() == reqwest::StatusCode::FORBIDDEN {
            return Err(Error::Api(
                "CurseForge rechazó la API key o el recurso no es distribuible".into(),
            ));
        }
        Ok(res.error_for_status()?.json().await?)
    }

    /// Con `loaders` (nombres de Modrinth: "forge", "neoforge", "fabric")
    /// solo salen proyectos de esos loaders.
    pub async fn search(
        &self,
        query: &str,
        class: CfClass,
        game_version: Option<&str>,
        loaders: &[&str],
        index: u32,
        page_size: u32,
    ) -> Result<CfSearchResponse> {
        let mut params = vec![
            ("gameId", MINECRAFT_GAME_ID.to_string()),
            ("classId", class.id().to_string()),
            ("searchFilter", query.to_string()),
            ("sortField", "2".into()), // 2 = popularidad
            ("sortOrder", "desc".into()),
            ("index", index.to_string()),
            ("pageSize", page_size.clamp(1, 50).to_string()),
        ];
        if let Some(gv) = game_version {
            params.push(("gameVersion", gv.to_string()));
        }
        let labels: Vec<&str> = LOADERS
            .iter()
            .filter(|(_, name, _)| loaders.contains(name))
            .map(|(.., label)| *label)
            .collect();
        let mut found = None;
        if !labels.is_empty() {
            let mut filtered = params.clone();
            filtered.push(("modLoaderTypes", serde_json::to_string(&labels)?));
            match self.get::<Vec<CfMod>>("/mods/search", &filtered).await {
                Ok(env) => found = Some(env),
                // Si la API no acepta el filtro, se filtra aquí.
                Err(Error::Http(e)) if e.status() == Some(reqwest::StatusCode::BAD_REQUEST) => {}
                Err(e) => return Err(e),
            }
        }
        let env: Envelope<Vec<CfMod>> = match found {
            Some(env) => env,
            None => self.get("/mods/search", &params).await?,
        };
        let mut data = env.data;
        if !loaders.is_empty() {
            data.retain(|m| m.matches_loaders(loaders));
        }
        Ok(CfSearchResponse {
            data,
            pagination: env.pagination,
        })
    }

    pub async fn files(&self, mod_id: u64, game_version: Option<&str>) -> Result<Vec<CfFile>> {
        let mut params = vec![("pageSize", "50".to_string())];
        if let Some(gv) = game_version {
            params.push(("gameVersion", gv.to_string()));
        }
        Ok(self
            .get(&format!("/mods/{mod_id}/files"), &params)
            .await?
            .data)
    }

    pub async fn file(&self, mod_id: u64, file_id: u64) -> Result<CfFile> {
        Ok(self
            .get(&format!("/mods/{mod_id}/files/{file_id}"), &[])
            .await?
            .data)
    }

    /// Para un archivo de modpack, devuelve su "server pack" (el ZIP listo
    /// para servidor que suben los autores), si existe.
    pub async fn server_pack_for(&self, file: &CfFile) -> Result<CfFile> {
        if file.is_server_pack == Some(true) {
            return Ok(file.clone());
        }
        match file.server_pack_file_id {
            Some(id) => self.file(file.mod_id, id).await,
            None => Err(Error::NotFound(format!(
                "server pack para «{}» (el autor no publicó uno)",
                file.display_name
            ))),
        }
    }

    pub async fn download(
        &self,
        file: &CfFile,
        dest: &Path,
        progress: Option<Progress<'_>>,
    ) -> Result<()> {
        let url = match &file.download_url {
            Some(u) => u.clone(),
            None => {
                let path = format!("/mods/{}/files/{}/download-url", file.mod_id, file.id);
                self.get::<String>(&path, &[]).await?.data
            }
        };
        let expected = file.sha1().map(|h| ExpectedHash::new(HashAlgo::Sha1, h));
        download_file(&self.http, &url, dest, expected.as_ref(), progress).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(indexes: &[u8], labels: &[&str]) -> CfMod {
        CfMod {
            id: 1,
            name: "Pack".into(),
            slug: "pack".into(),
            summary: String::new(),
            download_count: 0.0,
            logo: None,
            latest_files: vec![CfFile {
                id: 2,
                mod_id: 1,
                display_name: "Pack 1.0".into(),
                file_name: "pack.zip".into(),
                download_url: None,
                game_versions: labels.iter().map(|l| l.to_string()).collect(),
                server_pack_file_id: None,
                is_server_pack: None,
                hashes: vec![],
                file_length: 0,
            }],
            latest_files_indexes: indexes
                .iter()
                .map(|id| CfFileIndex {
                    mod_loader: Some(*id),
                })
                .collect(),
            allow_mod_distribution: Some(true),
        }
    }

    #[test]
    fn loaders_of_a_project() {
        let forge = pack(&[1], &["1.20.1", "Forge"]);
        assert_eq!(forge.loaders(), vec!["forge"]);
        assert!(forge.matches_loaders(&["forge", "neoforge"]));
        assert!(!forge.matches_loaders(&["fabric"]));

        let neo = pack(&[6], &["1.21.1"]);
        assert_eq!(neo.loaders(), vec!["neoforge"]);
        assert!(neo.matches_loaders(&["forge", "neoforge"]));

        let fabric = pack(&[], &["1.21.1", "Fabric", "Quilt"]);
        assert_eq!(fabric.loaders(), vec!["fabric", "quilt"]);
        assert!(fabric.matches_loaders(&["fabric"]));
        assert!(!fabric.matches_loaders(&["forge", "neoforge"]));

        // Sin datos de loader no se descarta.
        assert!(pack(&[], &["1.20.1"]).matches_loaders(&["fabric"]));
    }
}
