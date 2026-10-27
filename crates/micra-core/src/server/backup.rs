//! Copias de seguridad del mundo (o del servidor entero) en ZIP y su
//! restauración con vuelta atrás si algo falla.
//!
//! - Las copias viven fuera de la carpeta del servidor (`AppPaths::backups_dir`).
//! - Con el servidor encendido se usa `save-off` / `save-all flush` /
//!   `save-on` para copiar un mundo coherente sin pararlo.
//! - Cada ZIP lleva `micra-backup.json` con qué cubre la copia, para que
//!   restaurar sepa exactamente qué sustituir (y qué borrar).

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

use crate::archive::safe_join;
use crate::fsutil::{ensure_space, walk_files};
use crate::server::config::ServerConfig;
use crate::server::process::{ServerHandle, ServerStatus};
use crate::server::properties::{level_name, read_properties_sync};
use crate::{tr, Error, Result};

pub const MANIFEST_ENTRY: &str = "micra-backup.json";
const FORMAT: u32 = 1;
/// Copias "de seguridad de la seguridad" (antes de restaurar o actualizar)
/// que se conservan de cada tipo.
const KEEP_SAFETY_COPIES: usize = 3;

/// Progreso: (bytes procesados, total).
pub type BackupProgress = Arc<dyn Fn(u64, u64) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackupKind {
    Manual,
    Auto,
    PreRestore,
    PreUpdate,
}

impl BackupKind {
    fn slug(self) -> &'static str {
        match self {
            BackupKind::Manual => "manual",
            BackupKind::Auto => "auto",
            BackupKind::PreRestore => "pre-restore",
            BackupKind::PreUpdate => "pre-update",
        }
    }
}

/// Qué cubre la copia.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackupScope {
    /// Mundo (las tres dimensiones), listas de jugadores, `server.properties`
    /// y configuración de mods.
    World,
    /// Todo el servidor salvo registros y cachés (antes de actualizar un modpack).
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupManifest {
    pub format: u32,
    pub server_id: String,
    pub server_name: String,
    pub mc_version: String,
    pub kind: BackupKind,
    pub scope: BackupScope,
    pub created_at: DateTime<Local>,
    /// Entradas de la carpeta del servidor que cubre la copia, existieran o
    /// no: al restaurar, las que no estén en la copia se borran.
    pub roots: Vec<String>,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub file_name: String,
    pub kind: BackupKind,
    pub scope: BackupScope,
    pub created_at: DateTime<Local>,
    /// Tamaño del ZIP.
    pub size: u64,
    pub mc_version: String,
}

/// Archivos de la raíz del servidor que van en una copia del mundo.
const WORLD_FILES: &[&str] = &[
    "server.properties",
    "whitelist.json",
    "ops.json",
    "banned-players.json",
    "banned-ips.json",
    // Configuración de mods (Forge/NeoForge/Fabric).
    "config",
    "defaultconfigs",
];

/// Entradas de la raíz que nunca van en una copia completa: registros,
/// cachés que se regeneran y los datos internos de Micra.
const FULL_EXCLUDED: &[&str] = &[
    "logs",
    "crash-reports",
    "debug",
    "cache",
    ".cache",
    ".fabric",
    "bluemap",
    ".micra",
];

/// Lo único de `.micra` que sí se guarda: qué archivos instaló el modpack.
const PACK_MANIFEST: &str = crate::server::pack::MANIFEST;

