//! Registro de lo que instaló un modpack (`.micra/pack.json`), para saber
//! qué archivos quitar o sustituir al actualizarlo sin tocar lo que añadió
//! la persona por su cuenta.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::archive::{self, safe_join};
use crate::fsutil::write_atomic;
use crate::providers::curseforge::CurseForgeClient;
use crate::providers::loaders;
use crate::providers::modrinth::ModrinthClient;
use crate::providers::mrpack::{self, is_protected};
use crate::provision::{cache_name, internal_paths, ProvisionStep, Provisioner};
use crate::server::backup::{create_backup, BackupKind, BackupScope};
use crate::server::config::{ContentSource, ServerConfig};
use crate::server::properties::{level_name, read_properties_sync};
use crate::{tr, Error, Result};

pub const MANIFEST: &str = ".micra/pack.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackManifest {
    pub name: String,
    pub version: String,
    /// Rutas relativas con `/`.
    pub files: Vec<String>,
}

impl PackManifest {
    pub async fn load(server_dir: &Path) -> Option<Self> {
        let raw = tokio::fs::read(server_dir.join(MANIFEST)).await.ok()?;
        serde_json::from_slice(&raw).ok()
    }

    pub async fn save(&self, server_dir: &Path) -> Result<()> {
        let path = server_dir.join(MANIFEST);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        write_atomic(&path, &serde_json::to_vec_pretty(self)?).await
    }
}

/// Versión nueva disponible del modpack de un servidor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackUpdate {
    pub current: Option<String>,
    pub latest: String,
    pub target: UpdateTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UpdateTarget {
    #[serde(rename_all = "camelCase")]
    Modrinth { version_id: String },
    #[serde(rename_all = "camelCase")]
    CurseForge { file_id: u64 },
}

/// Busca una versión más nueva del modpack para la misma versión de
/// Minecraft (cambiar de versión del juego puede romper el mundo).
pub async fn check_update(
    modrinth: &ModrinthClient,
    curseforge: Option<&CurseForgeClient>,
    cfg: &ServerConfig,
    manifest: Option<&PackManifest>,
) -> Result<Option<PackUpdate>> {
    let current = manifest.map(|m| m.version.clone());
    match &cfg.source {
        None => Ok(None),
        Some(ContentSource::Modrinth {
            project_id,
            version_id,
        }) => {
            let versions = modrinth
                .project_versions(project_id, Some(&cfg.mc_version), None)
                .await?;
            // Modrinth las da de la más nueva a la más antigua: solo cuentan
            // las anteriores a la instalada.
            let newer: Vec<_> = versions
                .iter()
                .take_while(|v| &v.id != version_id)
                .collect();
            let installed_is_release = versions
                .iter()
                .find(|v| &v.id == version_id)
                .is_none_or(|v| v.version_type == "release");
            let pick =
                newer
                    .iter()
                    .find(|v| v.version_type == "release")
                    .or(if installed_is_release {
                        None
                    } else {
                        newer.first()
                    });
            Ok(pick.map(|v| PackUpdate {
                current,
                latest: v.version_number.clone(),
                target: UpdateTarget::Modrinth {
                    version_id: v.id.clone(),
                },
            }))
        }
        Some(ContentSource::CurseForge { mod_id, file_id }) => {
            let Some(cf) = curseforge else {
                return Ok(None);
            };
            let files = cf.files(*mod_id, Some(&cfg.mc_version)).await?;
            let pick = files
                .iter()
                .filter(|f| f.id > *file_id && f.is_server_pack != Some(true))
                .filter(|f| f.server_pack_file_id.is_some())
                .max_by_key(|f| f.id);
            Ok(pick.map(|f| PackUpdate {
                current,
                latest: f.display_name.clone(),
                target: UpdateTarget::CurseForge { file_id: f.id },
            }))
        }
    }
}

/// Lo que una actualización del pack nunca toca: el mundo, la configuración
/// del servidor, las listas de jugadores y los archivos de Micra.
pub fn protected_paths(server_dir: &Path) -> Vec<String> {
    let level = level_name(&read_properties_sync(server_dir));
    let mut out = vec![format!("{level}_nether"), format!("{level}_the_end"), level];
    out.extend(
        [
            "server.properties",
            "eula.txt",
            "whitelist.json",
            "ops.json",
            "banned-players.json",
            "banned-ips.json",
            "usercache.json",
        ]
        .map(String::from),
    );
    out.extend(internal_paths());
    out
}

