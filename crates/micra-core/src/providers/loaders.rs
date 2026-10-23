//! Instalación del "jar de arranque" según el loader:
//!
//! - Vanilla: `server.jar` del manifiesto de Mojang.
//! - Fabric: lanzador de servidor de meta.fabricmc.net (descarga vanilla solo).
//! - Forge / NeoForge: se descarga el instalador oficial y se ejecuta con
//!   `--installServer` usando el Java gestionado.
//!
//! El asistente elige la versión de Minecraft según el loader: Fabric la saca
//! de meta.fabricmc.net, Forge de sus versiones promocionadas y NeoForge de
//! su repositorio Maven.
//!
//! Todo se verifica antes de usarse: el `server.jar` con el SHA-1 del
//! manifiesto de Mojang y los instaladores con el `.sha1` que publica su
//! repositorio Maven (si falta, no se instala). El lanzador de Fabric lo
//! genera al vuelo meta.fabricmc.net y no tiene hash publicado: se descarga
//! solo por HTTPS desde ese dominio oficial.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::download::{download_file, ExpectedHash, HashAlgo, Progress};
use crate::java::JavaRuntime;
use crate::providers::mojang;
use crate::server::config::{LaunchTarget, Loader};
use crate::server::detect::detect_launch_target;
use crate::server::version::{at_least, compare};
use crate::{tr, Error, Result};

const FORGE_PROMOS: &str =
    "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
const NEOFORGE_VERSIONS: &str =
    "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge";

/// Loaders con los que el asistente crea servidores con mods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModLoader {
    Fabric,
    Forge,
    NeoForge,
}

/// Versiones de Minecraft que tienen servidor con este loader, de la más
/// nueva a la más antigua.
pub async fn game_versions(http: &reqwest::Client, loader: ModLoader) -> Result<Vec<String>> {
    let mut versions = match loader {
        ModLoader::Fabric => {
            let list: Vec<FabricGame> =
                get_json(http, "https://meta.fabricmc.net/v2/versions/game").await?;
            list.into_iter()
                .filter(|g| g.stable)
                .map(|g| g.version)
                .collect()
        }
        ModLoader::Forge => forge_mc_versions(&forge_promos(http).await?),
        ModLoader::NeoForge => neoforge_versions(http)
            .await?
            .iter()
            .filter_map(|v| neoforge_mc_version(v))
            .collect(),
    };
    versions.sort_by(|a, b| compare(b, a));
    versions.dedup();
    Ok(versions)
}

