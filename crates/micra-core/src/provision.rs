//! Orquestación del asistente de creación: de "lo que eligió el usuario" a un
//! directorio de servidor listo para arrancar.
//!
//! 1. Obtener el contenido (vanilla, Fabric, Forge, NeoForge, `.mrpack` de
//!    Modrinth o server pack de CurseForge).
//! 2. Averiguar la versión de Minecraft y el loader.
//! 3. Elegir y asegurar el runtime de Java correcto.
//! 4. Instalar el loader si el pack no lo trae.
//! 5. EULA, `server.properties` seguro por defecto y `micra.json`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::download::{download_file, ExpectedHash, HashAlgo};
use crate::java::{required_java_for, JavaManager};
use crate::paths::AppPaths;
use crate::providers::curseforge::CurseForgeClient;
use crate::providers::modrinth::ModrinthClient;
use crate::providers::{loaders, mojang, mrpack};
use crate::server::config::{
    slugify, validate_resources, BackupPolicy, ContentSource, CpuConfig, LaunchTarget, Loader,
    MapConfig, MemoryConfig, NetworkPolicy, SchedulePolicy, SecurityPolicy, ServerConfig,
    WorldTweaks,
};
use crate::server::detect::{
    bundled_installer, detect_launch_target, detect_loader, detect_mc_version, pack_declared_loader,
};
use crate::server::pack::PackManifest;
use crate::server::players::{self, PlayerAction};
use crate::server::{eula, ports, properties};
use crate::{archive, hardware, tr, Error, Result};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ServerSource {
    #[serde(rename_all = "camelCase")]
    Vanilla { mc_version: String },
    /// Sin `loader_version` se usa la última versión estable.
    #[serde(rename_all = "camelCase")]
    Fabric {
        mc_version: String,
        loader_version: Option<String>,
    },
    /// Sin `version` se usa la recomendada (o la última) para esa versión.
    #[serde(rename_all = "camelCase")]
    Forge {
        mc_version: String,
        version: Option<String>,
    },
    /// Sin `version` se usa la última estable (o la última beta).
    #[serde(rename_all = "camelCase")]
    NeoForge {
        mc_version: String,
        version: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Modrinth { version_id: String },
    #[serde(rename_all = "camelCase")]
    CurseForge { mod_id: u64, file_id: u64 },
}

/// Lo que envía el asistente de la interfaz.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewServerRequest {
    pub name: String,
    pub source: ServerSource,
    pub memory: MemoryConfig,
    #[serde(default)]
    pub cpu: CpuConfig,
    #[serde(default = "default_port")]
    pub port: u16,
    pub eula_accepted: bool,
    /// Cuenta de Minecraft de quien crea el servidor: entra en la lista
    /// blanca y como operador, porque la lista blanca viene activada.
    #[serde(default)]
    pub owner: Option<String>,
}

fn default_port() -> u16 {
    25565
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "step", rename_all = "camelCase")]
pub enum ProvisionStep {
    FetchingContent,
    DownloadingFiles { done: usize, total: usize },
    InstallingJava { major: u32 },
    DownloadingJava { downloaded: u64, total: Option<u64> },
    InstallingLoader,
    Finalizing,
}

/// Valores de `server.properties` con los que nace todo servidor: solo
/// entran cuentas reales (online-mode) que estén en la lista blanca, y RCON y
/// query quedan apagados (Micra usa la consola directamente).
pub const SECURE_DEFAULTS: &[(&str, &str)] = &[
    ("online-mode", "true"),
    ("white-list", "true"),
    ("enforce-whitelist", "true"),
    ("enable-rcon", "false"),
    ("enable-query", "false"),
];

pub struct Provisioner<'a> {
    pub http: &'a reqwest::Client,
    pub paths: &'a AppPaths,
    pub java: &'a JavaManager,
    pub modrinth: &'a ModrinthClient,
    pub curseforge: Option<&'a CurseForgeClient>,
}