/// Borra los archivos del pack anterior que ya no están en el nuevo
/// (también su variante desactivada). Lo que añadió la persona no se toca.
async fn remove_dropped_files(
    server_dir: &Path,
    old: &[String],
    new: &HashSet<String>,
    protected: &[String],
) {
    for rel in old {
        if new.contains(rel) || is_protected(rel, protected) {
            continue;
        }
        let Ok(path) = safe_join(server_dir, rel) else {
            continue;
        };
        let _ = tokio::fs::remove_file(&path).await;
        let mut disabled = path.into_os_string();
        disabled.push(".disabled");
        let _ = tokio::fs::remove_file(PathBuf::from(disabled)).await;
    }
}

/// Mods que la persona había desactivado y que el pack nuevo vuelve a
/// traer: se mantienen desactivados, ya con el archivo nuevo.
async fn keep_disabled(server_dir: &Path, new_files: &HashSet<String>, before: &[String]) {
    for rel in before {
        if !new_files.contains(rel) {
            continue;
        }
        let Ok(path) = safe_join(server_dir, rel) else {
            continue;
        };
        let mut disabled = path.clone().into_os_string();
        disabled.push(".disabled");
        let disabled = PathBuf::from(disabled);
        if path.exists() {
            let _ = tokio::fs::remove_file(&disabled).await;
            let _ = tokio::fs::rename(&path, &disabled).await;
        }
    }
}

/// Archivos del pack (ya registrados) que están desactivados ahora mismo.
fn disabled_pack_files(server_dir: &Path, files: &[String]) -> Vec<String> {
    files
        .iter()
        .filter(|rel| safe_join(server_dir, &format!("{rel}.disabled")).is_ok_and(|p| p.exists()))
        .cloned()
        .collect()
}

impl Provisioner<'_> {
    /// Actualiza el modpack de un servidor apagado. Antes hace una copia
    /// completa (`pre-update`) para poder volver atrás desde Copias.
    pub async fn update_pack(
        &self,
        server_dir: &Path,
        cfg: &mut ServerConfig,
        target: &UpdateTarget,
        on_step: &(dyn Fn(ProvisionStep) + Send + Sync),
    ) -> Result<()> {
        create_backup(
            server_dir,
            &self.paths.backups_dir(&cfg.id),
            cfg,
            BackupKind::PreUpdate,
            BackupScope::Full,
            None,
        )
        .await?;
        let old = PackManifest::load(server_dir).await.unwrap_or_default();
        let protected = protected_paths(server_dir);
        let disabled_before = disabled_pack_files(server_dir, &old.files);
        on_step(ProvisionStep::FetchingContent);

        let (manifest, source) = match (target, &cfg.source) {
            (
                UpdateTarget::Modrinth { version_id },
                Some(ContentSource::Modrinth { project_id, .. }),
            ) => {
                let version = self.modrinth.version(version_id).await?;
                if &version.project_id != project_id {
                    return Err(wrong_pack());
                }
                let pack = self.download_mrpack(&version).await?;
                let info = mrpack::inspect(&pack).await?;
                if info.mc_version != cfg.mc_version {
                    return Err(other_minecraft(&info.mc_version));
                }
                let new_files: HashSet<String> = info.files.iter().cloned().collect();
                remove_dropped_files(server_dir, &old.files, &new_files, &protected).await;
                let files_progress =
                    |done, total| on_step(ProvisionStep::DownloadingFiles { done, total });
                let installed = mrpack::install_mrpack(
                    self.http,
                    &pack,
                    server_dir,
                    Some(&files_progress),
                    &protected,
                )
                .await?;
                let _ = tokio::fs::remove_file(&pack).await;
                if installed.loader != cfg.loader {
                    on_step(ProvisionStep::InstallingLoader);
                    let java = self.java.ensure(cfg.java_major, None).await?;
                    cfg.launch = loaders::install_loader(
                        self.http,
                        &installed.loader,
                        &cfg.mc_version,
                        server_dir,
                        &java,
                        None,
                    )
                    .await?;
                    cfg.loader = installed.loader.clone();
                }
                keep_disabled(server_dir, &new_files, &disabled_before).await;
                (
                    PackManifest {
                        name: installed.name,
                        version: version.version_number.clone(),
                        files: installed.files,
                    },
                    ContentSource::Modrinth {
                        project_id: project_id.clone(),
                        version_id: version.id.clone(),
                    },
                )
            }
            (
                UpdateTarget::CurseForge { file_id },
                Some(ContentSource::CurseForge { mod_id, .. }),
            ) => {
                let cf = self.curseforge.ok_or_else(|| {
                    Error::InvalidConfig(tr!(
                        "CurseForge no está configurado: añade tu clave de API en Ajustes",
                        "CurseForge is not set up: add your API key in Settings"
                    ))
                })?;
                let file = cf.file(*mod_id, *file_id).await?;
                if file
                    .minecraft_version()
                    .is_some_and(|mc| mc != cfg.mc_version)
                {
                    return Err(other_minecraft(
                        file.minecraft_version().unwrap_or_default(),
                    ));
                }
                let server_pack = cf.server_pack_for(&file).await?;
                let zip_path = self.paths.cache().join(cache_name(&server_pack.file_name));
                cf.download(&server_pack, &zip_path, None).await?;
                let listed = {
                    let zip = zip_path.clone();
                    tokio::task::spawn_blocking(move || archive::list_zip(&zip, true))
                        .await
                        .expect("la lectura no debería entrar en pánico")?
                };
                let new_files: HashSet<String> = listed.iter().cloned().collect();
                remove_dropped_files(server_dir, &old.files, &new_files, &protected).await;
                let written = {
                    let zip = zip_path.clone();
                    let dir = server_dir.to_owned();
                    let protected = protected.clone();
                    tokio::task::spawn_blocking(move || {
                        archive::extract_zip_filtered(&zip, &dir, true, &|rel| {
                            is_protected(rel, &protected)
                        })
                    })
                    .await
                    .expect("la extracción no debería entrar en pánico")?
                };
                let _ = tokio::fs::remove_file(&zip_path).await;
                on_step(ProvisionStep::InstallingLoader);
                let java = self.java.ensure(cfg.java_major, None).await?;
                let (loader, launch) = self
                    .finish_server_pack(server_dir, &cfg.mc_version, &java)
                    .await?;
                cfg.loader = loader;
                cfg.launch = launch;
                keep_disabled(server_dir, &new_files, &disabled_before).await;
                (
                    PackManifest {
                        name: file.display_name.clone(),
                        version: file.display_name.clone(),
                        files: written,
                    },
                    ContentSource::CurseForge {
                        mod_id: *mod_id,
                        file_id: *file_id,
                    },
                )
            }
            _ => return Err(wrong_pack()),
        };
        on_step(ProvisionStep::Finalizing);
        manifest.save(server_dir).await?;
        cfg.source = Some(source);
        cfg.save(server_dir).await?;
        Ok(())
    }
}

