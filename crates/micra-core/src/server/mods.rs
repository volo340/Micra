//! Gestión de los mods de un servidor: listar, activar/desactivar, quitar,
//! instalar desde Modrinth o CurseForge (con sus dependencias) y actualizar.
//!
//! Todo lo que se descarga se verifica con el hash que publica su fuente
//! (SHA-512 en Modrinth, SHA-1 en CurseForge) y pasa por el escáner de
//! malware antes de llegar a `mods/`. Las versiones que se sustituyen o
//! quitan se guardan en `.micra/old-mods/` por si hay que volver.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::download::{download_file, ExpectedHash, HashAlgo};
use crate::fsutil::{check_no_links, write_atomic};
use crate::providers::curseforge::{CfFile, CfMod, CurseForgeClient};
use crate::providers::modrinth::{ModrinthClient, Version};
use crate::providers::mrpack::is_allowed_url;
use crate::providers::search::{curseforge_missing, dedupe_key, Source};
use crate::server::config::ServerConfig;
use crate::server::pack::PackManifest;
use crate::server::scan::{self, Severity};
use crate::{tr, Error, Result};

pub const MODS_DIR: &str = "mods";
const DISABLED_SUFFIX: &str = ".disabled";
const OLD_MODS_DIR: &str = ".micra/old-mods";
const DOWNLOADS_DIR: &str = ".micra/downloads";
/// Mods instalados desde CurseForge (ver `CfRegistry`).
const CURSEFORGE_REGISTRY: &str = ".micra/curseforge-mods.json";
/// Tope de mods que puede arrastrar una instalación por dependencias.
const MAX_INSTALLS: usize = 25;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModInfo {
    /// Nombre real en disco (con `.disabled` si está desactivado).
    pub file_name: String,
    pub enabled: bool,
    pub size: u64,
    pub sha1: String,
    pub mod_id: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    /// Lo instaló el modpack (una actualización del pack puede cambiarlo).
    pub from_pack: bool,
    /// Sus metadatos dicen que solo funciona en el juego (cliente).
    pub client_only: bool,
}

/// Lo que sabe Modrinth o CurseForge de un mod instalado.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModRemoteInfo {
    pub sha1: String,
    pub source: Source,
    pub project_id: String,
    pub title: Option<String>,
    pub icon_url: Option<String>,
    pub version_number: String,
    /// Su fuente dice que solo funciona en el juego (cliente).
    pub client_only: bool,
    pub update: Option<ModUpdate>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModUpdate {
    /// Id de la versión de Modrinth o del archivo de CurseForge.
    pub version_id: String,
    pub version_number: String,
    pub file_name: String,
}

/// Clientes con los que se buscan y descargan mods.
#[derive(Clone, Copy)]
pub struct Sources<'a> {
    pub http: &'a reqwest::Client,
    pub modrinth: &'a ModrinthClient,
    /// Sin clave de API no hay CurseForge.
    pub curseforge: Option<&'a CurseForgeClient>,
}

/// Mods instalados desde CurseForge, por nombre de archivo (sin
/// `.disabled`). CurseForge no identifica archivos por su hash como
/// Modrinth: Micra apunta de qué proyecto es cada uno para buscar sus
/// actualizaciones.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CfRegistry {
    #[serde(default)]
    pub mods: BTreeMap<String, CfEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfEntry {
    pub mod_id: u64,
    pub file_id: u64,
}

impl CfRegistry {
    pub async fn load(server_dir: &Path) -> Self {
        match tokio::fs::read(server_dir.join(CURSEFORGE_REGISTRY)).await {
            Ok(raw) => serde_json::from_slice(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub async fn save(&self, server_dir: &Path) -> Result<()> {
        crate::fsutil::internal_dir(server_dir, ".micra")?;
        let path = server_dir.join(CURSEFORGE_REGISTRY);
        write_atomic(&path, &serde_json::to_vec_pretty(self)?).await
    }

    pub fn get(&self, file_name: &str) -> Option<CfEntry> {
        self.mods.get(base_name(file_name)).copied()
    }
}

/// Nombre de archivo de un mod: sin rutas y terminado en `.jar` (o
/// `.jar.disabled`).
pub fn validate_mod_file_name(name: &str) -> Result<()> {
    let base = base_name(name);
    let ok = base.to_ascii_lowercase().ends_with(".jar")
        && base.len() > 4
        && name.len() <= 200
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':', '\0']);
    if ok {
        Ok(())
    } else {
        Err(Error::UnsafePath(name.to_string()))
    }
}

/// El nombre sin `.disabled`.
fn base_name(file_name: &str) -> &str {
    file_name.strip_suffix(DISABLED_SUFFIX).unwrap_or(file_name)
}

fn is_mod_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".jar") || lower.ends_with(".jar.disabled")
}

