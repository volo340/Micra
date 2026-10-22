//! Instalación de modpacks en formato `.mrpack` (Modrinth) en modo servidor.
//! Especificación: https://support.modrinth.com/en/articles/8802351-modrinth-modpack-format-mrpack

use std::collections::HashMap;
use std::path::Path;

use futures_util::{stream, TryStreamExt};
use serde::Deserialize;

use crate::archive::{extract_zip_prefix_filtered, list_zip_prefix, safe_join};
use crate::download::{download_file, ExpectedHash, HashAlgo};
use crate::server::config::Loader;
use crate::{tr, Error, Result};

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
    /// Rutas relativas de todo lo que instaló el pack (para poder quitarlo
    /// al actualizar).
    pub files: Vec<String>,
}

/// Progreso: (archivos completados, total de archivos).
pub type FileProgress<'a> = &'a (dyn Fn(usize, usize) + Send + Sync);

/// `protected`: rutas (o carpetas) de la carpeta del servidor que el pack no
/// puede sobrescribir, como el mundo o `server.properties` al actualizar.
pub async fn install_mrpack(
    http: &reqwest::Client,
    mrpack: &Path,
    server_dir: &Path,
    progress: Option<FileProgress<'_>>,
    protected: &[String],
) -> Result<PackInfo> {
    let index = read_index(mrpack).await?;
    if index.game != "minecraft" {
        let game = &index.game;
        return Err(Error::InvalidConfig(tr!(
            "juego no soportado: {game}",
            "unsupported game: {game}"
        )));
    }

    // Archivos sueltos: solo los que el servidor necesita.
    let files: Vec<&MrpackFile> = server_files(&index)
        .filter(|f| !is_protected(&f.path, protected))
        .collect();
    let mut installed: Vec<String> = files.iter().map(|f| f.path.replace('\\', "/")).collect();
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
                    _ => {
                        let path = &file.path;
                        return Err(Error::InvalidConfig(tr!(
                            "{path} no tiene hash: no se puede verificar",
                            "{path} has no hash: it cannot be verified"
                        )));
                    }
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
    let protected_owned = protected.to_vec();
    let overrides = tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
        let skip = |rel: &str| is_protected(rel, &protected_owned);
        let mut written = extract_zip_prefix_filtered(&mrpack_owned, "overrides/", &dir, &skip)?;
        written.extend(extract_zip_prefix_filtered(
            &mrpack_owned,
            "server-overrides/",
            &dir,
            &skip,
        )?);
        Ok(written)
    })
    .await
    .expect("la extracción no debería entrar en pánico")?;
    installed.extend(overrides);
    installed.sort();
    installed.dedup();

    let mc_version = mc_version_of(&index)?;
    Ok(PackInfo {
        name: index.name,
        version: index.version_id,
        mc_version,
        loader: loader_from_dependencies(&index.dependencies),
        files: installed,
    })
}

/// Versión de Minecraft, loader y lista de archivos de un `.mrpack` sin
/// instalarlo (para comparar versiones al actualizar).
pub async fn inspect(mrpack: &Path) -> Result<PackInfo> {
    let index = read_index(mrpack).await?;
    let mut files: Vec<String> = server_files(&index)
        .map(|f| f.path.replace('\\', "/"))
        .collect();
    let path = mrpack.to_owned();
    let overrides = tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
        let mut list = list_zip_prefix(&path, "overrides/")?;
        list.extend(list_zip_prefix(&path, "server-overrides/")?);
        Ok(list)
    })
    .await
    .expect("la lectura no debería entrar en pánico")?;
    files.extend(overrides);
    files.sort();
    files.dedup();
    Ok(PackInfo {
        mc_version: mc_version_of(&index)?,
        loader: loader_from_dependencies(&index.dependencies),
        name: index.name,
        version: index.version_id,
        files,
    })
}

fn server_files(index: &MrpackIndex) -> impl Iterator<Item = &MrpackFile> {
    index
        .files
        .iter()
        .filter(|f| f.env.as_ref().is_none_or(|e| e.server != "unsupported"))
}

fn mc_version_of(index: &MrpackIndex) -> Result<String> {
    index.dependencies.get("minecraft").cloned().ok_or_else(|| {
        Error::InvalidConfig(tr!(
            "el pack no declara versión de Minecraft",
            "the pack does not declare a Minecraft version"
        ))
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
        Loader::Fabric {
            loader_version: v.clone(),
        }
    } else if let Some(v) = deps.get("quilt-loader") {
        Loader::Quilt {
            loader_version: v.clone(),
        }
    } else {
        Loader::Vanilla
    }
}

/// `true` si `rel` es una de las rutas protegidas o está dentro de una.
pub fn is_protected(rel: &str, protected: &[String]) -> bool {
    let rel = rel.replace('\\', "/");
    let rel = rel.trim_start_matches("./");
    protected.iter().any(|p| {
        rel.eq_ignore_ascii_case(p)
            || (rel.len() > p.len()
                && rel.as_bytes()[p.len()] == b'/'
                && rel[..p.len()].eq_ignore_ascii_case(p))
    })
}

/// Solo HTTPS y solo los dominios que permite la especificación.
pub fn is_allowed_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed
            .host_str()
            .is_some_and(|h| ALLOWED_HOSTS.contains(&h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_allowlist() {
        assert!(is_allowed_url(
            "https://cdn.modrinth.com/data/AANobbMI/versions/x/sodium.jar"
        ));
        assert!(!is_allowed_url("http://cdn.modrinth.com/a.jar"));
        assert!(!is_allowed_url("https://evil.example.com/a.jar"));
        assert!(!is_allowed_url(
            "https://cdn.modrinth.com.evil.example/a.jar"
        ));
    }

    #[test]
    fn protected_paths() {
        let protected = vec!["world".to_string(), "server.properties".to_string()];
        assert!(is_protected("world/level.dat", &protected));
        assert!(is_protected("World/region/r.0.0.mca", &protected));
        assert!(is_protected("server.properties", &protected));
        assert!(!is_protected("worldedit/config.yml", &protected));
        assert!(!is_protected("config/server.properties.bak", &protected));
    }

    #[test]
    fn loader_detection() {
        let mut deps = HashMap::new();
        deps.insert("minecraft".into(), "1.20.1".into());
        deps.insert("fabric-loader".into(), "0.16.9".into());
        assert_eq!(
            loader_from_dependencies(&deps),
            Loader::Fabric {
                loader_version: "0.16.9".into()
            }
        );
    }
}