/// Formatos ya comprimidos: se guardan tal cual para no gastar CPU.
fn is_precompressed(rel: &str) -> bool {
    let lower = rel.to_ascii_lowercase();
    [
        ".mca", ".mcc", ".dat", ".dat_old", ".gz", ".zip", ".jar", ".png", ".ogg", ".mrpack",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

/// Archivos que no se copian nunca: el bloqueo del mundo (en Windows ni
/// siquiera se puede leer con el servidor encendido) y descargas a medias.
fn skip_file(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name == "session.lock" || name.ends_with(".part") || name.ends_with(".tmp")
}

/// Nombre del ZIP: `2026-10-22_15-04-05_auto.zip` (hora local).
pub fn backup_file_name(at: DateTime<Local>, kind: BackupKind) -> String {
    format!("{}_{}.zip", at.format("%Y-%m-%d_%H-%M-%S"), kind.slug())
}

/// Entradas que cubre una copia de este alcance, con `/` como separador.
pub fn roots_for(server_dir: &Path, scope: BackupScope) -> Vec<String> {
    match scope {
        BackupScope::World => {
            let level = level_name(&read_properties_sync(server_dir));
            let mut roots = vec![
                level.clone(),
                format!("{level}_nether"),
                format!("{level}_the_end"),
            ];
            roots.extend(WORLD_FILES.iter().map(|s| s.to_string()));
            roots.retain(|r| valid_root(r));
            roots
        }
        BackupScope::Full => {
            let mut roots: Vec<String> = std::fs::read_dir(server_dir)
                .map(|entries| {
                    entries
                        .flatten()
                        .filter(|e| e.file_type().is_ok_and(|t| !t.is_symlink()))
                        .filter_map(|e| e.file_name().into_string().ok())
                        .filter(|n| !FULL_EXCLUDED.contains(&n.as_str()) && !skip_file(n))
                        .collect()
                })
                .unwrap_or_default();
            roots.push(PACK_MANIFEST.to_string());
            roots.sort();
            roots
        }
    }
}

/// Una raíz es un único nombre de la carpeta del servidor (o el manifiesto
/// del pack). Se comprueba también al restaurar: el ZIP podría venir de fuera.
fn valid_root(root: &str) -> bool {
    if root == PACK_MANIFEST {
        return true;
    }
    !root.is_empty()
        && root != "."
        && root != ".."
        && root != ".micra"
        && !root.contains(['/', '\\', ':'])
}

/// Crea la copia (síncrono: llamar desde `spawn_blocking`).
pub fn create_backup_sync(
    server_dir: &Path,
    backups_dir: &Path,
    cfg: &ServerConfig,
    kind: BackupKind,
    scope: BackupScope,
    progress: Option<BackupProgress>,
) -> Result<BackupInfo> {
    let roots = roots_for(server_dir, scope);
    let mut files: Vec<(String, PathBuf, u64)> = Vec::new();
    for root in &roots {
        let path = server_dir.join(root);
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_file() {
            if !skip_file(root) {
                files.push((root.clone(), path, meta.len()));
            }
        } else if meta.is_dir() {
            for (file, size) in walk_files(&path)? {
                let rel = relative(server_dir, &file);
                if !skip_file(&rel) {
                    files.push((rel, file, size));
                }
            }
        }
    }
    let total: u64 = files.iter().map(|(_, _, size)| size).sum();
    std::fs::create_dir_all(backups_dir)?;
    ensure_space(backups_dir, total)?;

    let now = Local::now();
    let base = backup_file_name(now, kind);
    let stem = base.trim_end_matches(".zip").to_string();
    let mut name = base;
    // Dos copias en el mismo segundo: no pisar la anterior.
    let mut n = 2;
    while backups_dir.join(&name).exists() {
        name = format!("{stem}-{n}.zip");
        n += 1;
    }
    let dest = backups_dir.join(&name);
    let part = backups_dir.join(format!(".{name}.part"));
    let result = write_zip(
        &part, cfg, kind, scope, &roots, &files, total, now, progress,
    );
    match result {
        Ok(()) => std::fs::rename(&part, &dest)?,
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    }
    Ok(BackupInfo {
        size: std::fs::metadata(&dest)?.len(),
        file_name: name,
        kind,
        scope,
        created_at: now,
        mc_version: cfg.mc_version.clone(),
    })
}

#[allow(clippy::too_many_arguments)]
fn write_zip(
    part: &Path,
    cfg: &ServerConfig,
    kind: BackupKind,
    scope: BackupScope,
    roots: &[String],
    files: &[(String, PathBuf, u64)],
    total: u64,
    now: DateTime<Local>,
    progress: Option<BackupProgress>,
) -> Result<()> {
    let mut zip = zip::ZipWriter::new(BufWriter::new(File::create(part)?));
    let manifest = BackupManifest {
        format: FORMAT,
        server_id: cfg.id.clone(),
        server_name: cfg.name.clone(),
        mc_version: cfg.mc_version.clone(),
        kind,
        scope,
        created_at: now,
        roots: roots.to_vec(),
        files: files.len() as u64,
        bytes: total,
    };
    zip.start_file(MANIFEST_ENTRY, SimpleFileOptions::default())?;
    zip.write_all(&serde_json::to_vec_pretty(&manifest)?)?;

    let mut done = 0u64;
    let mut buf = vec![0u8; 256 * 1024];
    for (rel, path, size) in files {
        let mut src = match File::open(path) {
            Ok(f) => f,
            // Borrado entre el listado y la copia (p. ej. un log rotado).
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        let method = if is_precompressed(rel) {
            CompressionMethod::Stored
        } else {
            CompressionMethod::Deflated
        };
        let options = SimpleFileOptions::default()
            .compression_method(method)
            .large_file(*size >= u32::MAX as u64);
        zip.start_file(rel.as_str(), options)?;
        loop {
            let n = src.read(&mut buf)?;
            if n == 0 {
                break;
            }
            zip.write_all(&buf[..n])?;
            done += n as u64;
            if let Some(cb) = &progress {
                cb(done.min(total), total);
            }
        }
    }
    zip.finish()?.flush()?;
    Ok(())
}

fn relative(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

pub async fn create_backup(
    server_dir: &Path,
    backups_dir: &Path,
    cfg: &ServerConfig,
    kind: BackupKind,
    scope: BackupScope,
    progress: Option<BackupProgress>,
) -> Result<BackupInfo> {
    let server_dir = server_dir.to_owned();
    let backups_dir = backups_dir.to_owned();
    let cfg = cfg.clone();
    tokio::task::spawn_blocking(move || {
        create_backup_sync(&server_dir, &backups_dir, &cfg, kind, scope, progress)
    })
    .await
    .expect("la copia no debería entrar en pánico")
}

/// Copia del mundo con el servidor encendido: se desactiva el guardado
/// automático, se fuerza un guardado completo y se reactiva al terminar
/// (también si la copia falla).
pub async fn create_live(
    handle: &ServerHandle,
    server_dir: &Path,
    backups_dir: &Path,
    cfg: &ServerConfig,
    kind: BackupKind,
    progress: Option<BackupProgress>,
) -> Result<BackupInfo> {
    if handle.status() != ServerStatus::Running {
        return Err(Error::InvalidConfig(tr!(
            "espera a que el servidor termine de arrancar para hacer la copia",
            "wait until the server has finished starting to make the backup"
        )));
    }
    handle.send_command("save-off").await?;
    let result = async {
        let saved = handle
            .command_and_wait("save-all flush", is_saved_line, Duration::from_secs(120))
            .await?;
        if !saved {
            return Err(Error::Api(tr!(
                "el servidor no confirmó que guardó el mundo, así que la copia se canceló",
                "the server did not confirm it saved the world, so the backup was cancelled"
            )));
        }
        create_backup(
            server_dir,
            backups_dir,
            cfg,
            kind,
            BackupScope::World,
            progress,
        )
        .await
    }
    .await;
    if let Err(e) = handle.send_command("save-on").await {
        tracing::warn!("no se pudo reactivar el guardado automático: {e}");
    }
    result
}

/// Respuesta de `save-all` en las distintas versiones.
fn is_saved_line(msg: &str) -> bool {
    msg.starts_with("Saved the game") || msg.starts_with("Saved the world")
}

/// El nombre llega de la interfaz: solo un ZIP de la carpeta de copias.
pub fn validate_backup_name(name: &str) -> Result<()> {
    let ok = name.ends_with(".zip")
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':'])
        && name.len() <= 120;
    if ok {
        Ok(())
    } else {
        Err(Error::NotFound(name.to_string()))
    }
}

fn read_manifest(zip_path: &Path) -> Result<BackupManifest> {
    let mut zip = zip::ZipArchive::new(File::open(zip_path)?)?;
    let entry = zip.by_name(MANIFEST_ENTRY)?;
    let manifest: BackupManifest = serde_json::from_reader(entry)?;
    if manifest.format > FORMAT {
        return Err(Error::InvalidConfig(tr!(
            "esta copia se hizo con una versión más nueva de Micra",
            "this backup was made with a newer version of Micra"
        )));
    }
    Ok(manifest)
}

/// Copias de un servidor, de la más nueva a la más antigua.
pub async fn list_backups(backups_dir: &Path) -> Vec<BackupInfo> {
    let dir = backups_dir.to_owned();
    tokio::task::spawn_blocking(move || list_backups_sync(&dir))
        .await
        .unwrap_or_default()
}

fn list_backups_sync(dir: &Path) -> Vec<BackupInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<BackupInfo> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            validate_backup_name(&name).ok()?;
            let manifest = read_manifest(&e.path()).ok()?;
            Some(BackupInfo {
                size: e.metadata().ok()?.len(),
                file_name: name,
                kind: manifest.kind,
                scope: manifest.scope,
                created_at: manifest.created_at,
                mc_version: manifest.mc_version,
            })
        })
        .collect();
    out.sort_by_key(|b| std::cmp::Reverse(b.created_at));
    out
}

