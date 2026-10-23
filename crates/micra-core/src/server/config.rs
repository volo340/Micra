use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil::write_atomic;
use crate::{tr, Error, Result};

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
    #[serde(default)]
    pub backups: BackupPolicy,
    #[serde(default)]
    pub schedule: SchedulePolicy,
    #[serde(default)]
    pub network: NetworkPolicy,
    #[serde(default)]
    pub security: SecurityPolicy,
    #[serde(default)]
    pub map: MapConfig,
    /// De qué modpack sale el servidor, para poder actualizarlo.
    #[serde(default)]
    pub source: Option<ContentSource>,
    /// Jugador que se añade a la lista blanca y como operador la primera vez
    /// que el servidor arranca (lo pide el asistente).
    #[serde(default)]
    pub pending_owner: Option<String>,
    #[serde(default)]
    pub world: WorldTweaks,
}

fn default_true() -> bool {
    true
}

/// Copias automáticas mientras el servidor está encendido.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BackupPolicy {
    pub enabled: bool,
    pub interval_hours: u32,
    /// Copias automáticas que se guardan; las más antiguas se borran.
    pub keep: u32,
}

impl Default for BackupPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_hours: 6,
            keep: 10,
        }
    }
}

/// Reinicios programados y apagado por inactividad.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SchedulePolicy {
    /// Hora local "HH:MM" del reinicio diario.
    pub daily_restart: Option<String>,
    /// Reiniciar cada N horas de funcionamiento.
    pub restart_every_hours: Option<u32>,
    /// Apagar si no hay nadie conectado durante N minutos.
    pub idle_shutdown_minutes: Option<u32>,
}

/// Cómo llegan los jugadores de fuera de la red local. Todo desactivado por
/// defecto: abrir el servidor a internet es decisión de la persona.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NetworkPolicy {
    /// Abrir el puerto del juego en el router con UPnP.
    pub upnp: bool,
    /// Túnel de playit.gg (para conexiones con CGNAT).
    pub tunnel: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SecurityPolicy {
    pub guard: GuardPolicy,
    /// Ejecutar el servidor con permisos mínimos (ver `server::sandbox`).
    pub sandbox: bool,
    /// Windows: además, nivel de integridad bajo (experimental).
    pub low_integrity: bool,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            guard: GuardPolicy::default(),
            sandbox: true,
            low_integrity: false,
        }
    }
}

/// Bloqueo automático de IPs que intentan entrar demasiadas veces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GuardPolicy {
    pub enabled: bool,
    /// Intentos fallidos permitidos por IP dentro de la ventana.
    pub max_attempts: u32,
    pub window_secs: u32,
    /// Duración del bloqueo; 0 = permanente.
    pub ban_minutes: u32,
    /// Jugadores conectados a la vez desde una misma IP de internet; 0 = sin límite.
    pub max_per_ip: u32,
}

impl Default for GuardPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_attempts: 8,
            window_secs: 60,
            ban_minutes: 60,
            max_per_ip: 4,
        }
    }
}

/// Mapa web con BlueMap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MapConfig {
    pub enabled: bool,
    pub port: u16,
    /// La persona aceptó que BlueMap descargue los recursos del juego.
    pub accepted_download: bool,
}

impl Default for MapConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 8100,
            accepted_download: false,
        }
    }
}

/// Cambios para el mundo que esperan a que el servidor arranque.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorldTweaks {
    /// Reglas de juego por propiedad (`pvp`, `allow-nether`...) que se
    /// aplican con `/gamerule` cuando el servidor termina de arrancar.
    pub pending_rules: BTreeMap<String, bool>,
    /// Modo extremo que se escribe en `level.dat` antes de arrancar.
    pub pending_hardcore: Option<bool>,
}

/// Origen del contenido instalado, para buscar y aplicar actualizaciones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ContentSource {
    #[serde(rename_all = "camelCase")]
    Modrinth {
        project_id: String,
        version_id: String,
    },
    #[serde(rename_all = "camelCase")]
    CurseForge { mod_id: u64, file_id: u64 },
}

