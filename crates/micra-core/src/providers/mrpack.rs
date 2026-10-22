//! Instalación de modpacks en formato `.mrpack` (Modrinth) en modo servidor.
//! Especificación: https://support.modrinth.com/en/articles/8802351-modrinth-modpack-format-mrpack

use std::collections::HashMap;
use std::path::Path;

use futures_util::{stream, TryStreamExt};
use serde::Deserialize;

use crate::archive::{extract_zip_prefix, safe_join};
use crate::download::{download_file, ExpectedHash, HashAlgo};
use crate::server::config::Loader;
use crate::{Error, Result};

/// Dominios desde los que la especificación permite descargar archivos.
const ALLOWED_HOSTS: &[&str] = &[
    "cdn.modrinth.com",
    "github.com",
    "raw.githubusercontent.com",
    "gitlab.com",
];

const PARALLEL_DOWNLOADS: usize = 8;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrpackIndex {
    pub format_version: u32,
    pub game: String,
    pub version_id: String,
    pub name: String,
    pub files: Vec<MrpackFile>,
    pub dependencies: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrpackFile {
    pub path: String,
    pub hashes: HashMap<String, String>,
    pub env: Option<MrpackEnv>,
    pub downloads: Vec<String>,
    pub file_size: u64,
}

#[derive(Debug, Deserialize)]
pub struct MrpackEnv {
    pub client: String,
    pub server: String,
}

/// Lo que el resto de la app necesita saber del pack instalado.
#[derive(Debug, Clone)]
pub struct PackInfo {
    pub name: String,
    pub version: String,
    pub mc_version: String,
    pub loader: Loader,
}

/// Progreso: (archivos completados, total de archivos).
pub type FileProgress<'a> = &'a (dyn Fn(usize, usize) + Send + Sync);

pub async fn install_mrpack(
    http: &reqwest::Client,
    mrpack: &Path,
    server_dir: &Path,
    progress: Option<FileProgress<'_>>,
) -> Result<PackInfo> {
    let index = read_index(mrpack).await?;
    if index.game != "minecraft" {
        return Err(Error::InvalidConfig(format!("juego no soportado: {}", index.game)));
    }

    // Archivos sueltos: solo los que el servidor necesita.
    let files: Vec<&MrpackFile> = index
        .files
        .iter()
        .filter(|f| f.env.as_ref().is_none_or(|e| e.server != "unsupported"))
        .collect();
    let total = files.len();
    let done = std::sync::atomic::AtomicUsize::new(0);

    stream::iter(files.into_iter().map(Ok::<_, Error>))
        .try_for_each_concurrent(PARALLEL_DOWNLOADS, |file| {
            let done = &done;
            async move {
                let dest = safe_join(server_dir, &file.path)?;
                let url = file
                    .downloads
                    .iter()
                    .find(|u| is_allowed_url(u))
                    .ok_or_else(|| Error::DisallowedUrl(file.downloads.join(", ")))?;
                let expected = match (file.hashes.get("sha512"), file.hashes.get("sha1")) {
                    (Some(h), _) => ExpectedHash::new(HashAlgo::Sha512, h),
                    (None, Some(h)) => ExpectedHash::new(HashAlgo::Sha1, h),
                    _ => return Err(Error::InvalidConfig(format!("{} no tiene hash", file.path))),
                };
                download_file(http, url, &dest, Some(&expected), None).await?;
                let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if let Some(cb) = progress {
                    cb(n, total);
                }
                Ok(())
            }
        })
        .await?;

    // `overrides/` aplica a cliente y servidor; `server-overrides/` va después
    // para que tenga prioridad.
    let mrpack_owned = mrpack.to_owned();
    let dir = server_dir.to_owned();
    tokio::task::spawn_blocking(move || -> Result<()> {
        extract_zip_prefix(&mrpack_owned, "overrides/", &dir)?;
        extract_zip_prefix(&mrpack_owned, "server-overrides/", &dir)?;
        Ok(())
    })
    .await
    .expect("la extracción no debería entrar en pánico")?;

    let mc_version = index
        .dependencies
        .get("minecraft")
        .cloned()
        .ok_or_else(|| Error::InvalidConfig("el pack no declara versión de Minecraft".into()))?;
    Ok(PackInfo {
        name: index.name,
        version: index.version_id,
        mc_version,
        loader: loader_from_dependencies(&index.dependencies),
    })
}

async fn read_index(mrpack: &Path) -> Result<MrpackIndex> {
    let mrpack = mrpack.to_owned();
    tokio::task::spawn_blocking(move || -> Result<MrpackIndex> {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(mrpack)?)?;
        let entry = zip.by_name("modrinth.index.json")?;
        Ok(serde_json::from_reader(entry)?)
    })
    .await
    .expect("la lectura no debería entrar en pánico")
}

fn loader_from_dependencies(deps: &HashMap<String, String>) -> Loader {
    if let Some(v) = deps.get("neoforge") {
        Loader::NeoForge { version: v.clone() }
    } else if let Some(v) = deps.get("forge") {
        Loader::Forge { version: v.clone() }
    } else if let Some(v) = deps.get("fabric-loader") {
        Loader::Fabric { loader_version: v.clone() }
    } else if let Some(v) = deps.get("quilt-loader") {
        Loader::Quilt { loader_version: v.clone() }
    } else {
        Loader::Vanilla
    }
}

fn is_allowed_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else { return false };
    parsed.scheme() == "https"
        && parsed.host_str().is_some_and(|h| ALLOWED_HOSTS.contains(&h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_allowlist() {
        assert!(is_allowed_url("https://cdn.modrinth.com/data/AANobbMI/versions/x/sodium.jar"));
        assert!(!is_allowed_url("http://cdn.modrinth.com/a.jar"));
        assert!(!is_allowed_url("https://evil.example.com/a.jar"));
        assert!(!is_allowed_url("https://cdn.modrinth.com.evil.example/a.jar"));
    }

    #[test]
    fn loader_detection() {
        let mut deps = HashMap::new();
        deps.insert("minecraft".into(), "1.20.1".into());
        deps.insert("fabric-loader".into(), "0.16.9".into());
        assert_eq!(
            loader_from_dependencies(&deps),
            Loader::Fabric { loader_version: "0.16.9".into() }
        );
    }
}
