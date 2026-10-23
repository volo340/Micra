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
        apply_sandbox_dirs(&mut cmd, cfg, server_dir);
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
/// se queda dentro del servidor (y dentro de lo que Landlock permite).
fn apply_sandbox_dirs(cmd: &mut tokio::process::Command, cfg: &ServerConfig, server_dir: &Path) {
    use crate::server::sandbox::{HOME_DIR, TMP_DIR};

    let home = server_dir.join(HOME_DIR);
    let tmp = server_dir.join(TMP_DIR);
    cmd.arg(format!("-Djava.io.tmpdir={}", tmp.display()))
        .arg(format!("-Duser.home={}", home.display()));
    // Sin esto la JVM intenta escribir en /tmp/hsperfdata_<usuario>.
    let perf = "-XX:+PerfDisableSharedMem";
    if !cfg.use_aikar_flags && !cfg.extra_jvm_args.iter().any(|a| a == perf) {
        cmd.arg(perf);
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
        cmd.env("TEMP", &tmp).env("TMP", &tmp);
    }
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
    }
}