/// Lo que dejó el paso de contenido.
struct Content {
    mc_version: String,
    loader: Loader,
    /// Server pack de CurseForge: el loader se averigua tras extraerlo.
    prepackaged: bool,
    source: Option<ContentSource>,
    manifest: Option<PackManifest>,
}

impl Provisioner<'_> {
    pub async fn create(
        &self,
        req: NewServerRequest,
        on_step: &(dyn Fn(ProvisionStep) + Send + Sync),
    ) -> Result<ServerConfig> {
        // Validar antes de descargar nada.
        if !req.eula_accepted {
            return Err(Error::InvalidConfig(tr!(
                "debes aceptar el EULA de Minecraft (https://aka.ms/MinecraftEULA) para continuar",
                "you must accept the Minecraft EULA (https://aka.ms/MinecraftEULA) to continue"
            )));
        }
        let hw = hardware::hardware_info();
        validate_resources(
            &req.memory,
            &req.cpu,
            req.port,
            hw.total_memory_mb,
            hw.logical_cpus,
        )?;
        let used = used_ports(self.paths, None);
        check_port_free_among(req.port, &used)?;
        let owner = req
            .owner
            .as_deref()
            .map(str::trim)
            .filter(|o| !o.is_empty())
            .map(|o| players::validate_name(o).map(str::to_string))
            .transpose()?;

        let id = self.unique_id(&req.name);
        let dir = self.paths.server_dir(&id);
        tokio::fs::create_dir_all(&dir).await?;
        let result = self.create_in(&dir, id, req, owner, &used, on_step).await;
        if result.is_err() {
            // No dejar carpetas a medias que luego parezcan servidores rotos.
            let _ = tokio::fs::remove_dir_all(&dir).await;
        }
        result
    }

    async fn create_in(
        &self,
        dir: &Path,
        id: String,
        req: NewServerRequest,
        owner: Option<String>,
        used: &[(u16, String)],
        on_step: &(dyn Fn(ProvisionStep) + Send + Sync),
    ) -> Result<ServerConfig> {
        on_step(ProvisionStep::FetchingContent);
        let content = self.fetch_content(&req.source, dir, on_step).await?;
        let mut mc_version = content.mc_version;

        // Versión de Java: el manifiesto de Mojang manda; la heurística es el respaldo.
        let java_major = self.java_major_for(&mc_version).await;
        on_step(ProvisionStep::InstallingJava { major: java_major });
        let java_progress =
            |downloaded, total| on_step(ProvisionStep::DownloadingJava { downloaded, total });
        let java = self.java.ensure(java_major, Some(&java_progress)).await?;

        let (loader, launch) = if content.prepackaged {
            on_step(ProvisionStep::InstallingLoader);
            let (loader, launch) = self.finish_server_pack(dir, &mc_version, &java).await?;
            if let Some(detected) = detect_mc_version(dir) {
                mc_version = detected;
            }
            (loader, launch)
        } else {
            on_step(ProvisionStep::InstallingLoader);
            let launch =
                loaders::install_loader(self.http, &content.loader, &mc_version, dir, &java, None)
                    .await?;
            (content.loader, launch)
        };

        on_step(ProvisionStep::Finalizing);
        let taken: Vec<u16> = used.iter().map(|(p, _)| *p).chain([req.port]).collect();
        let map = MapConfig {
            port: ports::suggest_port(MapConfig::default().port, &taken),
            ..MapConfig::default()
        };
        let mut config = ServerConfig {
            id,
            name: req.name.trim().to_string(),
            mc_version,
            loader,
            java_major,
            memory: req.memory,
            cpu: req.cpu,
            port: req.port,
            eula_accepted: req.eula_accepted,
            launch,
            use_aikar_flags: true,
            extra_jvm_args: Vec::new(),
            backups: BackupPolicy::default(),
            schedule: SchedulePolicy::default(),
            network: NetworkPolicy::default(),
            security: SecurityPolicy::default(),
            map,
            source: content.source,
            pending_owner: None,
            world: WorldTweaks::default(),
        };
        eula::write_eula(dir, config.eula_accepted).await?;
        let mut values: Vec<(&str, String)> = vec![
            ("server-port", config.port.to_string()),
            ("motd", config.name.clone()),
        ];
        values.extend(SECURE_DEFAULTS.iter().map(|(k, v)| (*k, v.to_string())));
        properties::set_properties(dir, &values).await?;
        if let Some(owner) = owner {
            config.pending_owner = add_owner(self.http, dir, &owner).await;
        }
        if let Some(manifest) = content.manifest {
            manifest.save(dir).await?;
        }
        config.save(dir).await?;
        Ok(config)
    }

    async fn fetch_content(
        &self,
        source: &ServerSource,
        dir: &Path,
        on_step: &(dyn Fn(ProvisionStep) + Send + Sync),
    ) -> Result<Content> {
        let files_progress = |done, total| on_step(ProvisionStep::DownloadingFiles { done, total });
        Ok(match source {
            ServerSource::Vanilla { mc_version } => Content {
                mc_version: mc_version.clone(),
                loader: Loader::Vanilla,
                prepackaged: false,
                source: None,
                manifest: None,
            },
            ServerSource::Fabric {
                mc_version,
                loader_version,
            } => {
                let loader_version = match loader_version {
                    Some(v) => v.clone(),
                    None => loaders::latest_fabric_loader(self.http, mc_version).await?,
                };
                Content {
                    mc_version: mc_version.clone(),
                    loader: Loader::Fabric { loader_version },
                    prepackaged: false,
                    source: None,
                    manifest: None,
                }
            }
            ServerSource::Forge {
                mc_version,
                version,
            } => {
                let version = match version {
                    Some(v) => v.clone(),
                    None => loaders::latest_forge(self.http, mc_version).await?,
                };
                Content {
                    mc_version: mc_version.clone(),
                    loader: Loader::Forge { version },
                    prepackaged: false,
                    source: None,
                    manifest: None,
                }
            }
            ServerSource::NeoForge {
                mc_version,
                version,
            } => {
                let version = match version {
                    Some(v) => v.clone(),
                    None => loaders::latest_neoforge(self.http, mc_version).await?,
                };
                Content {
                    mc_version: mc_version.clone(),
                    loader: Loader::NeoForge { version },
                    prepackaged: false,
                    source: None,
                    manifest: None,
                }
            }
            ServerSource::Modrinth { version_id } => {
                let version = self.modrinth.version(version_id).await?;
                let pack_path = self.download_mrpack(&version).await?;
                let info = mrpack::install_mrpack(
                    self.http,
                    &pack_path,
                    dir,
                    Some(&files_progress),
                    &internal_paths(),
                )
                .await?;
                let _ = tokio::fs::remove_file(&pack_path).await;
                Content {
                    mc_version: info.mc_version,
                    loader: info.loader,
                    prepackaged: false,
                    source: Some(ContentSource::Modrinth {
                        project_id: version.project_id.clone(),
                        version_id: version.id.clone(),
                    }),
                    manifest: Some(PackManifest {
                        name: info.name,
                        version: version.version_number.clone(),
                        files: info.files,
                    }),
                }
            }
            ServerSource::CurseForge { mod_id, file_id } => {
                let cf = self.curseforge.ok_or_else(curseforge_missing)?;
                let file = cf.file(*mod_id, *file_id).await?;
                let server_pack = cf.server_pack_for(&file).await?;
                let zip_path = self.paths.cache().join(cache_name(&server_pack.file_name));
                cf.download(&server_pack, &zip_path, None).await?;
                let dir_clone = dir.to_path_buf();
                let zip_clone = zip_path.clone();
                let files = tokio::task::spawn_blocking(move || {
                    let protected = internal_paths();
                    archive::extract_zip_filtered(&zip_clone, &dir_clone, true, &|rel| {
                        mrpack::is_protected(rel, &protected)
                    })
                })
                .await
                .expect("la extracción no debería entrar en pánico")?;
                let _ = tokio::fs::remove_file(&zip_path).await;
                let mc = file
                    .minecraft_version()
                    .map(str::to_string)
                    .or_else(|| detect_mc_version(dir))
                    .or_else(|| pack_declared_loader(dir).and_then(|(mc, _)| mc))
                    .ok_or_else(|| {
                        Error::NotFound(tr!(
                            "versión de Minecraft del modpack",
                            "the modpack's Minecraft version"
                        ))
                    })?;
                Content {
                    mc_version: mc,
                    loader: Loader::Vanilla,
                    prepackaged: true,
                    source: Some(ContentSource::CurseForge {
                        mod_id: *mod_id,
                        file_id: *file_id,
                    }),
                    manifest: Some(PackManifest {
                        name: file.display_name.clone(),
                        version: file.display_name.clone(),
                        files,
                    }),
                }
            }
        })
    }

    /// Descarga el `.mrpack` de una versión verificando su SHA-512.
    pub async fn download_mrpack(
        &self,
        version: &crate::providers::modrinth::Version,
    ) -> Result<PathBuf> {
        let file = version
            .primary_file()
            .ok_or_else(|| Error::NotFound(".mrpack".into()))?;
        let sha512 = file.hashes.sha512.as_ref().ok_or_else(|| {
            Error::Api(tr!(
                "Modrinth no publicó el hash del modpack, así que no se puede verificar",
                "Modrinth did not publish the modpack's hash, so it cannot be verified"
            ))
        })?;
        if !mrpack::is_allowed_url(&file.url) {
            return Err(Error::DisallowedUrl(file.url.clone()));
        }
        let pack_path = self.paths.cache().join(cache_name(&file.filename));
        let expected = ExpectedHash::new(HashAlgo::Sha512, sha512);
        download_file(self.http, &file.url, &pack_path, Some(&expected), None).await?;
        Ok(pack_path)
    }

    pub async fn java_major_for(&self, mc_version: &str) -> u32 {
        java_major_for(self.http, mc_version).await
    }

    /// Deja listo un server pack ya extraído: ejecuta el instalador que trae,
    /// usa el que ya viene instalado o instala el loader que declara.
    pub(crate) async fn finish_server_pack(
        &self,
        dir: &Path,
        mc_version: &str,
        java: &crate::java::JavaRuntime,
    ) -> Result<(Loader, LaunchTarget)> {
        if let Some(installer) = bundled_installer(dir) {
            let launch = loaders::run_installer(&installer, dir, java).await?;
            let _ = tokio::fs::remove_file(&installer).await;
            return Ok((detect_loader(dir), launch));
        }
        let detected = detect_loader(dir);
        match (detect_launch_target(dir)?, pack_declared_loader(dir)) {
            // Loader ya instalado dentro del pack.
            (Some(launch), _) if detected.supports_mods() => return Ok((detected, launch)),
            // El pack solo declara qué loader necesita: lo instalamos nosotros.
            (_, Some((declared_mc, loader))) => {
                let mc = declared_mc.unwrap_or_else(|| mc_version.to_string());
                let launch =
                    loaders::install_loader(self.http, &loader, &mc, dir, java, None).await?;
                return Ok((loader, launch));
            }
            (Some(launch), None) => return Ok((detected, launch)),
            (None, None) => {}
        }
        Err(Error::Api(tr!(
            "este server pack no trae un servidor instalable que Micra reconozca. Prueba con la versión de Modrinth del pack o impórtalo después de instalarlo a mano",
            "this server pack does not include an installable server that Micra recognises. Try the pack's Modrinth version, or import it after installing it by hand"
        )))
    }

    fn unique_id(&self, name: &str) -> String {
        unique_id(self.paths, name)
    }
}

