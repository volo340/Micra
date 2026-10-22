//! Cliente mínimo de la API de Adoptium (Eclipse Temurin).
//! Documentación: https://api.adoptium.net/q/swagger-ui/

use serde::Deserialize;

use crate::{Error, Result};

const API: &str = "https://api.adoptium.net/v3";

#[derive(Debug, Clone)]
pub struct RuntimeRelease {
    pub release_name: String,
    pub file_name: String,
    pub url: String,
    pub sha256: Option<String>,
    pub size: u64,
}

#[derive(Deserialize)]
struct Asset {
    binary: Binary,
    release_name: String,
}

#[derive(Deserialize)]
struct Binary {
    package: Package,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    link: String,
    checksum: Option<String>,
    size: u64,
}

/// Sistema y arquitectura con los nombres que usa Adoptium.
pub fn platform() -> Result<(&'static str, &'static str)> {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "windows" => "windows",
        "macos" => "mac",
        other => return Err(Error::UnsupportedPlatform(other.into())),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "aarch64",
        "x86" => "x32",
        other => return Err(Error::UnsupportedPlatform(other.into())),
    };
    Ok((os, arch))
}

/// Último build GA de Temurin para la versión mayor pedida.
/// Prefiere JRE (más ligero); si no existe para esta plataforma, usa JDK.
pub async fn latest_runtime(client: &reqwest::Client, major: u32) -> Result<RuntimeRelease> {
    let (os, arch) = platform()?;
    for image_type in ["jre", "jdk"] {
        let url = format!(
            "{API}/assets/latest/{major}/hotspot?architecture={arch}&image_type={image_type}&os={os}&vendor=eclipse"
        );
        let assets: Vec<Asset> = client.get(&url).send().await?.error_for_status()?.json().await?;
        if let Some(asset) = assets.into_iter().next() {
            let pkg = asset.binary.package;
            return Ok(RuntimeRelease {
                release_name: asset.release_name,
                file_name: pkg.name,
                url: pkg.link,
                sha256: pkg.checksum,
                size: pkg.size,
            });
        }
    }
    Err(Error::NotFound(format!("Temurin {major} para {os}/{arch}")))
}