pub async fn list_mods(server_dir: &Path) -> Result<Vec<ModInfo>> {
    let dir = server_dir.to_owned();
    let pack_files: HashSet<String> = PackManifest::load(server_dir)
        .await
        .map(|m| m.files.into_iter().collect())
        .unwrap_or_default();
    tokio::task::spawn_blocking(move || {
        let Ok(entries) = std::fs::read_dir(dir.join(MODS_DIR)) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if !is_mod_file(&name) || !entry.file_type().is_ok_and(|t| t.is_file()) {
                continue;
            }
            let path = entry.path();
            let meta = read_metadata(&path);
            let enabled = !name.ends_with(DISABLED_SUFFIX);
            out.push(ModInfo {
                from_pack: pack_files.contains(&format!("{MODS_DIR}/{}", base_name(&name))),
                sha1: scan::sha1_file(&path)?,
                size: entry.metadata()?.len(),
                enabled,
                mod_id: meta.id,
                name: meta.name,
                version: meta.version,
                client_only: meta.client_only,
                file_name: name,
            });
        }
        out.sort_by_key(|m| {
            m.name
                .clone()
                .unwrap_or_else(|| m.file_name.clone())
                .to_lowercase()
        });
        Ok(out)
    })
    .await
    .expect("la lectura de mods no debería entrar en pánico")
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ModMetadata {
    pub id: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    /// El mod dice que solo funciona en el juego (cliente).
    pub client_only: bool,
}

/// Lee id, nombre y versión de los metadatos de cada loader.
pub fn read_metadata(jar: &Path) -> ModMetadata {
    let Ok(file) = std::fs::File::open(jar) else {
        return ModMetadata::default();
    };
    let Ok(mut zip) = zip::ZipArchive::new(file) else {
        return ModMetadata::default();
    };
    let mut read = |name: &str| -> Option<String> {
        let entry = zip.by_name(name).ok()?;
        if entry.size() > 1024 * 1024 {
            return None;
        }
        let mut text = String::new();
        entry.take(1024 * 1024).read_to_string(&mut text).ok()?;
        Some(text)
    };

    let mut meta = if let Some(json) = read("fabric.mod.json") {
        let v: serde_json::Value =
            serde_json::from_str(&strip_json_comments(&json)).unwrap_or_default();
        ModMetadata {
            id: str_field(&v["id"]),
            name: str_field(&v["name"]),
            version: str_field(&v["version"]),
            client_only: v["environment"].as_str() == Some("client"),
        }
    } else if let Some(json) = read("quilt.mod.json") {
        let v: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
        let q = &v["quilt_loader"];
        ModMetadata {
            id: str_field(&q["id"]),
            name: str_field(&q["metadata"]["name"]),
            version: str_field(&q["version"]),
            client_only: v["minecraft"]["environment"].as_str() == Some("client"),
        }
    } else if let Some(toml) =
        read("META-INF/neoforge.mods.toml").or_else(|| read("META-INF/mods.toml"))
    {
        parse_mods_toml(&toml)
    } else if let Some(json) = read("mcmod.info") {
        let v: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
        let first = if v.is_array() {
            &v[0]
        } else {
            &v["modList"][0]
        };
        ModMetadata {
            id: str_field(&first["modid"]),
            name: str_field(&first["name"]),
            version: str_field(&first["version"]),
            client_only: false,
        }
    } else {
        ModMetadata::default()
    };
    // Forge rellena `${file.jarVersion}` con la versión del manifiesto.
    if meta.version.as_deref().is_none_or(|v| v.contains("${")) {
        meta.version = read("META-INF/MANIFEST.MF").and_then(|m| {
            m.lines()
                .find_map(|l| l.strip_prefix("Implementation-Version:"))
                .map(|v| v.trim().to_string())
        });
    }
    meta
}

/// `read_metadata` sin bloquear.
async fn metadata_of(jar: &Path) -> ModMetadata {
    let jar = jar.to_owned();
    tokio::task::spawn_blocking(move || read_metadata(&jar))
        .await
        .unwrap_or_default()
}

