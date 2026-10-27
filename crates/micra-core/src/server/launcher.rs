//! Construcción de la línea de comandos de Java para el servidor.

use std::path::Path;

use crate::java::JavaRuntime;
use crate::server::config::{LaunchTarget, ServerConfig};

#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Flags de G1GC recomendados por la comunidad (Aikar) para servidores de
/// Minecraft. Reducen las pausas del recolector de basura (lag spikes).
pub const AIKAR_FLAGS: &[&str] = &[
    "-XX:+UseG1GC",
    "-XX:+ParallelRefProcEnabled",
    "-XX:MaxGCPauseMillis=200",
    "-XX:+UnlockExperimentalVMOptions",
    "-XX:+DisableExplicitGC",
    "-XX:+AlwaysPreTouch",
    "-XX:G1NewSizePercent=30",
    "-XX:G1MaxNewSizePercent=40",
    "-XX:G1HeapRegionSize=8M",
    "-XX:G1ReservePercent=20",
    "-XX:G1HeapWastePercent=5",
    "-XX:G1MixedGCCountTarget=4",
    "-XX:InitiatingHeapOccupancyPercent=15",
    "-XX:G1MixedGCLiveThresholdPercent=90",
    "-XX:SurvivorRatio=32",
    "-XX:+PerfDisableSharedMem",
    "-XX:MaxTenuringThreshold=1",
];

pub fn build_args(cfg: &ServerConfig) -> Vec<String> {
    let mut args = vec![
        format!("-Xms{}M", cfg.memory.min_mb),
        format!("-Xmx{}M", cfg.memory.max_mb),
    ];
    if cfg.use_aikar_flags {
        args.extend(AIKAR_FLAGS.iter().map(|s| s.to_string()));
    }
    // Consola sin códigos de color/JLine: la leemos por tubería.
    args.push("-Dterminal.jline=false".into());
    args.push("-Dterminal.ansi=false".into());
    // Log4Shell (CVE-2021-44228): un mensaje de chat con `${jndi:...}`
    // ejecutaba código en el servidor. Esto basta con Log4j 2.10 o posterior
    // (Minecraft 1.17 y 1.18); de 1.7 a 1.16 hace falta además otra
    // configuración del registro.
    args.push("-Dlog4j2.formatMsgNoLookups=true".into());
    if needs_log4j_config(&cfg.mc_version) {
        args.push(format!("-Dlog4j.configurationFile={LOG4J_CONFIG}"));
    }
    args.extend(cfg.extra_jvm_args.iter().cloned());
    match &cfg.launch {
        LaunchTarget::Jar { path } => {
            args.push("-jar".into());
            args.push(path.to_string_lossy().into_owned());
        }
        LaunchTarget::ArgsFile { path } => {
            args.push(format!("@{}", path.to_string_lossy()));
        }
    }
    args.push("nogui".into());
    args
}

/// Configuración del registro para Minecraft 1.7 a 1.16 (Log4j 2.0 a 2.8,
/// que no conocen `formatMsgNoLookups`), relativa a la carpeta del servidor.
pub const LOG4J_CONFIG: &str = ".micra/log4j2.xml";

/// La de Minecraft, salvo la ventana del servidor (no se usa), con dos
/// defensas: `%msg{nolookups}` (Log4j 2.7 o posterior) y un filtro que
/// descarta los mensajes con `${...}` antes de darles formato (para Log4j
/// 2.0, donde `nolookups` no existe). Es lo que publicó Mojang.
const LOG4J_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- Escrito por Micra en cada arranque: protege de Log4Shell (CVE-2021-44228). -->
<Configuration status="WARN">
    <Appenders>
        <Console name="SysOut" target="SYSTEM_OUT">
            <PatternLayout pattern="[%d{HH:mm:ss}] [%t/%level]: %msg{nolookups}%n" />
        </Console>
        <RollingRandomAccessFile name="File" fileName="logs/latest.log" filePattern="logs/%d{yyyy-MM-dd}-%i.log.gz">
            <PatternLayout pattern="[%d{HH:mm:ss}] [%t/%level]: %msg{nolookups}%n" />
            <Policies>
                <TimeBasedTriggeringPolicy />
                <OnStartupTriggeringPolicy />
            </Policies>
        </RollingRandomAccessFile>
    </Appenders>
    <Loggers>
        <Root level="info">
            <filters>
                <MarkerFilter marker="NETWORK_PACKETS" onMatch="DENY" onMismatch="NEUTRAL" />
                <RegexFilter regex="(?s).*\$\{[^}]*\}.*" onMatch="DENY" onMismatch="NEUTRAL" />
            </filters>
            <AppenderRef ref="SysOut" />
            <AppenderRef ref="File" />
        </Root>
    </Loggers>
