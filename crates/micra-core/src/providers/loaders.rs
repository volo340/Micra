//! Instalación del "jar de arranque" según el loader:
//!
//! - Vanilla: `server.jar` del manifiesto de Mojang.
//! - Fabric: lanzador de servidor de meta.fabricmc.net (descarga vanilla solo).
//! - Forge / NeoForge: se descarga el instalador oficial y se ejecuta con
//!   `--installServer` usando el Java gestionado.
//!
//! Todo se verifica antes de usarse: el `server.jar` con el SHA-1 del
//! manifiesto de Mojang y los instaladores con el `.sha1` que publica su
//! repositorio Maven (si falta, no se instala). El lanzador de Fabric lo
//! genera al vuelo meta.fabricmc.net y no tiene hash publicado: se descarga
//! solo por HTTPS desde ese dominio oficial.

use std::path::Path;

use serde::Deserialize;

use crate::download::{download_file, ExpectedHash, HashAlgo, Progress};
use crate::java::JavaRuntime;
use crate::providers::mojang;
use crate::server::config::{LaunchTarget, Loader};
use crate::server::detect::detect_launch_target;
use crate::{tr, Error, Result};

pub async fn install_loader(
    http: &reqwest::Client,
    loader: &Loader,
    mc_version: &str,
    server_dir: &Path,
    java: &JavaRuntime,
    progress: Option<Progress<'_>>,
) -> Result<LaunchTarget> {
    tokio::fs::create_dir_all(server_dir).await?;
    match loader {
        Loader::Vanilla => install_vanilla(http, mc_version, server_dir, progress).await,
        Loader::Fabric { loader_version } => {
            install_fabric(http, mc_version, loader_version, server_dir, progress).await
        }
        Loader::Forge { version } => {
            let full = format!("{mc_version}-{version}");
            let url = format!(
                "https://maven.minecraftforge.net/net/minecraftforge/forge/{full}/forge-{full}-installer.jar"
            );
            download_and_run_installer(http, &url, server_dir, java, progress).await
        }
        Loader::NeoForge { version } => {
            let url = format!(
                "https://maven.neoforged.net/releases/net/neoforged/neoforge/{version}/neoforge-{version}-installer.jar"
            );
            download_and_run_installer(http, &url, server_dir, java, progress).await
        }
        Loader::Quilt { .. } => Err(Error::UnsupportedPlatform(tr!(
            "Quilt todavía no está soportado",
            "Quilt is not supported yet"
        ))),
    }
}

async fn install_vanilla(
    http: &reqwest::Client,
    mc_version: &str,
    server_dir: &Path,
    progress: Option<Progress<'_>>,
) -> Result<LaunchTarget> {
    let details = mojang::fetch_version(http, mc_version).await?;
    let server = details
        .downloads
        .server
        .ok_or_else(|| Error::NotFound(format!("server.jar ({mc_version})")))?;
    let expected = ExpectedHash::new(HashAlgo::Sha1, server.sha1);
    download_file(
        http,
        &server.url,
        &server_dir.join("server.jar"),
        Some(&expected),
        progress,
    )
    .await?;
    Ok(LaunchTarget::Jar {
        path: "server.jar".into(),
    })
}

#[derive(Deserialize)]
struct FabricInstaller {
    version: String,
    stable: bool,
}

#[derive(Deserialize)]
struct FabricLoaderEntry {
    loader: FabricLoader,
}

#[derive(Deserialize)]
struct FabricLoader {
    version: String,
    stable: bool,
}

