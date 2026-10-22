//! Manifiesto oficial de versiones de Mojang. De aquí sale el `server.jar`
//! vanilla y, sobre todo, la versión de Java exacta que pide cada versión.

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionManifest {
    pub latest: Latest,
    pub versions: Vec<VersionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Latest {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    pub sha1: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionDetails {
    pub id: String,
    pub java_version: Option<JavaVersion>,
    pub downloads: Downloads,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaVersion {
    pub major_version: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Downloads {
    pub server: Option<DownloadInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DownloadInfo {
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

impl VersionDetails {
    /// Versiones muy antiguas no declaran `javaVersion`: todas funcionan con Java 8.
    pub fn java_major(&self) -> u32 {
        self.java_version.as_ref().map_or(8, |j| j.major_version)
    }
}

pub async fn fetch_manifest(client: &reqwest::Client) -> Result<VersionManifest> {
    Ok(client
        .get(MANIFEST_URL)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

pub async fn fetch_version(client: &reqwest::Client, version_id: &str) -> Result<VersionDetails> {
    let manifest = fetch_manifest(client).await?;
    let entry = manifest
        .versions
        .iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| Error::NotFound(format!("Minecraft {version_id}")))?;
    Ok(client
        .get(&entry.url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}
