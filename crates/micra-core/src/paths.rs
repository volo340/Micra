use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::{tr, Error, Result};

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
        let dirs = ProjectDirs::from("dev", "Micra", "Micra").ok_or_else(|| {
            Error::NotFound(tr!("directorio de datos del usuario", "user data folder"))
        })?;
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

    /// Copias de seguridad, fuera de la carpeta del servidor para que borrar
    /// o restaurar un servidor nunca se lleve sus propias copias.
    pub fn backups_dir(&self, server_id: &str) -> PathBuf {
        self.root.join("backups").join(server_id)
    }

    /// Estadísticas históricas (un CSV por día).
    pub fn stats_dir(&self, server_id: &str) -> PathBuf {
        self.root.join("stats").join(server_id)
    }

    /// Programas auxiliares que descarga Micra (p. ej. el agente del túnel).
    pub fn tools(&self) -> PathBuf {
        self.root.join("tools")
    }

    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    /// Respaldo de los secretos cuando el sistema no tiene llavero.
    pub fn secrets_file(&self) -> PathBuf {
        self.root.join("secrets.json")
    }
}
