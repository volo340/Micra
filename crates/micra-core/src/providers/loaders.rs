//! Instalación del "jar de arranque" según el loader:
//!
//! - Vanilla: `server.jar` del manifiesto de Mojang.
//! - Fabric: lanzador de servidor de meta.fabricmc.net (descarga vanilla solo).
//! - Forge / NeoForge: se descarga el instalador oficial y se ejecuta con
//!   `--installServer` usando el Java gestionado.

use std::path::Path;

use serde::Deserialize;

use crate::download::{download_file, ExpectedHash, HashAlgo, Progress};
use crate::java::JavaRuntime;
use crate::providers::mojang;
use crate::server::config::{LaunchTarget, Loader};
use crate::server::detect::detect_launch_target;
use crate::{Error, Result};

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
            run_installer(http, &url, server_dir, java, progress).await
        }
        Loader::NeoForge { version } => {
            let url = format!(
                "https://maven.neoforged.net/releases/net/neoforged/neoforge/{version}/neoforge-{version}-installer.jar"
            );
            run_installer(http, &url, server_dir, java, progress).await
        }
        Loader::Quilt { .. } => Err(Error::UnsupportedPlatform(
            "Quilt todavía no está soportado".into(),
        )),
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
        .ok_or_else(|| Error::NotFound(format!("server.jar para {mc_version}")))?;
    let expected = ExpectedHash::new(HashAlgo::Sha1, server.sha1);
    download_file(http, &server.url, &server_dir.join("server.jar"), Some(&expected), progress).await?;
    Ok(LaunchTarget::Jar { path: "server.jar".into() })
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
        .get(format!("https://meta.fabricmc.net/v2/versions/loader/{mc_version}"))
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
        .ok_or_else(|| Error::NotFound(format!("Fabric para Minecraft {mc_version}")))
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
        .ok_or_else(|| Error::NotFound("instalador estable de Fabric".into()))?;
    let url = format!(
        "https://meta.fabricmc.net/v2/versions/loader/{mc_version}/{loader_version}/{}/server/jar",
        installer.version
    );
    let jar = "fabric-server-launch.jar";
    download_file(http, &url, &server_dir.join(jar), None, progress).await?;
    Ok(LaunchTarget::Jar { path: jar.into() })
}

async fn run_installer(
    http: &reqwest::Client,
    url: &str,
    server_dir: &Path,
    java: &JavaRuntime,
    progress: Option<Progress<'_>>,
) -> Result<LaunchTarget> {
    let installer = server_dir.join("micra-installer.jar");
    download_file(http, url, &installer, None, progress).await?;

    let mut cmd = tokio::process::Command::new(&java.java_bin);
    cmd.arg("-jar").arg(&installer).arg("--installServer").current_dir(server_dir);
    #[cfg(windows)]
    cmd.creation_flags(crate::server::launcher::CREATE_NO_WINDOW);
    let output = cmd.output().await?;
    let _ = tokio::fs::remove_file(&installer).await;
    let _ = tokio::fs::remove_file(server_dir.join("micra-installer.jar.log")).await;

    if !output.status.success() {
        let tail = String::from_utf8_lossy(&output.stdout);
        let tail: String = tail.lines().rev().take(15).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
        return Err(Error::Api(format!("el instalador del loader falló:\n{tail}")));
    }
    detect_launch_target(server_dir)?
        .ok_or_else(|| Error::NotFound("jar o archivo de argumentos tras instalar el loader".into()))
}
