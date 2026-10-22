//! Orquestación del asistente de creación: de "lo que eligió el usuario" a un
//! directorio de servidor listo para arrancar.
//!
//! 1. Obtener el contenido (vanilla, `.mrpack` de Modrinth o server pack de CurseForge).
//! 2. Averiguar la versión de Minecraft y el loader.
//! 3. Elegir y asegurar el runtime de Java correcto.
//! 4. Instalar el loader si el pack no lo trae.
//! 5. EULA, `server.properties` y `micra.json`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::download::{download_file, ExpectedHash, HashAlgo};
use crate::java::{required_java_for, JavaManager};
use crate::paths::AppPaths;
use crate::providers::curseforge::CurseForgeClient;
use crate::providers::modrinth::ModrinthClient;
use crate::providers::{loaders, mojang, mrpack};
use crate::server::config::{slugify, validate_resources, CpuConfig, Loader, MemoryConfig, ServerConfig};
use crate::server::detect::detect_launch_target;
use crate::server::{eula, properties};
use crate::{archive, hardware, Error, Result};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ServerSource {
    #[serde(rename_all = "camelCase")]
    Vanilla { mc_version: String },
    #[serde(rename_all = "camelCase")]
    Fabric { mc_version: String, loader_version: String },
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

pub struct Provisioner<'a> {
    pub http: &'a reqwest::Client,
    pub paths: &'a AppPaths,
    pub java: &'a JavaManager,
    pub modrinth: &'a ModrinthClient,
    pub curseforge: Option<&'a CurseForgeClient>,
}

impl Provisioner<'_> {
    pub async fn create(
        &self,
        req: NewServerRequest,
        on_step: &(dyn Fn(ProvisionStep) + Send + Sync),
    ) -> Result<ServerConfig> {
        // Validar antes de descargar nada.
        if !req.eula_accepted {
            return Err(Error::InvalidConfig(
                "debes aceptar el EULA de Minecraft (https://aka.ms/MinecraftEULA) para continuar".into(),
            ));
        }
        let hw = hardware::hardware_info();
        validate_resources(&req.memory, &req.cpu, req.port, hw.total_memory_mb, hw.logical_cpus)?;
        let id = self.unique_id(&req.name);
        let dir = self.paths.server_dir(&id);
        tokio::fs::create_dir_all(&dir).await?;

        on_step(ProvisionStep::FetchingContent);
        let files_progress = |done, total| on_step(ProvisionStep::DownloadingFiles { done, total });
        let (mc_version, loader, prepackaged) = match &req.source {
            ServerSource::Vanilla { mc_version } => (mc_version.clone(), Loader::Vanilla, false),
            ServerSource::Fabric { mc_version, loader_version } => (
                mc_version.clone(),
                Loader::Fabric { loader_version: loader_version.clone() },
                false,
            ),
            ServerSource::Modrinth { version_id } => {
                let version = self.modrinth.version(version_id).await?;
                let file = version
                    .primary_file()
                    .ok_or_else(|| Error::NotFound("archivo .mrpack".into()))?;
                let pack_path = self.paths.cache().join(&file.filename);
                let expected = file.hashes.sha512.as_ref().map(|h| ExpectedHash::new(HashAlgo::Sha512, h));
                download_file(self.http, &file.url, &pack_path, expected.as_ref(), None).await?;
                let info = mrpack::install_mrpack(self.http, &pack_path, &dir, Some(&files_progress)).await?;
                (info.mc_version, info.loader, false)
            }
            ServerSource::CurseForge { mod_id, file_id } => {
                let cf = self.curseforge.ok_or_else(|| {
                    Error::InvalidConfig("CurseForge no está configurado (falta la API key)".into())
                })?;
                let file = cf.file(*mod_id, *file_id).await?;
                let server_pack = cf.server_pack_for(&file).await?;
                let zip_path = self.paths.cache().join(&server_pack.file_name);
                cf.download(&server_pack, &zip_path, None).await?;
                let dir_clone = dir.clone();
                tokio::task::spawn_blocking(move || archive::extract_zip(&zip_path, &dir_clone, true))
                    .await
                    .expect("la extracción no debería entrar en pánico")?;
                let mc = file
                    .minecraft_version()
                    .ok_or_else(|| Error::NotFound("versión de Minecraft del modpack".into()))?
                    .to_string();
                // El server pack ya trae su loader; lo detectamos al final.
                (mc, Loader::Vanilla, true)
            }
        };

        // Versión de Java: el manifiesto de Mojang manda; la heurística es el respaldo.
        let java_major = match mojang::fetch_version(self.http, &mc_version).await {
            Ok(details) => details.java_major(),
            Err(e) => {
                tracing::warn!("sin manifiesto de Mojang ({e}); usando heurística");
                required_java_for(&mc_version)
            }
        };
        on_step(ProvisionStep::InstallingJava { major: java_major });
        let java_progress = |downloaded, total| on_step(ProvisionStep::DownloadingJava { downloaded, total });
        let java = self.java.ensure(java_major, Some(&java_progress)).await?;

        let launch = match detect_launch_target(&dir)? {
            Some(target) if prepackaged => target,
            _ => {
                on_step(ProvisionStep::InstallingLoader);
                loaders::install_loader(self.http, &loader, &mc_version, &dir, &java, None).await?
            }
        };

        on_step(ProvisionStep::Finalizing);
        let config = ServerConfig {
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
        };
        eula::write_eula(&dir, config.eula_accepted).await?;
        properties::set_properties(
            &dir,
            &[
                ("server-port", config.port.to_string()),
                ("motd", config.name.clone()),
            ],
        )
        .await?;
        config.save(&dir).await?;
        Ok(config)
    }

    fn unique_id(&self, name: &str) -> String {
        let base = slugify(name);
        let mut id = base.clone();
        let mut n = 2;
        while self.paths.server_dir(&id).exists() {
            id = format!("{base}-{n}");
            n += 1;
        }
        id
    }
}

/// Lista los servidores creados (cada carpeta con un `micra.json`).
pub async fn list_servers(paths: &AppPaths) -> Result<Vec<(ServerConfig, PathBuf)>> {
    let mut out = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(paths.servers()).await else {
        return Ok(out);
    };
    while let Some(entry) = entries.next_entry().await? {
        let dir = entry.path();
        if let Ok(cfg) = ServerConfig::load(&dir).await {
            out.push((cfg, dir));
        }
    }
    out.sort_by(|a, b| a.0.name.cmp(&b.0.name));
    Ok(out)
}
