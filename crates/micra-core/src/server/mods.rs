//! Gestión de los mods de un servidor: listar, activar/desactivar, quitar,
//! instalar desde Modrinth (con sus dependencias) y actualizar.
//!
//! Todo lo que se descarga se verifica con el SHA-512 de Modrinth y pasa por
//! el escáner de malware antes de llegar a `mods/`. Las versiones que se
//! sustituyen o quitan se guardan en `.micra/old-mods/` por si hay que volver.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::download::{download_file, ExpectedHash, HashAlgo};
use crate::providers::modrinth::{ModrinthClient, Version};
use crate::providers::mrpack::is_allowed_url;
use crate::server::config::ServerConfig;
use crate::server::pack::PackManifest;
use crate::server::scan::{self, Severity};
use crate::{tr, Error, Result};

pub const MODS_DIR: &str = "mods";
const DISABLED_SUFFIX: &str = ".disabled";
const OLD_MODS_DIR: &str = ".micra/old-mods";
const DOWNLOADS_DIR: &str = ".micra/downloads";
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
}

/// Lo que sabe Modrinth de un mod instalado.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModRemoteInfo {
    pub sha1: String,
    pub project_id: String,
    pub title: Option<String>,
    pub icon_url: Option<String>,
    pub version_number: String,
    pub update: Option<ModUpdate>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModUpdate {
    pub version_id: String,
    pub version_number: String,
    pub file_name: String,
}

/// Nombre de archivo de un mod: sin rutas y terminado en `.jar` (o
/// `.jar.disabled`).
pub fn validate_mod_file_name(name: &str) -> Result<()> {
    let base = name.strip_suffix(DISABLED_SUFFIX).unwrap_or(name);
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
            let base = name.strip_suffix(DISABLED_SUFFIX).unwrap_or(&name);
            out.push(ModInfo {
                from_pack: pack_files.contains(&format!("{MODS_DIR}/{base}")),
                sha1: scan::sha1_file(&path)?,
                size: entry.metadata()?.len(),
                enabled,
                mod_id: meta.id,
                name: meta.name,
                version: meta.version,
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
        }
    } else if let Some(json) = read("quilt.mod.json") {
        let v: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
        let q = &v["quilt_loader"];
        ModMetadata {
            id: str_field(&q["id"]),
            name: str_field(&q["metadata"]["name"]),
            version: str_field(&q["version"]),
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

/// Lo justo de TOML para el primer `[[mods]]` de `mods.toml`.
fn parse_mods_toml(toml: &str) -> ModMetadata {
    let mut meta = ModMetadata::default();
    let mut in_mods = false;
    for line in toml.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            if in_mods {
                break;
            }
            in_mods = line == "[[mods]]";
            continue;
        }
        if !in_mods {
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
        match key.trim() {
            "modId" => meta.id = Some(value),
            "version" => meta.version = Some(value),
            "displayName" => meta.name = Some(value),
            _ => {}
        }
    }
    meta
}

/// Activa o desactiva un mod renombrándolo (`.jar` ↔ `.jar.disabled`).
pub async fn set_enabled(server_dir: &Path, file_name: &str, enabled: bool) -> Result<String> {
    validate_mod_file_name(file_name)?;
    let base = file_name
        .strip_suffix(DISABLED_SUFFIX)
        .unwrap_or(file_name)
        .to_string();
    let target = if enabled {
        base.clone()
    } else {
        format!("{base}{DISABLED_SUFFIX}")
    };
    if target != file_name {
        let mods = server_dir.join(MODS_DIR);
        tokio::fs::rename(mods.join(file_name), mods.join(&target)).await?;
    }
    Ok(target)
}

/// Quita un mod (se guarda una copia en `.micra/old-mods`).
pub async fn remove_mod(server_dir: &Path, file_name: &str) -> Result<()> {
    validate_mod_file_name(file_name)?;
    move_to_old(server_dir, &server_dir.join(MODS_DIR).join(file_name)).await
}

async fn move_to_old(server_dir: &Path, path: &Path) -> Result<()> {
    let old = server_dir.join(OLD_MODS_DIR);
    tokio::fs::create_dir_all(&old).await?;
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
    let tmp = server_dir.join(DOWNLOADS_DIR).join(&file.filename);
    download_file(
        http,
        &file.url,
        &tmp,
        Some(&ExpectedHash::new(HashAlgo::Sha512, sha512)),
        None,
    )
    .await?;
    let path = tmp.clone();
    let findings = tokio::task::spawn_blocking(move || scan::scan_jar(&path))
        .await
        .expect("el análisis no debería entrar en pánico")?;
    if findings.iter().any(|f| f.severity == Severity::Critical) {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(Error::Malware(file.filename.clone()));
    }
    Ok((tmp, file.filename.clone()))
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
        let name = &version.name;
        let mc = &cfg.mc_version;
        Err(Error::InvalidConfig(tr!(
            "«{name}» no es compatible con este servidor (Minecraft {mc})",
            "\"{name}\" is not compatible with this server (Minecraft {mc})"
        )))
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
    if !cfg.loader.supports_mods() {
        return Err(Error::InvalidConfig(tr!(
            "un servidor vanilla no puede cargar mods",
            "a vanilla server cannot load mods"
        )));
    }
    let installed = list_mods(server_dir).await?;
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
    let mods_dir = server_dir.join(MODS_DIR);
    tokio::fs::create_dir_all(&mods_dir).await?;
    while let Some(version) = queue.pop_front() {
        if out.len() >= MAX_INSTALLS {
            break;
        }
        let (tmp, name) = fetch_verified(http, server_dir, &version).await?;
        tokio::fs::rename(&tmp, mods_dir.join(&name)).await?;
        present.insert(version.project_id.clone());
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

/// Busca en Modrinth los mods instalados (por su SHA-1) y si tienen una
/// versión más nueva para este servidor.
pub async fn check_updates(
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
            project_id: version.project_id.clone(),
            title: project.map(|p| p.title.clone()),
            icon_url: project.and_then(|p| p.icon_url.clone()),
            version_number: version.version_number.clone(),
            update,
        });
    }
    Ok(out)
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

/// Sustituye un mod por otra versión de Modrinth. Respeta si estaba
/// desactivado y actualiza el registro del modpack.
pub async fn update_mod(
    http: &reqwest::Client,
    modrinth: &ModrinthClient,
    server_dir: &Path,
    cfg: &ServerConfig,
    file_name: &str,
    version_id: &str,
) -> Result<String> {
    validate_mod_file_name(file_name)?;
    let version = modrinth.version(version_id).await?;
    check_compatible(&version, cfg)?;
    let (tmp, new_name) = fetch_verified(http, server_dir, &version).await?;
    let mods_dir = server_dir.join(MODS_DIR);
    let was_disabled = file_name.ends_with(DISABLED_SUFFIX);
    let target_name = if was_disabled {
        format!("{new_name}{DISABLED_SUFFIX}")
    } else {
        new_name.clone()
    };
    let old_path = mods_dir.join(file_name);
    if old_path.exists() {
        move_to_old(server_dir, &old_path).await?;
    }
    tokio::fs::rename(&tmp, mods_dir.join(&target_name)).await?;

    if let Some(mut manifest) = PackManifest::load(server_dir).await {
        let old_base = file_name.strip_suffix(DISABLED_SUFFIX).unwrap_or(file_name);
        let old_rel = format!("{MODS_DIR}/{old_base}");
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
                version: Some("0.14.3".into())
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
                version: Some("0.5.1.f".into())
            }
        );
        assert_eq!(
            read_metadata(&dir.path().join("nope.jar")),
            ModMetadata::default()
        );
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