</Configuration>
"#;

/// Minecraft 1.7 a 1.16 trae un Log4j vulnerable que no se arregla con
/// `formatMsgNoLookups` (antes de 1.7 no usaba Log4j).
pub fn needs_log4j_config(mc_version: &str) -> bool {
    use crate::server::version::at_least;
    at_least(mc_version, "1.7") && !at_least(mc_version, "1.17")
}

/// Escribe la configuración del registro si la versión la necesita.
pub async fn write_log4j_config(cfg: &ServerConfig, server_dir: &Path) -> crate::Result<()> {
    if needs_log4j_config(&cfg.mc_version) {
        let path = server_dir.join(LOG4J_CONFIG);
        crate::fsutil::check_no_links(server_dir, &path)?;
        crate::fsutil::write_atomic(&path, LOG4J_XML.as_bytes()).await?;
    }
    Ok(())
}

/// Variables de entorno que llegan al servidor aislado. El resto (la sesión
/// de D-Bus, la pantalla, el agente SSH, tokens que la persona tenga en su
/// terminal, `JAVA_TOOL_OPTIONS`, las de la AppImage...) no le hacen falta y
/// le darían pistas o puertas.
#[cfg(unix)]
const KEEP_ENV: &[&str] = &[
    "PATH",
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    "LC_NUMERIC",
    "LC_TIME",
    "LC_COLLATE",
    "LC_MONETARY",
    "TZ",
    "USER",
    "LOGNAME",
];
#[cfg(windows)]
const KEEP_ENV: &[&str] = &[
    "PATH",
    "PATHEXT",
    // Sin SystemRoot, Windows no deja usar la red.
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "PROCESSOR_IDENTIFIER",
    "PROCESSOR_LEVEL",
    "PROCESSOR_REVISION",
    "OS",
    "USERNAME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "CommonProgramFiles",
    "PUBLIC",
    "ALLUSERSPROFILE",
    "TZ",
];