pub async fn delete_backup(backups_dir: &Path, name: &str) -> Result<()> {
    validate_backup_name(name)?;
    tokio::fs::remove_file(backups_dir.join(name)).await?;
    Ok(())
}

/// Borra las copias automáticas que sobran (se guardan `keep_auto`) y deja
/// solo las últimas copias de seguridad previas a restaurar o actualizar.
/// Las copias manuales no se tocan nunca.
pub async fn prune(backups_dir: &Path, keep_auto: u32) -> Result<usize> {
    let list = list_backups(backups_dir).await;
    let mut removed = 0;
    for (kind, keep) in [
        (BackupKind::Auto, keep_auto.max(1) as usize),
        (BackupKind::PreRestore, KEEP_SAFETY_COPIES),
        (BackupKind::PreUpdate, KEEP_SAFETY_COPIES),
    ] {
        for old in list.iter().filter(|b| b.kind == kind).skip(keep) {
            tokio::fs::remove_file(backups_dir.join(&old.file_name)).await?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Restaura una copia con el servidor apagado. Antes guarda el estado
/// actual como copia "previa a restaurar", por si la persona se arrepiente.
pub async fn restore_backup(
    server_dir: &Path,
    backups_dir: &Path,
    cfg: &ServerConfig,
    name: &str,
    progress: Option<BackupProgress>,
) -> Result<()> {
    validate_backup_name(name)?;
    let zip_path = backups_dir.join(name);
    let manifest = {
        let path = zip_path.clone();
        tokio::task::spawn_blocking(move || read_manifest(&path))
            .await
            .expect("la lectura no debería entrar en pánico")?
    };
    if manifest.server_id != cfg.id {
        return Err(Error::InvalidConfig(tr!(
            "esta copia es de otro servidor",
            "this backup belongs to another server"
        )));
    }
    create_backup(
        server_dir,
        backups_dir,
        cfg,
        BackupKind::PreRestore,
        manifest.scope,
        None,
    )
    .await?;
    let server_dir = server_dir.to_owned();
    tokio::task::spawn_blocking(move || restore_sync(&server_dir, &zip_path, &manifest, progress))
        .await
        .expect("la restauración no debería entrar en pánico")?;
    prune(backups_dir, cfg.backups.keep).await?;
    Ok(())
}

fn restore_sync(
    server_dir: &Path,
    zip_path: &Path,
    manifest: &BackupManifest,
    progress: Option<BackupProgress>,
) -> Result<()> {
    // Carpetas de trabajo de verdad y vacías: si un mod dejó enlaces en su
    // lugar, se quitan los enlaces (no lo que apuntan).
    let micra = crate::fsutil::internal_dir(server_dir, ".micra")?;
    let staging = micra.join("restore-tmp");
    let old = micra.join("restore-old");
    for dir in [&staging, &old] {
        crate::fsutil::remove_entry(dir)?;
        std::fs::create_dir_all(dir)?;
    }
    ensure_space(server_dir, manifest.bytes)?;

    // 1. Extraer en una carpeta aparte (si falla, el servidor no se toca).
    extract_backup(zip_path, &staging, manifest.bytes, progress)?;

    // 2. Qué se sustituye: lo que cubría la copia y, en una copia completa,
    //    también lo que se añadió después (p. ej. mods nuevos).
    let mut roots: Vec<String> = manifest.roots.clone();
    if manifest.scope == BackupScope::Full {
        roots.extend(roots_for(server_dir, BackupScope::Full));
    }
    roots.sort();
    roots.dedup();
    if let Some(bad) = roots.iter().find(|r| !valid_root(r)) {
        return Err(Error::UnsafePath(bad.clone()));
    }

    // 3. Apartar lo actual y colocar lo restaurado; ante cualquier fallo,
    //    deshacer en orden inverso.
    let mut moved_out: Vec<&String> = Vec::new();
    let mut moved_in: Vec<&String> = Vec::new();
    let outcome = (|| -> Result<()> {
        for root in &roots {
            let current = server_dir.join(root);
            crate::fsutil::check_no_links(server_dir, &current)?;
            if std::fs::symlink_metadata(&current).is_ok() {
                let target = old.join(root);
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::rename(&current, &target)?;
                moved_out.push(root);
            }
        }
        for root in &roots {
            let restored = staging.join(root);
            if std::fs::symlink_metadata(&restored).is_ok() {
                let target = server_dir.join(root);
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::rename(&restored, &target)?;
                moved_in.push(root);
            }
        }
        Ok(())
    })();

    if let Err(e) = outcome {
        for root in moved_in.iter().rev() {
            let _ = remove_any(&server_dir.join(root));
        }
        for root in moved_out.iter().rev() {
            let _ = std::fs::rename(old.join(root), server_dir.join(root));
        }
        return Err(e);
    }
    let _ = std::fs::remove_dir_all(&old);
    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
}

fn extract_backup(
    zip_path: &Path,
    dest: &Path,
    total: u64,
    progress: Option<BackupProgress>,
) -> Result<()> {
    let mut zip = zip::ZipArchive::new(File::open(zip_path)?)?;
    let mut done = 0u64;
    let mut buf = vec![0u8; 256 * 1024];
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().replace('\\', "/");
        if name == MANIFEST_ENTRY || entry.is_dir() {
            continue;
        }
        let out = safe_join(dest, &name)?;
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = BufWriter::new(File::create(&out)?);
        loop {
            let n = entry.read(&mut buf)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            done += n as u64;
            if let Some(cb) = &progress {
                cb(done.min(total), total);
            }
        }
        file.flush()?;
    }
    Ok(())
}

fn remove_any(path: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(path)?.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::config::*;

    fn cfg(id: &str) -> ServerConfig {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": "Test", "mcVersion": "1.21.1", "loader": {"kind": "vanilla"},
            "javaMajor": 21, "memory": {"minMb": 1024, "maxMb": 2048}, "port": 25565,
            "eulaAccepted": true, "launch": {"kind": "jar", "path": "server.jar"}
        }))
        .unwrap()
    }

    fn write(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn names_and_validation() {
        let at = DateTime::parse_from_rfc3339("2026-10-22T15:04:05+02:00")
            .unwrap()
            .with_timezone(&Local);
        let name = backup_file_name(at, BackupKind::PreUpdate);
        assert!(name.ends_with("_pre-update.zip"));
        assert!(validate_backup_name(&name).is_ok());
        for bad in ["../x.zip", "a/b.zip", ".hidden.zip", "x.txt", "C:x.zip"] {
            assert!(validate_backup_name(bad).is_err(), "{bad}");
        }
        assert!(valid_root("world"));
        assert!(valid_root(".micra/pack.json"));
        assert!(!valid_root("../etc"));
        assert!(!valid_root(".micra"));
    }

    #[test]
    fn world_backup_and_restore_roundtrip() {
        let server = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let s = server.path();
        write(&s.join("server.properties"), "level-name=mundo\n");
        write(&s.join("mundo/level.dat"), "v1");
        write(&s.join("mundo/region/r.0.0.mca"), "region v1");
        write(&s.join("mundo/session.lock"), "lock");
        write(&s.join("whitelist.json"), "[]");
        write(&s.join("logs/latest.log"), "log");
        write(&s.join("mods/a.jar"), "mod");
        let cfg = cfg("test");

        let info = create_backup_sync(
            s,
            backups.path(),
            &cfg,
            BackupKind::Manual,
            BackupScope::World,
            None,
        )
        .unwrap();
        assert_eq!(info.scope, BackupScope::World);
        let manifest = read_manifest(&backups.path().join(&info.file_name)).unwrap();
        assert!(manifest.roots.contains(&"mundo_nether".to_string()));
        assert_eq!(manifest.files, 4); // level.dat, r.0.0.mca, server.properties, whitelist.json

        // Cambios posteriores: el mundo avanza y aparece el Nether.
        write(&s.join("mundo/level.dat"), "v2");
        write(&s.join("mundo/region/r.1.0.mca"), "nueva");
        write(&s.join("mundo_nether/level.dat"), "nether");
        write(&s.join("mods/b.jar"), "mod nuevo");

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(restore_backup(
            s,
            backups.path(),
            &cfg,
            &info.file_name,
            None,
        ))
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(s.join("mundo/level.dat")).unwrap(),
            "v1"
        );
        assert!(!s.join("mundo/region/r.1.0.mca").exists());
        assert!(!s.join("mundo_nether").exists());
        // Fuera del alcance de una copia del mundo: no se toca.
        assert!(s.join("mods/b.jar").exists());
        assert!(s.join("logs/latest.log").exists());
        assert!(!s.join(".micra/restore-tmp").exists());

        // Se guardó el estado anterior por si acaso.
        let list = rt.block_on(list_backups(backups.path()));
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|b| b.kind == BackupKind::PreRestore));
    }

    #[test]
    fn full_restore_removes_added_files() {
        let server = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let s = server.path();
        write(&s.join("mods/a.jar"), "a");
        write(&s.join("micra.json"), "{}");
        write(&s.join(".micra/pack.json"), "v1");
        write(&s.join(".micra/tmp/x"), "tmp");
        write(&s.join("logs/latest.log"), "log");
        let cfg = cfg("full");
        let info = create_backup_sync(
            s,
            backups.path(),
            &cfg,
            BackupKind::PreUpdate,
            BackupScope::Full,
            None,
        )
        .unwrap();

        write(&s.join("mods/b.jar"), "b");
        write(&s.join(".micra/pack.json"), "v2");
        write(&s.join("kubejs/script.js"), "nuevo");

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(restore_backup(
            s,
            backups.path(),
            &cfg,
            &info.file_name,
            None,
        ))
        .unwrap();
        assert!(s.join("mods/a.jar").exists());
        assert!(!s.join("mods/b.jar").exists());
        assert!(!s.join("kubejs").exists());
        assert_eq!(
            std::fs::read_to_string(s.join(".micra/pack.json")).unwrap(),
            "v1"
        );
        // Lo excluido sigue en su sitio.
        assert!(s.join("logs/latest.log").exists());
        assert!(s.join(".micra/tmp/x").exists());
    }

    #[test]
    fn rejects_backups_of_other_servers() {
        let server = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        write(&server.path().join("world/level.dat"), "x");
        let info = create_backup_sync(
            server.path(),
            backups.path(),
            &cfg("a"),
            BackupKind::Manual,
            BackupScope::World,
            None,
        )
        .unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert!(rt
            .block_on(restore_backup(
                server.path(),
                backups.path(),
                &cfg("b"),
                &info.file_name,
                None
            ))
            .is_err());
    }

    #[test]
    fn prunes_old_automatic_copies() {
        let server = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        write(&server.path().join("world/level.dat"), "x");
        let cfg = cfg("p");
        for _ in 0..4 {
            create_backup_sync(
                server.path(),
                backups.path(),
                &cfg,
                BackupKind::Auto,
                BackupScope::World,
                None,
            )
            .unwrap();
        }
        create_backup_sync(
            server.path(),
            backups.path(),
            &cfg,
            BackupKind::Manual,
            BackupScope::World,
            None,
        )
        .unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert_eq!(rt.block_on(prune(backups.path(), 2)).unwrap(), 2);
        let list = rt.block_on(list_backups(backups.path()));
        assert_eq!(
            list.iter().filter(|b| b.kind == BackupKind::Auto).count(),
            2
        );
        assert_eq!(
            list.iter().filter(|b| b.kind == BackupKind::Manual).count(),
            1
        );
    }

    #[test]
    fn save_confirmation_lines() {
        assert!(is_saved_line("Saved the game"));
        assert!(is_saved_line("Saved the world"));
        assert!(!is_saved_line("Saving the game (this may take a moment!)"));
    }
}
