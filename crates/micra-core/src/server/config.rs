use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

pub const CONFIG_FILE: &str = "micra.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Loader {
    Vanilla,
    #[serde(rename_all = "camelCase")]
    Fabric {
        loader_version: String,
    },
    #[serde(rename_all = "camelCase")]
    Quilt {
        loader_version: String,
    },
    Forge {
        version: String,
    },
    NeoForge {
        version: String,
    },
}

/// Cómo se arranca el servidor. Forge/NeoForge modernos (1.17+) ya no usan
/// un jar ejecutable sino un archivo de argumentos `@libraries/.../unix_args.txt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LaunchTarget {
    /// Ruta relativa al directorio del servidor.
    Jar { path: PathBuf },
    /// Ruta relativa al directorio del servidor.
    ArgsFile { path: PathBuf },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryConfig {
    pub min_mb: u64,
    pub max_mb: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuConfig {
    /// Índices de CPU lógicas permitidas. `None` = todas (sin afinidad).
    pub cores: Option<Vec<usize>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub id: String,
    pub name: String,
    pub mc_version: String,
    pub loader: Loader,
    pub java_major: u32,
    pub memory: MemoryConfig,
    #[serde(default)]
    pub cpu: CpuConfig,
    pub port: u16,
    pub eula_accepted: bool,
    pub launch: LaunchTarget,
    #[serde(default = "default_true")]
    pub use_aikar_flags: bool,
    #[serde(default)]
    pub extra_jvm_args: Vec<String>,
}

fn default_true() -> bool {
    true
}

impl ServerConfig {
    pub fn validate(&self, total_memory_mb: u64, logical_cpus: usize) -> Result<()> {
        validate_resources(
            &self.memory,
            &self.cpu,
            self.port,
            total_memory_mb,
            logical_cpus,
        )
    }

    pub async fn load(server_dir: &Path) -> Result<Self> {
        let raw = tokio::fs::read(server_dir.join(CONFIG_FILE)).await?;
        Ok(serde_json::from_slice(&raw)?)
    }

    pub async fn save(&self, server_dir: &Path) -> Result<()> {
        tokio::fs::create_dir_all(server_dir).await?;
        tokio::fs::write(
            server_dir.join(CONFIG_FILE),
            serde_json::to_vec_pretty(self)?,
        )
        .await?;
        Ok(())
    }
}

/// Comprueba RAM, núcleos y puerto. Se usa antes de descargar nada, para
/// que un error del asistente no se descubra tras varios minutos de descarga.
pub fn validate_resources(
    memory: &MemoryConfig,
    cpu: &CpuConfig,
    port: u16,
    total_memory_mb: u64,
    logical_cpus: usize,
) -> Result<()> {
    if memory.max_mb < 512 {
        return Err(Error::InvalidConfig(
            "la RAM máxima debe ser al menos 512 MB".into(),
        ));
    }
    if memory.min_mb > memory.max_mb {
        return Err(Error::InvalidConfig(
            "la RAM mínima no puede superar a la máxima".into(),
        ));
    }
    if total_memory_mb > 0 && memory.max_mb >= total_memory_mb {
        return Err(Error::InvalidConfig(format!(
            "la RAM máxima ({} MB) debe ser menor que la RAM total del equipo ({total_memory_mb} MB)",
            memory.max_mb
        )));
    }
    if let Some(cores) = &cpu.cores {
        if cores.is_empty() {
            return Err(Error::InvalidConfig("selecciona al menos un núcleo".into()));
        }
        if let Some(bad) = cores.iter().find(|&&c| c >= logical_cpus) {
            return Err(Error::InvalidConfig(format!(
                "el núcleo {bad} no existe (este equipo tiene {logical_cpus})"
            )));
        }
    }
    if port < 1024 {
        return Err(Error::InvalidConfig(
            "usa un puerto entre 1024 y 65535".into(),
        ));
    }
    Ok(())
}

/// Convierte un nombre libre en un identificador apto para carpeta.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "servidor".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ServerConfig {
        ServerConfig {
            id: "test".into(),
            name: "Test".into(),
            mc_version: "1.21.1".into(),
            loader: Loader::Vanilla,
            java_major: 21,
            memory: MemoryConfig {
                min_mb: 2048,
                max_mb: 4096,
            },
            cpu: CpuConfig {
                cores: Some(vec![0, 1]),
            },
            port: 25565,
            eula_accepted: true,
            launch: LaunchTarget::Jar {
                path: "server.jar".into(),
            },
            use_aikar_flags: true,
            extra_jvm_args: vec![],
        }
    }

    #[test]
    fn validation() {
        assert!(sample().validate(16384, 8).is_ok());
        assert!(sample().validate(4096, 8).is_err());
        let mut c = sample();
        c.cpu.cores = Some(vec![9]);
        assert!(c.validate(16384, 8).is_err());
    }

    #[test]
    fn slug() {
        assert_eq!(slugify("  Mi Servidor  Épico!! "), "mi-servidor-pico");
        assert_eq!(slugify("¡¡!!"), "servidor");
    }
}