impl Loader {
    /// Nombre del loader en las APIs de Modrinth (`None` para vanilla).
    pub fn modrinth_name(&self) -> Option<&'static str> {
        match self {
            Loader::Vanilla => None,
            Loader::Fabric { .. } => Some("fabric"),
            Loader::Quilt { .. } => Some("quilt"),
            Loader::Forge { .. } => Some("forge"),
            Loader::NeoForge { .. } => Some("neoforge"),
        }
    }

    /// Loaders cuyos mods valen en este servidor, para las búsquedas en
    /// Modrinth: Quilt carga mods de Fabric y el NeoForge de 1.20.1 carga
    /// mods de Forge.
    pub fn modrinth_loaders(&self, mc_version: &str) -> Vec<&'static str> {
        match self {
            Loader::Vanilla => Vec::new(),
            Loader::Fabric { .. } => vec!["fabric"],
            Loader::Quilt { .. } => vec!["quilt", "fabric"],
            Loader::Forge { .. } => vec!["forge"],
            Loader::NeoForge { .. } if mc_version == "1.20.1" => vec!["neoforge", "forge"],
            Loader::NeoForge { .. } => vec!["neoforge"],
        }
    }

    pub fn supports_mods(&self) -> bool {
        !matches!(self, Loader::Vanilla)
    }
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
        write_atomic(
            &server_dir.join(CONFIG_FILE),
            &serde_json::to_vec_pretty(self)?,
        )
        .await
    }

    /// Comprueba las políticas que la persona puede editar en Ajustes.
    pub fn validate_policies(&self) -> Result<()> {
        if let Some(time) = &self.schedule.daily_restart {
            if crate::server::schedule::parse_hhmm(time).is_none() {
                return Err(Error::InvalidConfig(tr!(
                    "la hora del reinicio diario debe tener el formato HH:MM",
                    "the daily restart time must use the HH:MM format"
                )));
            }
        }
        if self.schedule.restart_every_hours == Some(0)
            || self.schedule.idle_shutdown_minutes == Some(0)
        {
            return Err(Error::InvalidConfig(tr!(
                "los intervalos deben ser mayores que cero",
                "intervals must be greater than zero"
            )));
        }
        if self.backups.enabled && (self.backups.interval_hours == 0 || self.backups.keep == 0) {
            return Err(Error::InvalidConfig(tr!(
                "las copias automáticas necesitan un intervalo y un número de copias mayores que cero",
                "automatic backups need an interval and a number of copies greater than zero"
            )));
        }
        let guard = &self.security.guard;
        if guard.enabled && (guard.max_attempts < 2 || guard.window_secs < 10) {
            return Err(Error::InvalidConfig(tr!(
                "la protección necesita al menos 2 intentos y una ventana de 10 segundos",
                "the protection needs at least 2 attempts and a 10-second window"
            )));
        }
        if self.map.port < 1024 || self.map.port == self.port {
            return Err(Error::InvalidConfig(tr!(
                "el puerto del mapa debe estar entre 1024 y 65535 y ser distinto del del juego",
                "the map port must be between 1024 and 65535 and differ from the game port"
            )));
        }
        if let Some(owner) = &self.pending_owner {
            crate::server::players::validate_name(owner)?;
        }
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
        return Err(Error::InvalidConfig(tr!(
            "la RAM máxima debe ser al menos 512 MB",
            "the maximum RAM must be at least 512 MB"
        )));
    }
    if memory.min_mb > memory.max_mb {
        return Err(Error::InvalidConfig(tr!(
            "la RAM mínima no puede superar a la máxima",
            "the minimum RAM cannot exceed the maximum"
        )));
    }
    if total_memory_mb > 0 && memory.max_mb >= total_memory_mb {
        let max = memory.max_mb;
        return Err(Error::InvalidConfig(tr!(
            "la RAM máxima ({max} MB) debe ser menor que la RAM total del equipo ({total_memory_mb} MB)",
            "the maximum RAM ({max} MB) must be lower than this computer's total RAM ({total_memory_mb} MB)"
        )));
    }
    if let Some(cores) = &cpu.cores {
        if cores.is_empty() {
            return Err(Error::InvalidConfig(tr!(
                "selecciona al menos un núcleo",
                "select at least one core"
            )));
        }
        if let Some(bad) = cores.iter().find(|&&c| c >= logical_cpus) {
            return Err(Error::InvalidConfig(tr!(
                "el núcleo {bad} no existe (este equipo tiene {logical_cpus})",
                "core {bad} does not exist (this computer has {logical_cpus})"
            )));
        }
    }
    if port < 1024 {
        return Err(Error::InvalidConfig(tr!(
            "usa un puerto entre 1024 y 65535",
            "use a port between 1024 and 65535"
        )));
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
            backups: BackupPolicy::default(),
            schedule: SchedulePolicy::default(),
            network: NetworkPolicy::default(),
            security: SecurityPolicy::default(),
            map: MapConfig::default(),
            source: None,
            pending_owner: None,
            world: WorldTweaks::default(),
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
    fn policies() {
        let mut c = sample();
        assert!(c.validate_policies().is_ok());
        c.schedule.daily_restart = Some("25:00".into());
        assert!(c.validate_policies().is_err());
        c.schedule.daily_restart = Some("04:30".into());
        c.map.port = c.port;
        assert!(c.validate_policies().is_err());
        c.map.port = 8100;
        c.pending_owner = Some("no válido".into());
        assert!(c.validate_policies().is_err());
        c.pending_owner = Some("Steve_01".into());
        assert!(c.validate_policies().is_ok());
    }

    /// Un `micra.json` de la versión anterior (sin las políticas nuevas)
    /// debe seguir cargando con los valores por defecto.
    #[test]
    fn loads_old_config() {
        let old = r#"{"id":"a","name":"A","mcVersion":"1.21.1","loader":{"kind":"vanilla"},
            "javaMajor":21,"memory":{"minMb":1024,"maxMb":2048},"port":25565,
            "eulaAccepted":true,"launch":{"kind":"jar","path":"server.jar"}}"#;
        let cfg: ServerConfig = serde_json::from_str(old).unwrap();
        assert!(cfg.backups.enabled);
        assert!(cfg.security.sandbox);
        assert!(!cfg.network.upnp);
        assert_eq!(cfg.map.port, 8100);
        assert_eq!(cfg.source, None);
        assert_eq!(cfg.world, WorldTweaks::default());
    }

    #[test]
    fn slug() {
        assert_eq!(slugify("  Mi Servidor  Épico!! "), "mi-servidor-pico");
        assert_eq!(slugify("¡¡!!"), "servidor");
    }
}
