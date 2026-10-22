//! Gestor de runtimes de Java aislados.
//!
//! Cada versión mayor de Java (8, 17, 21, 25...) se descarga de Eclipse
//! Temurin (Adoptium) a `<datos>/runtimes/java-<N>` y se usa con su ruta
//! absoluta. Nunca se modifica JAVA_HOME, el PATH ni el Java del sistema.

pub mod adoptium;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::download::{download_file, ExpectedHash, HashAlgo, Progress};
use crate::paths::AppPaths;
use crate::{archive, Error, Result};

const MARKER_FILE: &str = "micra-runtime.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaRuntime {
    pub major: u32,
    pub release_name: String,
    pub home: PathBuf,
    pub java_bin: PathBuf,
}

pub struct JavaManager {
    paths: AppPaths,
    http: reqwest::Client,
}

impl JavaManager {
    pub fn new(paths: AppPaths, http: reqwest::Client) -> Self {
        Self { paths, http }
    }

    /// Devuelve el runtime pedido, descargándolo e instalándolo si hace falta.
    pub async fn ensure(&self, major: u32, progress: Option<Progress<'_>>) -> Result<JavaRuntime> {
        if let Some(rt) = self.installed(major) {
            return Ok(rt);
        }
        self.install(major, progress).await
    }

    pub fn installed(&self, major: u32) -> Option<JavaRuntime> {
        let dir = self.paths.runtime_dir(major);
        let marker = std::fs::read(dir.join(MARKER_FILE)).ok()?;
        let rt: JavaRuntime = serde_json::from_slice(&marker).ok()?;
        rt.java_bin.is_file().then_some(rt)
    }

    pub fn list_installed(&self) -> Vec<JavaRuntime> {
        let Ok(entries) = std::fs::read_dir(self.paths.runtimes()) else {
            return Vec::new();
        };
        let mut list: Vec<_> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let major = name.strip_prefix("java-")?.parse().ok()?;
                self.installed(major)
            })
            .collect();
        list.sort_by_key(|rt| rt.major);
        list
    }

    async fn install(&self, major: u32, progress: Option<Progress<'_>>) -> Result<JavaRuntime> {
        let release = adoptium::latest_runtime(&self.http, major).await?;
        tracing::info!(major, release = %release.release_name, "instalando Java");

        let archive_path = self.paths.cache().join(&release.file_name);
        let expected = release
            .sha256
            .as_ref()
            .map(|h| ExpectedHash::new(HashAlgo::Sha256, h));
        download_file(
            &self.http,
            &release.url,
            &archive_path,
            expected.as_ref(),
            progress,
        )
        .await?;

        let target = self.paths.runtime_dir(major);
        let staging = self.paths.runtimes().join(format!(".staging-java-{major}"));
        let file_name = release.file_name.clone();
        let staging_clone = staging.clone();
        let archive_clone = archive_path.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            if staging_clone.exists() {
                std::fs::remove_dir_all(&staging_clone)?;
            }
            if file_name.ends_with(".zip") {
                archive::extract_zip(&archive_clone, &staging_clone, false)
            } else {
                archive::extract_tar_gz(&archive_clone, &staging_clone)
            }
        })
        .await
        .expect("la extracción no debería entrar en pánico")?;

        // El archivo trae una carpeta raíz tipo `jdk-21.0.5+11-jre/`.
        let home = find_java_home(&staging)?;
        if target.exists() {
            tokio::fs::remove_dir_all(&target).await?;
        }
        tokio::fs::rename(&home, &target).await?;
        let _ = tokio::fs::remove_dir_all(&staging).await;
        let _ = tokio::fs::remove_file(&archive_path).await;

        let runtime = JavaRuntime {
            major,
            release_name: release.release_name,
            java_bin: java_bin_in(&target),
            home: target.clone(),
        };
        if !runtime.java_bin.is_file() {
            return Err(Error::NotFound(format!(
                "binario de Java en {}",
                target.display()
            )));
        }
        tokio::fs::write(
            target.join(MARKER_FILE),
            serde_json::to_vec_pretty(&runtime)?,
        )
        .await?;
        Ok(runtime)
    }
}

/// Versión de Java recomendada a partir del ID de versión de Minecraft.
///
/// Es un respaldo: lo correcto es leer `javaVersion.majorVersion` del
/// manifiesto de Mojang (ver `providers::mojang`), que es lo que hace
/// `provision`. Esto se usa si no hay red o la versión no está en el manifiesto.
pub fn required_java_for(mc_version: &str) -> u32 {
    let mut parts = mc_version.split(['.', '-']).map(|p| p.parse::<u32>().ok());
    let first = parts.next().flatten().unwrap_or(1);
    let minor = parts.next().flatten().unwrap_or(0);
    let patch = parts.next().flatten().unwrap_or(0);

    // Desde 2026 Mojang usa versiones por año (26.1, 26.2...), que piden Java 25.
    if first >= 26 {
        return 25;
    }
    match (minor, patch) {
        (m, _) if m >= 21 => 21,
        (20, p) if p >= 5 => 21,
        (m, _) if m >= 18 => 17,
        (17, _) => 17,
        _ => 8,
    }
}

fn java_bin_in(home: &Path) -> PathBuf {
    let exe = if cfg!(windows) { "java.exe" } else { "java" };
    home.join("bin").join(exe)
}

fn find_java_home(staging: &Path) -> Result<PathBuf> {
    if java_bin_in(staging).is_file() {
        return Ok(staging.to_path_buf());
    }
    for entry in std::fs::read_dir(staging)?.flatten() {
        let path = entry.path();
        if path.is_dir() && java_bin_in(&path).is_file() {
            return Ok(path);
        }
    }
    Err(Error::NotFound(format!(
        "JAVA_HOME dentro de {}",
        staging.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::required_java_for;

    #[test]
    fn java_version_heuristic() {
        assert_eq!(required_java_for("1.12.2"), 8);
        assert_eq!(required_java_for("1.16.5"), 8);
        assert_eq!(required_java_for("1.17.1"), 17);
        assert_eq!(required_java_for("1.18.2"), 17);
        assert_eq!(required_java_for("1.20.4"), 17);
        assert_eq!(required_java_for("1.20.5"), 21);
        assert_eq!(required_java_for("1.21.4"), 21);
        assert_eq!(required_java_for("26.1"), 25);
    }
}
