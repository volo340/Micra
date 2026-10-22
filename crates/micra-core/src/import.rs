//! Importar un servidor que ya existe en otra carpeta (creado a mano, con
//! otro programa o copiado de otro equipo).
//!
//! La carpeta original no se toca: se copia a la carpeta de servidores de
//! Micra y se le añade un `micra.json`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::fsutil::{copy_dir, dir_size, ensure_space};
use crate::java::JavaManager;
use crate::paths::AppPaths;
use crate::providers::loaders;
use crate::provision::{check_port_free_among, java_major_for, unique_id, used_ports};
use crate::server::config::{
    validate_resources, BackupPolicy, CpuConfig, LaunchTarget, Loader, MapConfig, MemoryConfig,
    NetworkPolicy, SchedulePolicy, SecurityPolicy, ServerConfig,
};
use crate::server::detect::{
    bundled_installer, detect_launch_target, detect_loader, detect_mc_version,
};
use crate::server::properties::{flag, level_name, read_properties_sync, set_properties};
use crate::server::{eula, ports};
use crate::{hardware, tr, Error, Result};

/// Lo que Micra entiende de una carpeta antes de importarla.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportAnalysis {
    pub path: String,
    pub suggested_name: String,
    pub mc_version: Option<String>,
    pub loader: Loader,
    /// Cómo se arrancaría; `None` si hace falta ejecutar un instalador.
    pub launch: Option<LaunchTarget>,
    pub has_installer: bool,
    pub size: u64,
    pub has_world: bool,
    pub port: Option<u16>,
    pub online_mode: bool,
    pub whitelist: bool,
    pub mod_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub path: String,
    pub name: String,
    /// Obligatoria si el análisis no pudo averiguarla.
    pub mc_version: Option<String>,
    pub memory: MemoryConfig,
    #[serde(default)]
    pub cpu: CpuConfig,
    pub port: u16,
    pub eula_accepted: bool,
}

/// Progreso de la copia: (bytes copiados, total).
pub type ImportProgress = Arc<dyn Fn(u64, u64) + Send + Sync>;

/// Carpetas que no merece la pena copiar.
const SKIP_DIRS: &[&str] = &["logs", "crash-reports", "debug", "cache", ".cache"];

fn validate_source(paths: &AppPaths, path: &Path) -> Result<PathBuf> {
    let dir = path
        .canonicalize()
        .map_err(|_| Error::NotFound(path.display().to_string()))?;
    if !dir.is_dir() {
        return Err(Error::NotFound(path.display().to_string()));
    }
    // Ni la carpeta de datos de Micra ni algo que la contenga (copia infinita).
    if let Ok(root) = paths.root().canonicalize() {
        if dir.starts_with(&root) || root.starts_with(&dir) {
            return Err(Error::InvalidConfig(tr!(
                "esa carpeta ya es de Micra o contiene sus datos",
                "that folder already belongs to Micra or contains its data"
            )));
        }
    }
    let looks_like_server = [
        "server.properties",
        "eula.txt",
        "libraries",
        "mods",
        "world",
    ]
    .iter()
    .any(|f| dir.join(f).exists())
        || std::fs::read_dir(&dir).is_ok_and(|mut entries| {
            entries.any(|e| {
                e.is_ok_and(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .to_ascii_lowercase()
                        .ends_with(".jar")
                })
            })
        });
    if !looks_like_server {
        return Err(Error::InvalidConfig(tr!(
            "esa carpeta no parece un servidor de Minecraft (no hay server.properties, jars ni mundo)",
            "that folder does not look like a Minecraft server (no server.properties, jars or world)"
        )));
    }
    Ok(dir)
}

pub async fn analyze(paths: &AppPaths, path: &Path) -> Result<ImportAnalysis> {
    let dir = validate_source(paths, path)?;
    tokio::task::spawn_blocking(move || {
        let props = read_properties_sync(&dir);
        let level = level_name(&props);
        let launch = detect_launch_target(&dir)?;
        let mod_count = std::fs::read_dir(dir.join("mods"))
            .map(|e| {
                e.flatten()
                    .filter(|f| f.file_name().to_string_lossy().ends_with(".jar"))
                    .count()
            })
            .unwrap_or(0);
        Ok(ImportAnalysis {
            suggested_name: props
                .get("motd")
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty() && m != "A Minecraft Server" && m.chars().count() <= 40)
                .or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "Servidor".into()),
            mc_version: detect_mc_version(&dir),
            loader: detect_loader(&dir),
            has_installer: launch.is_none() && bundled_installer(&dir).is_some(),
            launch,
            size: dir_size(&dir),
            has_world: dir.join(&level).join("level.dat").exists(),
            port: props.get("server-port").and_then(|p| p.trim().parse().ok()),
            online_mode: flag(&props, "online-mode", true),
            whitelist: flag(&props, "white-list", false),
            mod_count,
            path: dir.display().to_string(),
        })
    })
    .await
    .expect("el análisis no debería entrar en pánico")
}