fn str_field(v: &serde_json::Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Algunos `fabric.mod.json` llevan comentarios `//` (Fabric los tolera).
fn strip_json_comments(json: &str) -> String {
    json.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Lo justo de TOML para el primer `[[mods]]` de `mods.toml` (y
/// `clientSideOnly`, que va antes de las secciones).
fn parse_mods_toml(toml: &str) -> ModMetadata {
    let mut meta = ModMetadata::default();
    let mut in_root = true;
    let mut in_mods = false;
    for line in toml.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            if in_mods {
                break;
            }
            in_root = false;
            in_mods = line == "[[mods]]";
            continue;
        }
        if !in_mods && !in_root {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let value = value
            .split_once('#')
            .filter(|(v, _)| v.matches('"').count() % 2 == 0)
            .map_or(value, |(v, _)| v)
            .trim()
            .trim_matches(|c| c == '"' || c == '\'')
            .to_string();
        match (in_root, key.trim()) {
            (true, "clientSideOnly") => meta.client_only = value == "true",
            (false, "modId") => meta.id = Some(value),
            (false, "version") => meta.version = Some(value),
            (false, "displayName") => meta.name = Some(value),
            _ => {}
        }
    }
    meta
}

/// Activa o desactiva un mod renombrándolo (`.jar` ↔ `.jar.disabled`).
pub async fn set_enabled(server_dir: &Path, file_name: &str, enabled: bool) -> Result<String> {
    validate_mod_file_name(file_name)?;
    let base = base_name(file_name).to_string();
    let target = if enabled {
        base.clone()
    } else {
        format!("{base}{DISABLED_SUFFIX}")
    };
    if target != file_name {
        let mods = server_dir.join(MODS_DIR);
        check_no_links(server_dir, &mods.join(file_name))?;
        tokio::fs::rename(mods.join(file_name), mods.join(&target)).await?;
    }
    Ok(target)
}

/// Quita un mod (se guarda una copia en `.micra/old-mods`).
pub async fn remove_mod(server_dir: &Path, file_name: &str) -> Result<()> {
    validate_mod_file_name(file_name)?;
    move_to_old(server_dir, &server_dir.join(MODS_DIR).join(file_name)).await?;
    let mut registry = CfRegistry::load(server_dir).await;
    if registry.mods.remove(base_name(file_name)).is_some() {
        registry.save(server_dir).await?;
    }
    Ok(())
}

async fn move_to_old(server_dir: &Path, path: &Path) -> Result<()> {
    check_no_links(server_dir, path)?;
    let old = crate::fsutil::internal_dir(server_dir, OLD_MODS_DIR)?;
    let name = path.file_name().unwrap_or_default();
    tokio::fs::rename(path, old.join(name)).await?;
    Ok(())
}

/// Descarga el archivo principal de una versión de Modrinth a una carpeta
/// temporal, verificado y analizado. Devuelve la ruta temporal.
async fn fetch_verified(
    http: &reqwest::Client,
    server_dir: &Path,
    version: &Version,
) -> Result<(PathBuf, String)> {
    let file = version
        .primary_file()
        .ok_or_else(|| Error::NotFound(version.name.clone()))?;
    validate_mod_file_name(&file.filename)?;
    if !is_allowed_url(&file.url) {
        return Err(Error::DisallowedUrl(file.url.clone()));
    }
    let sha512 = file.hashes.sha512.as_ref().ok_or_else(|| {
        let name = &file.filename;
        Error::Api(tr!(
            "Modrinth no publicó el hash de {name}, así que no se puede verificar",
            "Modrinth did not publish the hash of {name}, so it cannot be verified"
        ))
    })?;
    let tmp = crate::fsutil::internal_dir(server_dir, DOWNLOADS_DIR)?.join(&file.filename);
    download_file(
        http,
        &file.url,
        &tmp,
        Some(&ExpectedHash::new(HashAlgo::Sha512, sha512)),
        None,
    )
    .await?;
    check_malware(&tmp, &file.filename).await?;
    Ok((tmp, file.filename.clone()))
}

/// Como `fetch_verified`, para un archivo de CurseForge (con su SHA-1).
async fn fetch_curseforge(
    cf: &CurseForgeClient,
    server_dir: &Path,
    file: &CfFile,
) -> Result<PathBuf> {
    validate_mod_file_name(&file.file_name)?;
    if file.sha1().is_none() {
        let name = &file.file_name;
        return Err(Error::Api(tr!(
            "CurseForge no publicó el hash de {name}, así que no se puede verificar",
            "CurseForge did not publish the hash of {name}, so it cannot be verified"
        )));
    }
    let tmp = crate::fsutil::internal_dir(server_dir, DOWNLOADS_DIR)?.join(&file.file_name);
    cf.download(file, &tmp, None).await?;
    check_malware(&tmp, &file.file_name).await?;
    Ok(tmp)
}

/// Borra la descarga si el escáner encuentra malware conocido.
async fn check_malware(tmp: &Path, name: &str) -> Result<()> {
    let path = tmp.to_owned();
    let findings = tokio::task::spawn_blocking(move || scan::scan_jar(&path))
        .await
        .expect("el análisis no debería entrar en pánico")?;
    if findings.iter().any(|f| f.severity == Severity::Critical) {
        let _ = tokio::fs::remove_file(tmp).await;
        return Err(Error::Malware(name.to_string()));
    }
    Ok(())
}

fn ensure_mods_supported(cfg: &ServerConfig) -> Result<()> {
    if cfg.loader.supports_mods() {
        Ok(())
    } else {
        Err(Error::InvalidConfig(tr!(
            "un servidor vanilla no puede cargar mods",
            "a vanilla server cannot load mods"
        )))
    }
}

fn not_compatible(name: &str, cfg: &ServerConfig) -> Error {
    let mc = &cfg.mc_version;
    Error::InvalidConfig(tr!(
        "«{name}» no es compatible con este servidor (Minecraft {mc})",
        "\"{name}\" is not compatible with this server (Minecraft {mc})"
    ))
}

fn already_installed(name: &str, existing: &str) -> Error {
    Error::InvalidConfig(tr!(
        "«{name}» ya está instalado ({existing})",
        "\"{name}\" is already installed ({existing})"
    ))
}

fn client_only(name: &str) -> Error {
    Error::InvalidConfig(tr!(
        "«{name}» es solo para el juego de cada jugador (cliente): en el servidor no hace nada y puede impedir que arranque",
        "\"{name}\" is only for each player's game (client): it does nothing on the server and can stop it from starting"
    ))
}

fn check_compatible(version: &Version, cfg: &ServerConfig) -> Result<()> {
    let loaders = cfg.loader.modrinth_loaders(&cfg.mc_version);
    let loader_ok = version
        .loaders
        .iter()
        .any(|l| loaders.contains(&l.as_str()));
    let mc_ok = version.game_versions.iter().any(|g| g == &cfg.mc_version);
    if loader_ok && mc_ok {
        Ok(())
    } else {
        Err(not_compatible(&version.name, cfg))
    }
}

/// Versión más adecuada de un proyecto para el servidor: la release más
/// nueva y, si no hay, la versión más nueva.
async fn best_version(
    modrinth: &ModrinthClient,
    project: &str,
    cfg: &ServerConfig,
) -> Result<Version> {
    let loaders = cfg.loader.modrinth_loaders(&cfg.mc_version);
    let versions = modrinth
        .project_versions_any(project, Some(&cfg.mc_version), &loaders)
        .await?;
    let pick = versions
        .iter()
        .find(|v| v.version_type == "release")
        .or(versions.first())
        .cloned();
    pick.ok_or_else(|| {
        let mc = &cfg.mc_version;
        Error::NotFound(tr!(
            "una versión de {project} para Minecraft {mc}",
            "a version of {project} for Minecraft {mc}"
        ))
    })
}

/// Id del loader de cada mod instalado → su archivo, para no instalar dos
/// veces el mismo mod aunque venga de otra fuente o con otro nombre.
fn installed_ids(mods: &[ModInfo]) -> HashMap<String, String> {
    mods.iter()
        .filter_map(|m| Some((m.mod_id.as_ref()?.to_lowercase(), m.file_name.clone())))
        .collect()
}

/// Lleva a `mods/` un archivo ya verificado. Si ese mod ya está instalado
/// (con otro nombre o de otra fuente) lo descarta: error si es el que se
/// pidió y `false` si es una dependencia.
async fn place(
    server_dir: &Path,
    tmp: &Path,
    name: &str,
    ids: &mut HashMap<String, String>,
    requested: bool,
) -> Result<bool> {
    let meta = metadata_of(tmp).await;
    if let Some(id) = meta.id.as_deref().map(str::to_lowercase) {
        if let Some(existing) = ids.get(&id) {
            let _ = tokio::fs::remove_file(tmp).await;
            if requested {
                return Err(already_installed(
                    meta.name.as_deref().unwrap_or(name),
                    existing,
                ));
            }
            return Ok(false);
        }
        ids.insert(id, name.to_string());
    }
    let target = server_dir.join(MODS_DIR).join(name);
    check_no_links(server_dir, &target)?;
    tokio::fs::rename(tmp, target).await?;
    Ok(true)
}

/// Instala un proyecto de Modrinth o CurseForge (o una versión concreta) y
/// sus dependencias obligatorias que falten. Devuelve los archivos
/// instalados.
pub async fn install(
    sources: Sources<'_>,
    server_dir: &Path,
    cfg: &ServerConfig,
    source: Source,
    project_id: &str,
    version_id: Option<&str>,
) -> Result<Vec<String>> {
    match source {
        Source::Modrinth => {
            install_from_modrinth(
                sources.http,
                sources.modrinth,
                server_dir,
                cfg,
                project_id,
                version_id,
            )
            .await
        }
        Source::Curseforge => {
            let cf = sources.curseforge.ok_or_else(curseforge_missing)?;
            let mod_id = curseforge_id(project_id)?;
            let file_id = version_id.map(curseforge_id).transpose()?;
            install_from_curseforge(cf, server_dir, cfg, mod_id, file_id).await
        }
    }
}

fn curseforge_id(id: &str) -> Result<u64> {
    id.parse().map_err(|_| {
        Error::InvalidConfig(tr!(
            "id de CurseForge no válido: {id}",
            "invalid CurseForge id: {id}"
        ))
    })
}

/// Instala un proyecto de Modrinth (o una versión concreta) y sus
/// dependencias obligatorias que falten. Devuelve los archivos instalados.
pub async fn install_from_modrinth(
    http: &reqwest::Client,
    modrinth: &ModrinthClient,
    server_dir: &Path,
    cfg: &ServerConfig,
    project_id: &str,
    version_id: Option<&str>,
) -> Result<Vec<String>> {
    ensure_mods_supported(cfg)?;
    let installed = list_mods(server_dir).await?;
    let mut ids = installed_ids(&installed);
    let hashes: Vec<String> = installed.iter().map(|m| m.sha1.clone()).collect();
    let mut present: HashSet<String> = modrinth
        .versions_by_hashes(&hashes)
        .await
        .unwrap_or_default()
        .into_values()
        .map(|v| v.project_id)
        .collect();

    let first = match version_id {
        Some(v) => modrinth.version(v).await?,
        None => best_version(modrinth, project_id, cfg).await?,
    };
    check_compatible(&first, cfg)?;
    let mut queue = VecDeque::from([first]);
    let mut out = Vec::new();
    let mut requested = true;
    tokio::fs::create_dir_all(server_dir.join(MODS_DIR)).await?;
    while let Some(version) = queue.pop_front() {
        if out.len() >= MAX_INSTALLS {
            break;
        }
        let (tmp, name) = fetch_verified(http, server_dir, &version).await?;
        present.insert(version.project_id.clone());
        if !place(server_dir, &tmp, &name, &mut ids, requested).await? {
            continue;
        }
        requested = false;
        out.push(name);
        for dep in version
            .dependencies
            .iter()
            .filter(|d| d.dependency_type == "required")
        {
            let Some(project) = dep.project_id.as_deref() else {
                continue;
            };
            if present.contains(project) || queue.iter().any(|v| v.project_id == project) {
                continue;
            }
            let dep_version = match dep.version_id.as_deref() {
                Some(v) => modrinth.version(v).await?,
                None => best_version(modrinth, project, cfg).await?,
            };
            queue.push_back(dep_version);
        }
    }
    Ok(out)
}

/// Instala un mod de CurseForge (o un archivo concreto) y sus dependencias
/// obligatorias que falten. Devuelve los archivos instalados.
pub async fn install_from_curseforge(
    cf: &CurseForgeClient,
    server_dir: &Path,
    cfg: &ServerConfig,
    mod_id: u64,
    file_id: Option<u64>,
) -> Result<Vec<String>> {
    ensure_mods_supported(cfg)?;
    let mc = cfg.mc_version.as_str();
    let loaders = cfg.loader.modrinth_loaders(mc);
    let installed = list_mods(server_dir).await?;
    let mut ids = installed_ids(&installed);
    // Nombres de lo instalado: una dependencia que ya está no se descarga.
    let names: HashSet<String> = installed
        .iter()
        .filter_map(|m| m.name.as_deref())
        .map(dedupe_key)
        .collect();
    let mut registry = CfRegistry::load(server_dir).await;
    let mut present: HashSet<u64> = installed
        .iter()
        .filter_map(|m| registry.get(&m.file_name))
        .map(|e| e.mod_id)
        .collect();

    let first = match file_id {
        Some(f) => cf.file(mod_id, f).await?,
        None => cf.best_file(mod_id, mc, &loaders).await?,
    };
    if !first.fits(mc, &loaders) {
        return Err(not_compatible(&first.display_name, cfg));
    }
    if first.is_client_only() {
        return Err(client_only(&first.display_name));
    }
    let mut queue = VecDeque::from([first]);
    let mut queued = HashSet::from([mod_id]);
    let mut out = Vec::new();
    let mut requested = true;
    tokio::fs::create_dir_all(server_dir.join(MODS_DIR)).await?;
    while let Some(file) = queue.pop_front() {
        if out.len() >= MAX_INSTALLS {
            break;
        }
        let tmp = fetch_curseforge(cf, server_dir, &file).await?;
        present.insert(file.mod_id);
        if !place(server_dir, &tmp, &file.file_name, &mut ids, requested).await? {
            continue;
        }
        requested = false;
        registry.mods.insert(
            file.file_name.clone(),
            CfEntry {
                mod_id: file.mod_id,
                file_id: file.id,
            },
        );
        registry.save(server_dir).await?;
        out.push(file.file_name.clone());

        let deps: Vec<u64> = file
            .dependencies
            .iter()
            .filter(|d| d.is_required())
            .map(|d| d.mod_id)
            .filter(|id| !present.contains(id) && !queued.contains(id))
            .collect();
        if deps.is_empty() {
            continue;
        }
        let projects = cf.mods(&deps).await.unwrap_or_default();
        for dep in deps {
            queued.insert(dep);
            let project = projects.iter().find(|m| m.id == dep);
            if project.is_some_and(|p| already_there(p, &names, &ids)) {
                continue;
            }
            let file = cf
                .best_file(dep, mc, &loaders)
                .await
                .map_err(|e| match project {
                    Some(p) => {
                        let name = &p.name;
                        Error::Api(tr!(
                            "falta la dependencia «{name}»: {e}",
                            "missing dependency \"{name}\": {e}"
                        ))
                    }
                    None => e,
                })?;
            queue.push_back(file);
        }
    }
    Ok(out)
}

/// Si una dependencia de CurseForge ya está instalada (de otra fuente): por
/// su nombre o porque su slug es el id de un mod instalado.
fn already_there(project: &CfMod, names: &HashSet<String>, ids: &HashMap<String, String>) -> bool {
    names.contains(&dedupe_key(&project.name)) || ids.contains_key(&project.slug.to_lowercase())
}

/// Busca en Modrinth (por su SHA-1) y en CurseForge (los que se instalaron
/// desde allí) los mods instalados y si tienen una versión más nueva para
/// este servidor.
pub async fn check_updates(
    sources: Sources<'_>,
    server_dir: &Path,
    cfg: &ServerConfig,
    mods: &[ModInfo],
) -> Result<Vec<ModRemoteInfo>> {
    let mut out = modrinth_updates(sources.modrinth, cfg, mods).await?;
    if let Some(cf) = sources.curseforge {
        match curseforge_updates(cf, server_dir, cfg, mods).await {
            Ok(found) => {
                // Lo instalado desde CurseForge se actualiza desde CurseForge.
                out.retain(|m| !found.iter().any(|f| f.sha1 == m.sha1));
                out.extend(found);
            }
            Err(e) => tracing::warn!("no se pudo buscar actualizaciones en CurseForge: {e}"),
        }
    }
    Ok(out)
}

async fn modrinth_updates(
    modrinth: &ModrinthClient,
    cfg: &ServerConfig,
    mods: &[ModInfo],
) -> Result<Vec<ModRemoteInfo>> {
    let hashes: Vec<String> = mods.iter().map(|m| m.sha1.clone()).collect();
    let current = modrinth.versions_by_hashes(&hashes).await?;
    if current.is_empty() {
        return Ok(Vec::new());
    }
    let loaders = cfg.loader.modrinth_loaders(&cfg.mc_version);
    let latest = modrinth
        .latest_by_hashes(&hashes, &loaders, &cfg.mc_version)
        .await
        .unwrap_or_default();
    let mut ids: Vec<String> = current.values().map(|v| v.project_id.clone()).collect();
    ids.sort();
    ids.dedup();
    let projects: BTreeMap<String, _> = modrinth
        .projects(&ids)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|p| (p.id.clone(), p))
        .collect();

    let mut out = Vec::new();
    for (sha1, version) in &current {
        let project = projects.get(&version.project_id);
        let update = latest
            .get(sha1)
            .filter(|new| new.id != version.id && is_newer(new, version))
            .and_then(|new| {
                Some(ModUpdate {
                    version_id: new.id.clone(),
                    version_number: new.version_number.clone(),
                    file_name: new.primary_file()?.filename.clone(),
                })
            });
        out.push(ModRemoteInfo {
            sha1: sha1.clone(),
            source: Source::Modrinth,
            project_id: version.project_id.clone(),
            title: project.map(|p| p.title.clone()),
            icon_url: project.and_then(|p| p.icon_url.clone()),
            version_number: version.version_number.clone(),
            client_only: project.is_some_and(|p| p.server_side == "unsupported"),
            update,
        });
    }
    Ok(out)
}

async fn curseforge_updates(
    cf: &CurseForgeClient,
    server_dir: &Path,
    cfg: &ServerConfig,
    mods: &[ModInfo],
) -> Result<Vec<ModRemoteInfo>> {
    let registry = CfRegistry::load(server_dir).await;
    let installed: Vec<(&ModInfo, CfEntry)> = mods
        .iter()
        .filter_map(|m| Some((m, registry.get(&m.file_name)?)))
        .collect();
    if installed.is_empty() {
        return Ok(Vec::new());
    }
    let mut ids: Vec<u64> = installed.iter().map(|(_, e)| e.mod_id).collect();
    ids.sort_unstable();
    ids.dedup();
    let projects: HashMap<u64, CfMod> = cf
        .mods(&ids)
        .await?
        .into_iter()
        .map(|m| (m.id, m))
        .collect();
    let loaders = cfg.loader.modrinth_loaders(&cfg.mc_version);
    Ok(installed
        .into_iter()
        .map(|(m, entry)| {
            let project = projects.get(&entry.mod_id);
            let update = project
                .and_then(|p| p.update_for(entry.file_id, &cfg.mc_version, &loaders))
                .map(|i| ModUpdate {
                    version_id: i.file_id.to_string(),
                    version_number: i
                        .filename
                        .strip_suffix(".jar")
                        .unwrap_or(&i.filename)
                        .to_string(),
                    file_name: i.filename.clone(),
                });
            ModRemoteInfo {
                sha1: m.sha1.clone(),
                source: Source::Curseforge,
                project_id: entry.mod_id.to_string(),
                title: project.map(|p| p.name.clone()),
                icon_url: project.and_then(|p| p.logo.as_ref().map(|l| l.thumbnail_url.clone())),
                version_number: String::new(),
                client_only: project.is_some_and(CfMod::is_client_only),
                update,
            }
        })
        .collect())
}

/// Modrinth puede devolver una versión "más compatible" que no es más nueva;
/// solo se ofrece si se publicó después.
fn is_newer(candidate: &Version, current: &Version) -> bool {
    let parse = |s: &str| chrono::DateTime::parse_from_rfc3339(s).ok();
    match (
        parse(&candidate.date_published),
        parse(&current.date_published),
    ) {
        (Some(a), Some(b)) => a > b,
        _ => true,
    }
}

/// Sustituye un mod por otra versión de su fuente. Respeta si estaba
/// desactivado y actualiza los registros del modpack y de CurseForge.
pub async fn update_mod(
    sources: Sources<'_>,
    server_dir: &Path,
    cfg: &ServerConfig,
    file_name: &str,
    source: Source,
    version_id: &str,
) -> Result<String> {
    validate_mod_file_name(file_name)?;
    let mut registry = CfRegistry::load(server_dir).await;
    let (tmp, new_name, entry) = match source {
        Source::Modrinth => {
            let version = sources.modrinth.version(version_id).await?;
            check_compatible(&version, cfg)?;
            let (tmp, name) = fetch_verified(sources.http, server_dir, &version).await?;
            (tmp, name, None)
        }
        Source::Curseforge => {
            let cf = sources.curseforge.ok_or_else(curseforge_missing)?;
            let mod_id = registry
                .get(file_name)
                .ok_or_else(|| {
                    Error::NotFound(tr!(
                        "el proyecto de CurseForge de {file_name}",
                        "the CurseForge project of {file_name}"
                    ))
                })?
                .mod_id;
            let file_id = curseforge_id(version_id)?;
            let file = cf.file(mod_id, file_id).await?;
            let loaders = cfg.loader.modrinth_loaders(&cfg.mc_version);
            if !file.fits(&cfg.mc_version, &loaders) {
                return Err(not_compatible(&file.display_name, cfg));
            }
            let tmp = fetch_curseforge(cf, server_dir, &file).await?;
            (tmp, file.file_name, Some(CfEntry { mod_id, file_id }))
        }
    };
    let target_name = replace_mod(server_dir, file_name, &tmp, &new_name).await?;
    let had_entry = registry.mods.remove(base_name(file_name)).is_some();
    if let Some(entry) = entry {
        registry.mods.insert(new_name, entry);
    }
    if had_entry || entry.is_some() {
        registry.save(server_dir).await?;
    }
    Ok(target_name)
}

/// Pone un archivo ya verificado en lugar de `file_name` (que va a
/// `.micra/old-mods`), desactivado si lo estaba, y apunta el cambio en el
/// registro del modpack.
async fn replace_mod(
    server_dir: &Path,
    file_name: &str,
    tmp: &Path,
    new_name: &str,
) -> Result<String> {
    let mods_dir = server_dir.join(MODS_DIR);
    let target_name = if file_name.ends_with(DISABLED_SUFFIX) {
        format!("{new_name}{DISABLED_SUFFIX}")
    } else {
        new_name.to_string()
    };
    let old_path = mods_dir.join(file_name);
    if old_path.exists() {
        move_to_old(server_dir, &old_path).await?;
    }
    check_no_links(server_dir, &mods_dir.join(&target_name))?;
    tokio::fs::rename(tmp, mods_dir.join(&target_name)).await?;

    if let Some(mut manifest) = PackManifest::load(server_dir).await {
        let old_rel = format!("{MODS_DIR}/{}", base_name(file_name));
        if let Some(entry) = manifest.files.iter_mut().find(|f| **f == old_rel) {
            *entry = format!("{MODS_DIR}/{new_name}");
            manifest.save(server_dir).await?;
        }
    }
    Ok(target_name)
}

/// SHA-1 → nombre de archivo de los mods instalados (para la interfaz).
pub fn by_sha1(mods: &[ModInfo]) -> HashMap<&str, &str> {
    mods.iter()
        .map(|m| (m.sha1.as_str(), m.file_name.as_str()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn jar(path: &Path, entries: &[(&str, &str)]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (name, data) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn file_names() {
        assert!(validate_mod_file_name("sodium-0.6.jar").is_ok());
        assert!(validate_mod_file_name("sodium-0.6.jar.disabled").is_ok());
        for bad in [
            "../x.jar",
            "a/b.jar",
            "x.zip",
            ".jar",
            ".hidden.jar",
            "C:x.jar",
        ] {
            assert!(validate_mod_file_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn reads_metadata_of_each_loader() {
        let dir = tempfile::tempdir().unwrap();
        let fabric = dir.path().join("f.jar");
        jar(
            &fabric,
            &[(
                "fabric.mod.json",
                r#"{"schemaVersion":1,"id":"lithium","name":"Lithium","version":"0.14.3"}"#,
            )],
        );
        assert_eq!(
            read_metadata(&fabric),
            ModMetadata {
                id: Some("lithium".into()),
                name: Some("Lithium".into()),
                version: Some("0.14.3".into()),
                client_only: false,
            }
        );

        let forge = dir.path().join("g.jar");
        jar(
            &forge,
            &[
                (
                    "META-INF/mods.toml",
                    "modLoader=\"javafml\"\n[[mods]]\nmodId=\"create\" # comentario\nversion=\"${file.jarVersion}\"\ndisplayName=\"Create\"\n[[dependencies.create]]\nmodId=\"forge\"\n",
                ),
                (
                    "META-INF/MANIFEST.MF",
                    "Manifest-Version: 1.0\r\nImplementation-Version: 0.5.1.f\r\n",
                ),
            ],
        );
        assert_eq!(
            read_metadata(&forge),
            ModMetadata {
                id: Some("create".into()),
                name: Some("Create".into()),
                version: Some("0.5.1.f".into()),
                client_only: false,
            }
        );
        assert_eq!(
            read_metadata(&dir.path().join("nope.jar")),
            ModMetadata::default()
        );
    }

    #[test]
    fn client_only_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let fabric = dir.path().join("f.jar");
        jar(
            &fabric,
            &[(
                "fabric.mod.json",
                r#"{"id":"zoomify","name":"Zoomify","version":"2","environment":"client"}"#,
            )],
        );
        assert!(read_metadata(&fabric).client_only);
        let forge = dir.path().join("g.jar");
        jar(
            &forge,
            &[(
                "META-INF/mods.toml",
                "modLoader=\"javafml\"\nclientSideOnly=true\n[[mods]]\nmodId=\"oculus\"\nversion=\"1.6\"\n",
            )],
        );
        let meta = read_metadata(&forge);
        assert!(meta.client_only);
        assert_eq!(meta.id.as_deref(), Some("oculus"));
        // `clientSideOnly` dentro de una sección no cuenta.
        assert!(!parse_mods_toml("[[mods]]\nmodId=\"x\"\nclientSideOnly=true\n").client_only);
    }

    #[test]
    fn same_mod_is_not_installed_twice() {
        let server = tempfile::tempdir().unwrap();
        let downloads = server.path().join(DOWNLOADS_DIR);
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::create_dir_all(server.path().join(MODS_DIR)).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let mut ids = HashMap::from([("jei".to_string(), "jei-curseforge.jar".to_string())]);
            let tmp = downloads.join("jei-modrinth.jar");
            jar(
                &tmp,
                &[(
                    "META-INF/mods.toml",
                    "[[mods]]\nmodId=\"jei\"\ndisplayName=\"JEI\"\n",
                )],
            );
            // Pedido por la persona: se avisa.
            let err = place(server.path(), &tmp, "jei-modrinth.jar", &mut ids, true)
                .await
                .unwrap_err();
            assert!(err.to_string().contains("jei-curseforge.jar"));
            assert!(!tmp.exists());
            // Como dependencia: se salta sin más.
            jar(&tmp, &[("META-INF/mods.toml", "[[mods]]\nmodId=\"JEI\"\n")]);
            assert!(
                !place(server.path(), &tmp, "jei-modrinth.jar", &mut ids, false)
                    .await
                    .unwrap()
            );
            // Uno nuevo sí entra y queda apuntado.
            jar(&tmp, &[("fabric.mod.json", r#"{"id":"jade"}"#)]);
            assert!(place(server.path(), &tmp, "jade.jar", &mut ids, true)
                .await
                .unwrap());
            assert!(server.path().join(MODS_DIR).join("jade.jar").exists());
            assert_eq!(ids.get("jade").map(String::as_str), Some("jade.jar"));
        });
    }

    #[test]
    fn curseforge_registry_follows_the_file() {
        let server = tempfile::tempdir().unwrap();
        let mods = server.path().join(MODS_DIR);
        std::fs::create_dir_all(&mods).unwrap();
        jar(
            &mods.join("jei.jar"),
            &[("META-INF/mods.toml", "[[mods]]\nmodId=\"jei\"\n")],
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let mut registry = CfRegistry::default();
            let entry = CfEntry {
                mod_id: 238222,
                file_id: 5101366,
            };
            registry.mods.insert("jei.jar".into(), entry);
            registry.save(server.path()).await.unwrap();
            let off = set_enabled(server.path(), "jei.jar", false).await.unwrap();
            assert_eq!(CfRegistry::load(server.path()).await.get(&off), Some(entry));
            remove_mod(server.path(), &off).await.unwrap();
            assert!(CfRegistry::load(server.path()).await.mods.is_empty());
        });
    }

    #[test]
    fn list_toggle_and_remove() {
        let server = tempfile::tempdir().unwrap();
        let mods = server.path().join(MODS_DIR);
        std::fs::create_dir_all(&mods).unwrap();
        jar(
            &mods.join("a.jar"),
            &[(
                "fabric.mod.json",
                r#"{"id":"a","name":"Alpha","version":"1"}"#,
            )],
        );
        std::fs::write(mods.join("notes.txt"), "x").unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            PackManifest {
                name: "p".into(),
                version: "1".into(),
                files: vec!["mods/a.jar".into()],
            }
            .save(server.path())
            .await
            .unwrap();
            let list = list_mods(server.path()).await.unwrap();
            assert_eq!(list.len(), 1);
            assert!(list[0].enabled && list[0].from_pack);
            assert_eq!(list[0].name.as_deref(), Some("Alpha"));

            let off = set_enabled(server.path(), "a.jar", false).await.unwrap();
            assert_eq!(off, "a.jar.disabled");
            let list = list_mods(server.path()).await.unwrap();
            assert!(!list[0].enabled && list[0].from_pack);
            remove_mod(server.path(), &off).await.unwrap();
            assert!(list_mods(server.path()).await.unwrap().is_empty());
            assert!(server.path().join(OLD_MODS_DIR).join(&off).exists());
        });
    }
}