/// Última versión estable del loader de Fabric compatible con `mc_version`.
pub async fn latest_fabric_loader(http: &reqwest::Client, mc_version: &str) -> Result<String> {
    let entries: Vec<FabricLoaderEntry> = http
        .get(format!(
            "https://meta.fabricmc.net/v2/versions/loader/{mc_version}"
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    entries
        .iter()
        .find(|e| e.loader.stable)
        .or(entries.first())
        .map(|e| e.loader.version.clone())
        .ok_or_else(|| Error::NotFound(format!("Fabric (Minecraft {mc_version})")))
}

async fn install_fabric(
    http: &reqwest::Client,
    mc_version: &str,
    loader_version: &str,
    server_dir: &Path,
    progress: Option<Progress<'_>>,
) -> Result<LaunchTarget> {
    let installers: Vec<FabricInstaller> = http
        .get("https://meta.fabricmc.net/v2/versions/installer")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let installer = installers
        .iter()
        .find(|i| i.stable)
        .ok_or_else(|| Error::NotFound("Fabric installer".into()))?;
    let url = format!(
        "https://meta.fabricmc.net/v2/versions/loader/{mc_version}/{loader_version}/{}/server/jar",
        installer.version
    );
    let jar = "fabric-server-launch.jar";
    download_file(http, &url, &server_dir.join(jar), None, progress).await?;
    Ok(LaunchTarget::Jar { path: jar.into() })
}

/// Descarga el instalador y su `.sha1` del mismo repositorio Maven, y solo
/// lo ejecuta si coinciden.
async fn download_and_run_installer(
    http: &reqwest::Client,
    url: &str,
    server_dir: &Path,
    java: &JavaRuntime,
    progress: Option<Progress<'_>>,
) -> Result<LaunchTarget> {
    let sha1 = maven_sha1(http, url).await?;
    let installer = server_dir.join("micra-installer.jar");
    let expected = ExpectedHash::new(HashAlgo::Sha1, sha1);
    download_file(http, url, &installer, Some(&expected), progress).await?;
    let result = run_installer(&installer, server_dir, java).await;
    let _ = tokio::fs::remove_file(&installer).await;
    result
}

/// Hash publicado junto a un artefacto Maven (`<url>.sha1`).
async fn maven_sha1(http: &reqwest::Client, url: &str) -> Result<String> {
    let missing = || {
        Error::Api(tr!(
            "el repositorio no publica el hash del instalador, así que no se puede verificar y no se instalará",
            "the repository does not publish the installer's hash, so it cannot be verified and will not be installed"
        ))
    };
    let res = http.get(format!("{url}.sha1")).send().await?;
    if !res.status().is_success() {
        return Err(missing());
    }
    let text = res.text().await?;
    parse_sha1(&text).ok_or_else(missing)
}

/// El archivo `.sha1` puede traer solo el hash o "hash  nombre".
fn parse_sha1(text: &str) -> Option<String> {
    let hash = text.split_whitespace().next()?.to_ascii_lowercase();
    (hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hash)
}

/// Ejecuta un instalador de Forge/NeoForge ya presente en disco (descargado
/// y verificado por nosotros, o incluido en un server pack verificado) con
/// `--installServer` y devuelve cómo arrancar el resultado.
pub async fn run_installer(
    installer: &Path,
    server_dir: &Path,
    java: &JavaRuntime,
) -> Result<LaunchTarget> {
    let mut cmd = tokio::process::Command::new(&java.java_bin);
    cmd.arg("-jar")
        .arg(installer)
        .arg("--installServer")
        .current_dir(server_dir);
    #[cfg(windows)]
    cmd.creation_flags(crate::server::launcher::CREATE_NO_WINDOW);
    let output = cmd.output().await?;
    let mut log = installer.as_os_str().to_os_string();
    log.push(".log");
    let _ = tokio::fs::remove_file(&log).await;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.lines().collect();
        let tail = lines[lines.len().saturating_sub(15)..].join("\n");
        return Err(Error::Api(tr!(
            "el instalador del loader falló:\n{tail}",
            "the loader installer failed:\n{tail}"
        )));
    }
    detect_launch_target(server_dir)?.ok_or_else(|| {
        Error::NotFound(tr!(
            "jar o archivo de argumentos tras instalar el loader",
            "jar or arguments file after installing the loader"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::parse_sha1;

    #[test]
    fn maven_sha1_files() {
        let h = "0123456789abcdef0123456789ABCDEF01234567";
        assert_eq!(
            parse_sha1(h).as_deref(),
            Some(h.to_ascii_lowercase().as_str())
        );
        assert_eq!(
            parse_sha1(&format!("{h}  forge-installer.jar\n")).as_deref(),
            Some(h.to_ascii_lowercase().as_str())
        );
        assert_eq!(parse_sha1("<html>404</html>"), None);
        assert_eq!(parse_sha1(""), None);
    }
}