/// Copia la carpeta y la registra como servidor de Micra.
pub async fn import(
    paths: &AppPaths,
    http: &reqwest::Client,
    java: &JavaManager,
    req: ImportRequest,
    progress: Option<ImportProgress>,
) -> Result<ServerConfig> {
    if !req.eula_accepted {
        return Err(Error::InvalidConfig(tr!(
            "debes aceptar el EULA de Minecraft (https://aka.ms/MinecraftEULA) para continuar",
            "you must accept the Minecraft EULA (https://aka.ms/MinecraftEULA) to continue"
        )));
    }
    let source = validate_source(paths, Path::new(&req.path))?;
    let hw = hardware::hardware_info();
    validate_resources(
        &req.memory,
        &req.cpu,
        req.port,
        hw.total_memory_mb,
        hw.logical_cpus,
    )?;
    let used = used_ports(paths, None);
    check_port_free_among(req.port, &used)?;
    let mc_version = req
        .mc_version
        .clone()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| detect_mc_version(&source))
        .ok_or_else(|| {
            Error::InvalidConfig(tr!(
                "no se pudo averiguar la versión de Minecraft: indícala a mano",
                "the Minecraft version could not be detected: enter it by hand"
            ))
        })?;

    let id = unique_id(paths, &req.name);
    let dest = paths.server_dir(&id);
    let size = dir_size(&source);
    tokio::fs::create_dir_all(paths.servers()).await?;
    ensure_space(&paths.servers(), size)?;

    let copy = {
        let source = source.clone();
        let dest = dest.clone();
        tokio::task::spawn_blocking(move || {
            let skip = |rel: &Path| {
                let first = rel
                    .components()
                    .next()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .unwrap_or_default();
                let name = rel
                    .file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default();
                SKIP_DIRS.contains(&first.as_str())
                    || first == crate::server::config::CONFIG_FILE
                    || first == ".micra"
                    || name == "session.lock"
            };
            let report = |done: u64, total: u64| {
                if let Some(cb) = &progress {
                    cb(done, total);
                }
            };
            copy_dir(&source, &dest, &skip, &report)
        })
        .await
        .expect("la copia no debería entrar en pánico")
    };
    let result = async {
        copy?;
        let java_major = java_major_for(http, &mc_version).await;
        let launch = match detect_launch_target(&dest)? {
            Some(launch) => launch,
            None => {
                let installer = bundled_installer(&dest).ok_or_else(|| {
                    Error::InvalidConfig(tr!(
                        "no se encontró cómo arrancar el servidor (ni jar ni instalador)",
                        "no way to start the server was found (no jar or installer)"
                    ))
                })?;
                let runtime = java.ensure(java_major, None).await?;
                let launch = loaders::run_installer(&installer, &dest, &runtime).await?;
                let _ = tokio::fs::remove_file(&installer).await;
                launch
            }
        };
        let taken: Vec<u16> = used.iter().map(|(p, _)| *p).chain([req.port]).collect();
        let config = ServerConfig {
            id: id.clone(),
            name: req.name.trim().to_string(),
            loader: detect_loader(&dest),
            mc_version,
            java_major,
            memory: req.memory,
            cpu: req.cpu,
            port: req.port,
            eula_accepted: true,
            launch,
            use_aikar_flags: true,
            extra_jvm_args: Vec::new(),
            backups: BackupPolicy::default(),
            schedule: SchedulePolicy::default(),
            network: NetworkPolicy::default(),
            security: SecurityPolicy::default(),
            map: MapConfig {
                port: ports::suggest_port(MapConfig::default().port, &taken),
                ..MapConfig::default()
            },
            source: None,
            pending_owner: None,
        };
        eula::write_eula(&dest, true).await?;
        set_properties(&dest, &[("server-port", config.port.to_string())]).await?;
        config.save(&dest).await?;
        Ok(config)
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_dir_all(&dest).await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyzes_and_rejects_folders() {
        let data = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(data.path());
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let empty = tempfile::tempdir().unwrap();
        assert!(rt.block_on(analyze(&paths, empty.path())).is_err());
        assert!(rt.block_on(analyze(&paths, data.path())).is_err());

        let server = tempfile::tempdir().unwrap();
        let s = server.path();
        std::fs::write(
            s.join("server.properties"),
            "motd=Mi mundo\nserver-port=25570\nonline-mode=false\nlevel-name=mapa\n",
        )
        .unwrap();
        std::fs::create_dir_all(s.join("mapa")).unwrap();
        std::fs::write(s.join("mapa/level.dat"), "x").unwrap();
        std::fs::create_dir_all(s.join("libraries/net/fabricmc/fabric-loader/0.16.9")).unwrap();
        std::fs::create_dir_all(s.join("libraries/net/fabricmc/intermediary/1.21.1")).unwrap();
        std::fs::write(s.join("fabric-server-launch.jar"), "x").unwrap();
        let a = rt.block_on(analyze(&paths, s)).unwrap();
        assert_eq!(a.suggested_name, "Mi mundo");
        assert_eq!(a.mc_version.as_deref(), Some("1.21.1"));
        assert!(matches!(a.loader, Loader::Fabric { .. }));
        assert!(a.has_world);
        assert_eq!(a.port, Some(25570));
        assert!(!a.online_mode);
        assert!(a.launch.is_some());
    }
}
