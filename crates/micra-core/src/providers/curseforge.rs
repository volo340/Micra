//! Cliente de la API oficial de CurseForge (https://docs.curseforge.com/rest-api/).
//!
//! Requiere una API key propia: se consigue gratis en https://console.curseforge.com/,
//! en la página «API keys» (no confundir con la «Profile API key» del perfil).
//! Desde el 16 de julio de 2026 el CDN de CurseForge también pide la clave para
//! descargar archivos. Además, algunos autores desactivan la distribución por
//! terceros: en ese caso `download_url` llega vacío y hay que mandar al usuario
//! a la web.

use std::path::Path;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::download::{download_request, ExpectedHash, HashAlgo, Progress};
use crate::{tr, Error, Result};

const API: &str = "https://api.curseforge.com/v1";
const MINECRAFT_GAME_ID: u32 = 432;
/// Así empiezan las «Profile API key» del perfil de la consola, que se
/// confunden con la clave de «API keys».
const PROFILE_KEY_PREFIX: &str = "cfc_pat_";

/// `modLoaderType` de CurseForge, nombre en Modrinth y etiqueta de CurseForge.
const LOADERS: &[(u8, &str, &str)] = &[
    (1, "forge", "Forge"),
    (4, "fabric", "Fabric"),
    (5, "quilt", "Quilt"),
    (6, "neoforge", "NeoForge"),
];