async fn get_json<T: serde::de::DeserializeOwned>(http: &reqwest::Client, url: &str) -> Result<T> {
    Ok(http
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

#[derive(Deserialize)]
struct FabricGame {
    version: String,
    stable: bool,
}

#[derive(Deserialize)]
struct ForgePromos {
    promos: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct MavenVersions {
    versions: Vec<String>,
}

async fn forge_promos(http: &reqwest::Client) -> Result<BTreeMap<String, String>> {
    Ok(get_json::<ForgePromos>(http, FORGE_PROMOS).await?.promos)
}

async fn neoforge_versions(http: &reqwest::Client) -> Result<Vec<String>> {
    Ok(get_json::<MavenVersions>(http, NEOFORGE_VERSIONS)
        .await?
        .versions)
}

/// Versiones de Minecraft con Forge promocionado (`1.20.1-recommended`).
/// Antes de 1.12 los instaladores usan otro nombre de archivo.
fn forge_mc_versions(promos: &BTreeMap<String, String>) -> Vec<String> {
    promos
        .keys()
        .filter_map(|k| {
            k.strip_suffix("-recommended")
                .or_else(|| k.strip_suffix("-latest"))
        })
        .filter(|mc| at_least(mc, "1.12"))
        .map(str::to_string)
        .collect()
}

/// La versión recomendada de Forge para `mc` o, si no hay, la última.
fn forge_version_for(promos: &BTreeMap<String, String>, mc_version: &str) -> Option<String> {
    promos
        .get(&format!("{mc_version}-recommended"))
        .or_else(|| promos.get(&format!("{mc_version}-latest")))
        .cloned()
}

pub async fn latest_forge(http: &reqwest::Client, mc_version: &str) -> Result<String> {
    forge_version_for(&forge_promos(http).await?, mc_version)
        .ok_or_else(|| Error::NotFound(format!("Forge (Minecraft {mc_version})")))
}

/// Versión de Minecraft de una versión de NeoForge. Hasta 1.21.x se numera
/// con la versión menor y el parche de Minecraft (`21.1.77` es 1.21.1 y
/// `21.0.5-beta` es 1.21); desde 26.1, con la versión entera más el build
/// (`26.1.2.122` es 26.1.2 y `26.3.0.95` es 26.3). Las alfas y las de
/// snapshots no cuentan.
pub fn neoforge_mc_version(version: &str) -> Option<String> {
    if version.contains("alpha") || version.contains("snapshot") {
        return None;
    }
    let base = version.split(['-', '+']).next()?;
    let parts: Vec<u32> = base
        .split('.')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    match parts.as_slice() {
        [year, drop, 0, _build] if *year >= 26 => Some(format!("{year}.{drop}")),
        [year, drop, hotfix, _build] if *year >= 26 => Some(format!("{year}.{drop}.{hotfix}")),
        [minor, 0, _build] if (20..26).contains(minor) => Some(format!("1.{minor}")),
        [minor, patch, _build] if (20..26).contains(minor) => Some(format!("1.{minor}.{patch}")),
        _ => None,
    }
}

/// La versión estable más nueva de NeoForge para `mc` (o la beta más nueva
/// si aún no hay estable).
fn neoforge_version_for(versions: &[String], mc_version: &str) -> Option<String> {
    let mut matching: Vec<&String> = versions
        .iter()
        .filter(|v| neoforge_mc_version(v).as_deref() == Some(mc_version))
        .collect();
    matching.sort_by(|a, b| {
        let stable = |v: &str| !v.contains("beta");
        stable(a).cmp(&stable(b)).then_with(|| compare(a, b))
    });
    matching.last().map(|v| v.to_string())
}

pub async fn latest_neoforge(http: &reqwest::Client, mc_version: &str) -> Result<String> {
    neoforge_version_for(&neoforge_versions(http).await?, mc_version)
        .ok_or_else(|| Error::NotFound(format!("NeoForge (Minecraft {mc_version})")))
}

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
    use super::*;

    #[test]
    fn forge_versions_from_promotions() {
        let promos: BTreeMap<String, String> = [
            ("1.7.10-recommended", "10.13.4.1614"),
            ("1.12.2-latest", "14.23.5.2860"),
            ("1.20.1-latest", "47.4.10"),
            ("1.20.1-recommended", "47.4.0"),
            ("26.3-latest", "66.1.3"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let mut versions = forge_mc_versions(&promos);
        versions.sort_by(|a, b| compare(b, a));
        versions.dedup();
        assert_eq!(versions, vec!["26.3", "1.20.1", "1.12.2"]);
        assert_eq!(
            forge_version_for(&promos, "1.20.1").as_deref(),
            Some("47.4.0")
        );
        assert_eq!(
            forge_version_for(&promos, "26.3").as_deref(),
            Some("66.1.3")
        );
        assert_eq!(forge_version_for(&promos, "1.19.2"), None);
    }

    #[test]
    fn neoforge_versions() {
        assert_eq!(neoforge_mc_version("21.1.77").as_deref(), Some("1.21.1"));
        assert_eq!(neoforge_mc_version("21.0.5-beta").as_deref(), Some("1.21"));
        assert_eq!(neoforge_mc_version("20.2.86").as_deref(), Some("1.20.2"));
        assert_eq!(neoforge_mc_version("20.4.237").as_deref(), Some("1.20.4"));
        assert_eq!(neoforge_mc_version("26.3.0.95").as_deref(), Some("26.3"));
        assert_eq!(neoforge_mc_version("26.1.2.122").as_deref(), Some("26.1.2"));
        assert_eq!(neoforge_mc_version("26.1.0.0-alpha.3+snapshot-2"), None);
        assert_eq!(neoforge_mc_version("0.25w14craftmine.3-beta"), None);

        let all: Vec<String> = [
            "21.1.9-beta",
            "21.1.77",
            "21.1.200",
            "21.0.5-beta",
            "21.0.10-beta",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        assert_eq!(
            neoforge_version_for(&all, "1.21.1").as_deref(),
            Some("21.1.200")
        );
        // Sin estable, la beta más nueva.
        assert_eq!(
            neoforge_version_for(&all, "1.21").as_deref(),
            Some("21.0.10-beta")
        );
        assert_eq!(neoforge_version_for(&all, "1.20.1"), None);
    }

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
