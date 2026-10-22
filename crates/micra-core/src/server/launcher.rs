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
    args.push("-Dterminal.ansi=true".into());
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
pub fn build_command(cfg: &ServerConfig, server_dir: &Path, java: &JavaRuntime) -> tokio::process::Command {
    use std::process::Stdio;

    let mut cmd = tokio::process::Command::new(&java.java_bin);
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
            loader: Loader::NeoForge { version: "21.1.77".into() },
            java_major: 21,
            memory: MemoryConfig { min_mb: 1024, max_mb: 6144 },
            cpu: CpuConfig::default(),
            port: 25565,
            eula_accepted: true,
            launch: LaunchTarget::ArgsFile { path: "libraries/x/unix_args.txt".into() },
            use_aikar_flags: false,
            extra_jvm_args: vec![],
        };
        let args = build_args(&cfg);
        assert_eq!(args[0], "-Xms1024M");
        assert_eq!(args[1], "-Xmx6144M");
        assert!(args.contains(&"@libraries/x/unix_args.txt".to_string()));
        assert_eq!(args.last().unwrap(), "nogui");
    }
}