/// Nombre de Modrinth de una etiqueta de loader de CurseForge ("NeoForge").
fn loader_from_label(label: &str) -> Option<&'static str> {
    LOADERS
        .iter()
        .find(|(.., l)| l.eq_ignore_ascii_case(label))
        .map(|(_, name, _)| *name)
}

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
            .filter_map(|i| LOADERS.iter().find(|(id, ..)| Some(*id) == i.mod_loader))
            .map(|(_, name, _)| *name);
        let by_label = self.latest_files.iter().flat_map(CfFile::loaders);
        let mut out: Vec<&'static str> = by_id.chain(by_label).collect();
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

    /// Loaders del archivo (nombres de Modrinth), sacados de `game_versions`.
    pub fn loaders(&self) -> Vec<&'static str> {
        self.game_versions
            .iter()
            .filter_map(|v| loader_from_label(v))
            .collect()
    }

    /// Si es de alguno de estos loaders. Los que no dicen el suyo se quedan.
    pub fn matches_loaders(&self, wanted: &[&str]) -> bool {
        let own = self.loaders();
        own.is_empty() || own.iter().any(|l| wanted.contains(l))
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

    async fn send(&self, path: &str, query: &[(&str, String)]) -> Result<reqwest::Response> {
        Ok(self
            .http
            .get(format!("{API}{path}"))
            .header("x-api-key", &self.api_key)
            .header("Accept", "application/json")
            .query(query)
            .send()
            .await?)
    }

    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Envelope<T>> {
        let res = self.send(path, query).await?;
        if rejects_key(res.status()) {
            return Err(self.key_rejected());
        }
        Ok(res.error_for_status()?.json().await?)
    }

    /// Pregunta a CurseForge por Minecraft para probar la clave: `Ok(false)`
    /// si la rechaza y error si no se pudo preguntar (sin internet...).
    pub async fn check_key(&self) -> Result<bool> {
        let res = self
            .send(&format!("/games/{MINECRAFT_GAME_ID}"), &[])
            .await?;
        if rejects_key(res.status()) {
            return Ok(false);
        }
        res.error_for_status()?;
        Ok(true)
    }

    /// Error para una clave que CurseForge no acepta, con una pista si es la
    /// «Profile API key» de la consola, que se confunde a menudo con la buena.
    pub fn key_rejected(&self) -> Error {
        Error::Api(if self.api_key.starts_with(PROFILE_KEY_PREFIX) {
            tr!(
                "CurseForge no acepta esa clave: es la «Profile API key» (empieza por {PROFILE_KEY_PREFIX}). En console.curseforge.com abre «API keys», copia la clave que aparece allí y guárdala en Ajustes",
                "CurseForge does not accept that key: it is the \"Profile API key\" (it starts with {PROFILE_KEY_PREFIX}). On console.curseforge.com, open \"API keys\", copy the key shown there and save it in Settings"
            )
        } else {
            tr!(
                "CurseForge no acepta la clave de API. En console.curseforge.com abre «API keys», copia la clave entera y guárdala en Ajustes",
                "CurseForge does not accept the API key. On console.curseforge.com, open \"API keys\", copy the whole key and save it in Settings"
            )
        })
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
        let types: Vec<String> = LOADERS
            .iter()
            .filter(|(_, name, _)| loaders.contains(name))
            .map(|(id, ..)| id.to_string())
            .collect();
        let mut found = None;
        if !types.is_empty() {
            let mut filtered = params.clone();
            // Con los números (`[1,6]`) no hay dudas sobre cómo escribir los nombres.
            filtered.push(("modLoaderTypes", format!("[{}]", types.join(","))));
            let res = self.send("/mods/search", &filtered).await?;
            // Si CurseForge no acepta el filtro, se busca sin él y se filtra aquí.
            if res.status().is_success() {
                found = res.json::<Envelope<Vec<CfMod>>>().await.ok();
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
            None => self.download_url(file).await?,
        };
        let expected = file.sha1().map(|h| ExpectedHash::new(HashAlgo::Sha1, h));
        let mut request = self.http.get(&url);
        // Desde el 16 de julio de 2026 el CDN de CurseForge también pide la clave.
        if is_curseforge_host(&url) {
            request = request.header("x-api-key", &self.api_key);
        }
        download_request(request, dest, expected.as_ref(), progress).await
    }

    /// Enlace de un archivo que no lo trae. CurseForge responde 403 cuando el
    /// autor no deja descargarlo desde otras apps.
    async fn download_url(&self, file: &CfFile) -> Result<String> {
        let path = format!("/mods/{}/files/{}/download-url", file.mod_id, file.id);
        let res = self.send(&path, &[]).await?;
        match res.status() {
            StatusCode::FORBIDDEN => {
                let name = &file.display_name;
                Err(Error::Api(tr!(
                    "el autor de «{name}» no deja descargarlo desde otras apps: descárgalo desde la web de CurseForge",
                    "the author of \"{name}\" does not allow downloading it from other apps: get it from the CurseForge website"
                )))
            }
            StatusCode::UNAUTHORIZED => Err(self.key_rejected()),
            _ => Ok(res
                .error_for_status()?
                .json::<Envelope<String>>()
                .await?
                .data),
        }
    }
}

/// CurseForge responde 401 o 403 cuando no acepta la clave.
fn rejects_key(status: StatusCode) -> bool {
    status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN
}

/// La clave solo viaja a CurseForge y a su CDN (forgecdn.net), siempre por HTTPS.
fn is_curseforge_host(url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(url) else {
        return false;
    };
    let host = url.host_str().unwrap_or_default();
    url.scheme() == "https"
        && ["forgecdn.net", "curseforge.com"].iter().any(|domain| {
            host == *domain
                || host
                    .strip_suffix(domain)
                    .is_some_and(|sub| sub.ends_with('.'))
        })
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

    #[test]
    fn loaders_of_a_file() {
        let file = &pack(&[], &["1.21.1", "NeoForge"]).latest_files[0];
        assert_eq!(file.loaders(), vec!["neoforge"]);
        assert!(file.matches_loaders(&["forge", "neoforge"]));
        assert!(!file.matches_loaders(&["fabric"]));
        assert!(pack(&[], &["1.21.1"]).latest_files[0].matches_loaders(&["fabric"]));
    }

    #[test]
    fn key_only_goes_to_curseforge() {
        assert!(is_curseforge_host(
            "https://edge.forgecdn.net/files/1/2/pack.zip"
        ));
        assert!(is_curseforge_host(
            "https://mediafilez.forgecdn.net/files/1/2/pack.zip"
        ));
        assert!(is_curseforge_host("https://api.curseforge.com/v1/mods/1"));
        assert!(!is_curseforge_host(
            "http://edge.forgecdn.net/files/1/2/pack.zip"
        ));
        assert!(!is_curseforge_host("https://evilforgecdn.net/pack.zip"));
        assert!(!is_curseforge_host(
            "https://forgecdn.net.example.com/pack.zip"
        ));
        assert!(!is_curseforge_host("https://github.com/pack.zip"));
        assert!(!is_curseforge_host("no es una url"));
    }

    #[test]
    fn rejected_key_hint() {
        let http = reqwest::Client::new();
        let profile = CurseForgeClient::new(http.clone(), "cfc_pat_0123456789abcdefghij");
        assert!(profile
            .key_rejected()
            .to_string()
            .contains("Profile API key"));
        let other = CurseForgeClient::new(http, "$2a$10$0123456789abcdefghijklmn");
        assert!(!other.key_rejected().to_string().contains("Profile API key"));
        assert!(other.key_rejected().to_string().contains("API keys"));
    }
}
