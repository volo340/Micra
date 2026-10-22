use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::{Error, Result};

/// Directorios de datos de la app. Todo lo que Micra descarga vive aquí,
/// aislado del sistema del usuario (nunca tocamos el Java global ni el PATH).
///
/// - Linux:   ~/.local/share/micra
/// - Windows: %APPDATA%\Micra\Micra\data
#[derive(Debug, Clone)]
pub struct AppPaths {
    root: PathBuf,
}

impl AppPaths {
    pub fn from_system() -> Result<Self> {
        let dirs = ProjectDirs::from("dev", "Micra", "Micra")
            .ok_or_else(|| Error::NotFound("directorio de datos del usuario".into()))?;
        Ok(Self::new(dirs.data_dir()))
    }

    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn runtimes(&self) -> PathBuf {
        self.root.join("runtimes")
    }

    pub fn runtime_dir(&self, java_major: u32) -> PathBuf {
        self.runtimes().join(format!("java-{java_major}"))
    }

    pub fn servers(&self) -> PathBuf {
        self.root.join("servers")
    }

    pub fn server_dir(&self, server_id: &str) -> PathBuf {
        self.servers().join(server_id)
    }

    pub fn cache(&self) -> PathBuf {
        self.root.join("cache")
    }
}