/// Nombre seguro para guardar en la caché un archivo cuyo nombre viene de
/// una API (sin rutas).
pub(crate) fn cache_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    if base.is_empty() || base.starts_with('.') {
        "download.bin".into()
    } else {
        base.to_string()
    }
}

/// Archivos de Micra que ningún pack puede sobrescribir.
pub fn internal_paths() -> Vec<String> {
    vec![
        crate::server::config::CONFIG_FILE.to_string(),
        ".micra".to_string(),
    ]
}

/// Java que pide una versión de Minecraft: el manifiesto de Mojang manda y
/// la heurística es el respaldo sin conexión.
pub async fn java_major_for(http: &reqwest::Client, mc_version: &str) -> u32 {
    match mojang::fetch_version(http, mc_version).await {
        Ok(details) => details.java_major(),
        Err(e) => {
            tracing::warn!("sin manifiesto de Mojang ({e}); usando heurística");
            required_java_for(mc_version)
        }
    }
}

/// Identificador libre para un servidor nuevo a partir de su nombre. Tampoco
/// vale el de un servidor borrado cuyas copias se guardaron: el nuevo las
/// heredaría.
pub fn unique_id(paths: &AppPaths, name: &str) -> String {
    let base = slugify(name);
    let mut id = base.clone();
    let mut n = 2;
    while paths.server_dir(&id).exists() || paths.backups_dir(&id).exists() {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// Añade al dueño a la lista blanca y a operadores editando los JSON. Si no
/// se puede consultar su UUID ahora (sin internet), devuelve el nombre para
/// hacerlo con comandos en el primer arranque.
async fn add_owner(http: &reqwest::Client, dir: &Path, owner: &str) -> Option<String> {
    match players::lookup_profile(http, owner).await {
        Ok(profile) => {
            for action in [PlayerAction::WhitelistAdd, PlayerAction::Op] {
                if let Err(e) =
                    players::apply_offline(dir, &action, &profile.name, Some(&profile)).await
                {
                    tracing::warn!("no se pudo añadir al dueño: {e}");
                    return Some(owner.to_string());
                }
            }
            None
        }
        Err(e) => {
            tracing::warn!("no se pudo consultar la cuenta {owner}: {e}");
            Some(owner.to_string())
        }
    }
}

/// Puertos que ya usan los servidores de Micra (juego y mapa), con el
/// nombre del servidor, salvo el servidor `except`.
pub fn used_ports(paths: &AppPaths, except: Option<&str>) -> Vec<(u16, String)> {
    let Ok(entries) = std::fs::read_dir(paths.servers()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(raw) = std::fs::read(entry.path().join(crate::server::config::CONFIG_FILE)) else {
            continue;
        };
        let Ok(cfg) = serde_json::from_slice::<ServerConfig>(&raw) else {
            continue;
        };
        if Some(cfg.id.as_str()) == except {
            continue;
        }
        out.push((cfg.port, cfg.name.clone()));
        if cfg.map.enabled {
            out.push((cfg.map.port, cfg.name));
        }
    }
    out
}

/// Error si otro servidor de Micra ya tiene asignado ese puerto.
pub fn check_port_free_among(port: u16, used: &[(u16, String)]) -> Result<()> {
    match used.iter().find(|(p, _)| *p == port) {
        Some((_, name)) => Err(Error::PortInUse {
            port,
            by: Some(name.clone()),
        }),
        None => Ok(()),
    }
}

fn curseforge_missing() -> Error {
    Error::InvalidConfig(tr!(
        "CurseForge no está configurado: añade tu clave de API en Ajustes",
        "CurseForge is not set up: add your API key in Settings"
    ))
}

/// Lista los servidores creados (cada carpeta con un `micra.json`).
pub async fn list_servers(paths: &AppPaths) -> Result<Vec<(ServerConfig, PathBuf)>> {
    let mut out = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(paths.servers()).await else {
        return Ok(out);
    };
    while let Some(entry) = entries.next_entry().await? {
        // Las carpetas con punto no son servidores (p. ej. uno a medio borrar).
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let dir = entry.path();
        if let Ok(cfg) = ServerConfig::load(&dir).await {
            out.push((cfg, dir));
        }
    }
    out.sort_by_key(|a| a.0.name.to_lowercase());
    Ok(out)
}

/// Prefijo de la carpeta de un servidor mientras se borra.
const DELETING_PREFIX: &str = ".deleting-";

/// Borra un servidor: su carpeta (mundo, mods y ajustes), sus estadísticas
/// y, si se pide, sus copias de seguridad. No se puede deshacer.
pub async fn delete_server(paths: &AppPaths, id: &str, with_backups: bool) -> Result<()> {
    let trash = paths.servers().join(format!("{DELETING_PREFIX}{id}"));
    remove_dir(&trash).await?;
    // Primero se aparta (es instantáneo): aunque el borrado falle a medias,
    // el servidor ya no sale en la lista.
    tokio::fs::rename(paths.server_dir(id), &trash).await?;
    remove_dir(&trash).await?;
    remove_dir(&paths.stats_dir(id)).await?;
    if with_backups {
        remove_dir(&paths.backups_dir(id)).await?;
    }
    Ok(())
}

/// Lo que quedó de servidores que no se terminaron de borrar.
pub async fn purge_deleted(paths: &AppPaths) {
    let Ok(mut entries) = tokio::fs::read_dir(paths.servers()).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(DELETING_PREFIX)
        {
            if let Err(e) = tokio::fs::remove_dir_all(entry.path()).await {
                tracing::warn!("no se pudo terminar de borrar {:?}: {e}", entry.file_name());
            }
        }
    }
}

async fn remove_dir(dir: &Path) -> Result<()> {
    match tokio::fs::remove_dir_all(dir).await {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_a_server() {
        let root = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(root.path());
        for dir in [
            paths.server_dir("survival").join("world"),
            paths.server_dir("other"),
            paths.backups_dir("survival"),
            paths.stats_dir("survival"),
            paths.backups_dir("other"),
        ] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            delete_server(&paths, "survival", false).await.unwrap();
            assert!(!paths.server_dir("survival").exists());
            assert!(!paths.stats_dir("survival").exists());
            assert!(paths.backups_dir("survival").exists());
            assert!(paths.server_dir("other").exists());
            // Uno nuevo con el mismo nombre no se queda sus copias.
            assert_eq!(unique_id(&paths, "Survival"), "survival-2");
            std::fs::create_dir_all(paths.server_dir("survival")).unwrap();
            delete_server(&paths, "survival", true).await.unwrap();
            assert!(!paths.backups_dir("survival").exists());
            assert!(paths.backups_dir("other").exists());
            assert_eq!(unique_id(&paths, "Survival"), "survival");

            // Un borrado que quedó a medias no aparece y se limpia al abrir.
            let leftover = paths.servers().join(".deleting-old");
            std::fs::create_dir_all(&leftover).unwrap();
            std::fs::write(leftover.join("server.json"), "{}").unwrap();
            assert!(list_servers(&paths).await.unwrap().is_empty());
            purge_deleted(&paths).await;
            assert!(!leftover.exists());
        });
    }

    #[test]
    fn ports_of_other_servers() {
        let used = vec![(25565, "Survival".to_string())];
        assert!(check_port_free_among(25566, &used).is_ok());
        match check_port_free_among(25565, &used) {
            Err(Error::PortInUse { port, by }) => {
                assert_eq!(port, 25565);
                assert_eq!(by.as_deref(), Some("Survival"));
            }
            other => panic!("inesperado: {other:?}"),
        }
    }

    #[test]
    fn secure_defaults_are_valid_properties() {
        for (k, v) in SECURE_DEFAULTS {
            properties::validate(k, v).unwrap();
        }
    }

    /// Lo que envía el asistente para Forge y NeoForge.
    #[test]
    fn forge_sources_from_the_wizard() {
        let forge: ServerSource =
            serde_json::from_str(r#"{"kind":"forge","mcVersion":"1.20.1","version":null}"#)
                .unwrap();
        assert!(matches!(
            forge,
            ServerSource::Forge { ref mc_version, version: None } if mc_version == "1.20.1"
        ));
        let neo: ServerSource =
            serde_json::from_str(r#"{"kind":"neoForge","mcVersion":"26.3","version":"26.3.0.95"}"#)
                .unwrap();
        assert!(matches!(
            neo,
            ServerSource::NeoForge { version: Some(ref v), .. } if v == "26.3.0.95"
        ));
    }
}