fn wrong_pack() -> Error {
    Error::InvalidConfig(tr!(
        "esa actualización no corresponde al modpack de este servidor",
        "that update does not belong to this server's modpack"
    ))
}

fn other_minecraft(mc: &str) -> Error {
    Error::InvalidConfig(tr!(
        "la versión nueva es para Minecraft {mc}; cambiar de versión del juego puede romper el mundo, así que crea un servidor nuevo para ella",
        "the new version is for Minecraft {mc}; changing the game version can break the world, so create a new server for it"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_files_and_disabled_mods() {
        let server = tempfile::tempdir().unwrap();
        let s = server.path();
        for f in [
            "mods/a.jar",
            "mods/b.jar.disabled",
            "mods/mine.jar",
            "world/level.dat",
        ] {
            let p = s.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x").unwrap();
        }
        let old = vec![
            "mods/a.jar".to_string(),
            "mods/b.jar".to_string(),
            "world/level.dat".to_string(),
        ];
        assert_eq!(disabled_pack_files(s, &old), vec!["mods/b.jar".to_string()]);

        let new: HashSet<String> = ["mods/b.jar".to_string()].into();
        let protected = protected_paths(s);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            remove_dropped_files(s, &old, &new, &protected).await;
            assert!(!s.join("mods/a.jar").exists());
            // Lo de la persona y el mundo siguen.
            assert!(s.join("mods/mine.jar").exists());
            assert!(s.join("world/level.dat").exists());
            // El pack nuevo trae b.jar otra vez: debe seguir desactivado.
            std::fs::write(s.join("mods/b.jar"), "nuevo").unwrap();
            keep_disabled(s, &new, &["mods/b.jar".to_string()]).await;
            assert!(!s.join("mods/b.jar").exists());
            assert_eq!(
                std::fs::read_to_string(s.join("mods/b.jar.disabled")).unwrap(),
                "nuevo"
            );
        });
    }
}