/// Prepara el `Command` listo para lanzar, con stdin/stdout/stderr en tubería
/// y la afinidad de CPU aplicada en Linux (en Windows se aplica tras el spawn).
pub fn build_command(
    cfg: &ServerConfig,
    server_dir: &Path,
    java: &JavaRuntime,
) -> tokio::process::Command {
    use std::process::Stdio;

    let mut cmd = tokio::process::Command::new(&java.java_bin);
    if cfg.security.sandbox {
        apply_sandbox_env(&mut cmd, server_dir);
        cmd.args(sandbox_jvm_args(server_dir));
        // Sin esto la JVM intenta escribir en /tmp/hsperfdata_<usuario>.
        let perf = "-XX:+PerfDisableSharedMem";
        if !cfg.use_aikar_flags && !cfg.extra_jvm_args.iter().any(|a| a == perf) {
            cmd.arg(perf);
        }
    }
    cmd.args(build_args(cfg))
        .current_dir(server_dir)
        .env("JAVA_HOME", &java.home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    if let Some(cores) = &cfg.cpu.cores {
        super::affinity::apply_before_spawn(&mut cmd, cores);
    }
    cmd
}

/// Con el aislamiento activo, la "carpeta personal" y la temporal del
/// servidor pasan a ser subcarpetas suyas: lo que un mod quiera guardar ahí
/// se queda dentro del servidor (y dentro de lo que Landlock permite). El
/// entorno empieza vacío y solo recibe `KEEP_ENV`.
pub fn apply_sandbox_env(cmd: &mut tokio::process::Command, server_dir: &Path) {
    use crate::server::sandbox::{HOME_DIR, TMP_DIR};

    let home = server_dir.join(HOME_DIR);
    let tmp = server_dir.join(TMP_DIR);
    cmd.env_clear();
    for key in KEEP_ENV {
        if let Some(value) = std::env::var_os(key) {
            cmd.env(key, value);
        }
    }
    #[cfg(unix)]
    {
        cmd.env("HOME", &home)
            .env("TMPDIR", &tmp)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_CACHE_HOME", home.join(".cache"))
            .env("XDG_DATA_HOME", home.join(".local/share"));
    }
    #[cfg(windows)]
    {
        let _ = home;
        cmd.env("TEMP", &tmp).env("TMP", &tmp);
    }
}

/// Opciones de Java que acompañan a `apply_sandbox_env`.
pub fn sandbox_jvm_args(server_dir: &Path) -> Vec<String> {
    use crate::server::sandbox::{HOME_DIR, TMP_DIR};

    vec![
        format!("-Djava.io.tmpdir={}", server_dir.join(TMP_DIR).display()),
        format!("-Duser.home={}", server_dir.join(HOME_DIR).display()),
        // Sin pantalla: que ninguna biblioteca intente abrir ventanas.
        "-Djava.awt.headless=true".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::config::*;

    #[test]
    fn args_for_forge_args_file() {
        let cfg = ServerConfig {
            id: "x".into(),
            name: "x".into(),
            mc_version: "1.21.1".into(),
            loader: Loader::NeoForge {
                version: "21.1.77".into(),
            },
            java_major: 21,
            memory: MemoryConfig {
                min_mb: 1024,
                max_mb: 6144,
            },
            cpu: CpuConfig::default(),
            port: 25565,
            eula_accepted: true,
            launch: LaunchTarget::ArgsFile {
                path: "libraries/x/unix_args.txt".into(),
            },
            use_aikar_flags: false,
            extra_jvm_args: vec![],
            backups: BackupPolicy::default(),
            schedule: SchedulePolicy::default(),
            network: NetworkPolicy::default(),
            security: SecurityPolicy::default(),
            map: MapConfig::default(),
            source: None,
            pending_owner: None,
            world: WorldTweaks::default(),
        };
        let args = build_args(&cfg);
        assert_eq!(args[0], "-Xms1024M");
        assert_eq!(args[1], "-Xmx6144M");
        assert!(args.contains(&"@libraries/x/unix_args.txt".to_string()));
        assert_eq!(args.last().unwrap(), "nogui");
        assert!(args.contains(&"-Dlog4j2.formatMsgNoLookups=true".to_string()));
        assert!(!args
            .iter()
            .any(|a| a.starts_with("-Dlog4j.configurationFile")));
    }

    #[test]
    fn log4shell_by_version() {
        assert!(!needs_log4j_config("1.6.4"));
        assert!(needs_log4j_config("1.7.10"));
        assert!(needs_log4j_config("1.12.2"));
        assert!(needs_log4j_config("1.16.5"));
        assert!(!needs_log4j_config("1.17"));
        assert!(!needs_log4j_config("1.20.1"));
        assert!(!needs_log4j_config("26.1"));
        assert!(!needs_log4j_config("24w14a"));
    }

    #[tokio::test]
    async fn log4j_config_is_written_for_old_versions() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = ServerConfig {
            mc_version: "1.12.2".into(),
            ..serde_json::from_str::<ServerConfig>(
                r#"{"id":"a","name":"A","mcVersion":"1.12.2","loader":{"kind":"vanilla"},
                "javaMajor":8,"memory":{"minMb":1024,"maxMb":2048},"port":25565,
                "eulaAccepted":true,"launch":{"kind":"jar","path":"server.jar"}}"#,
            )
            .unwrap()
        };
        write_log4j_config(&cfg, dir.path()).await.unwrap();
        let xml = std::fs::read_to_string(dir.path().join(LOG4J_CONFIG)).unwrap();
        assert!(xml.contains("%msg{nolookups}"));
        assert!(xml.contains(r#"regex="(?s).*\$\{[^}]*\}.*""#));
        assert!(build_args(&cfg).contains(&format!("-Dlog4j.configurationFile={LOG4J_CONFIG}")));

        let other = tempfile::tempdir().unwrap();
        cfg.mc_version = "1.21.1".into();
        write_log4j_config(&cfg, other.path()).await.unwrap();
        assert!(!other.path().join(LOG4J_CONFIG).exists());
    }
}
