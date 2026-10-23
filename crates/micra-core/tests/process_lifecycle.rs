//! Prueba el ciclo de vida del proceso con un "java" falso (script de shell)
//! que imita la salida de un servidor de Minecraft.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use micra_core::java::JavaRuntime;
use micra_core::server::backup::{self, BackupKind};
use micra_core::server::config::*;
use micra_core::server::process::{ServerEvent, ServerHandle, ServerStatus};
use micra_core::server::tps::{self, TpsReading};
use tokio::sync::broadcast::Receiver;

/// `{sys}` es el prefijo de los avisos del sistema: vacío hasta 1.21 y
/// `System chat: ` desde Minecraft 26.3.
const FAKE_JAVA: &str = r#"#!/bin/sh
echo "[12:00:00] [Server thread/INFO]: Starting minecraft server"
echo "[12:00:01] [Server thread/INFO]: Done (1.0s)! For help, type \"help\""
while read line; do
  case "$line" in
    say*)
      echo "[12:00:02] [Server thread/INFO]: Steve[/127.0.0.1:50000] logged in with entity id 7 at ([minecraft:overworld]0.5, 64.0, 0.5)"
      echo "[12:00:02] [Server thread/INFO]: {sys}Steve joined the game"
      echo "[12:00:02] [Server thread/INFO]: <Steve> hola" ;;
    kick*)
      echo "[12:00:02] [Server thread/INFO]: Steve lost connection: Kicked by an operator"
      echo "[12:00:02] [Server thread/INFO]: {sys}Steve left the game"
      echo "[12:00:02] [Server thread/INFO]: {sys}Kicked Steve: Kicked by an operator" ;;
    save-off) echo "[12:00:02] [Server thread/INFO]: {sys}Automatic saving is now disabled" ;;
    "save-all flush")
      echo "[12:00:02] [Server thread/INFO]: {sys}Saving the game (this may take a moment!)"
      echo "[12:00:02] [Server thread/INFO]: {sys}Saved the game" ;;
    save-on) echo "[12:00:02] [Server thread/INFO]: {sys}Automatic saving is now enabled" ;;
    "tick query")
      echo "[12:00:02] [Server thread/INFO]: {sys}The game is running normally"
      echo "[12:00:02] [Server thread/INFO]: {sys}Target tick rate: 20.0 per second."
      echo "[12:00:02] [Server thread/INFO]: {sys}Average time per tick: 62.5ms (Target: 50.0ms)" ;;
    stop) echo "[12:00:03] [Server thread/INFO]: Stopping server"; exit 0 ;;
  esac
done
"#;

#[tokio::test]
async fn launch_command_and_stop() {
    run_server("1.21.1", "").await;
}

#[tokio::test]
async fn launch_command_and_stop_26_3() {
    run_server("26.3", "System chat: ").await;
}

/// Recibe eventos hasta la línea de consola que acaba en `marker` y cuenta
/// las entradas y salidas de jugadores que llegaron antes.
async fn players_until(events: &mut Receiver<ServerEvent>, marker: &str) -> (usize, usize) {
    let (mut joined, mut left) = (0, 0);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await.unwrap() {
                ServerEvent::PlayerJoined { .. } => joined += 1,
                ServerEvent::PlayerLeft { .. } => left += 1,
                ServerEvent::Log { line, .. } if line.ends_with(marker) => break,
                _ => {}
            }
        }
    })
    .await
    .expect("la consola debería mostrar la línea");
    (joined, left)
}

async fn run_server(mc_version: &str, system_prefix: &str) {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let java_bin = bin.join("java");
    std::fs::write(&java_bin, FAKE_JAVA.replace("{sys}", system_prefix)).unwrap();
    std::fs::set_permissions(&java_bin, std::fs::Permissions::from_mode(0o755)).unwrap();

    let java = JavaRuntime {
        major: 21,
        release_name: "fake".into(),
        home: dir.path().into(),
        java_bin,
    };
    let cfg = ServerConfig {
        id: "t".into(),
        name: "t".into(),
        mc_version: mc_version.into(),
        loader: Loader::Vanilla,
        java_major: 21,
        memory: MemoryConfig {
            min_mb: 512,
            max_mb: 1024,
        },
        cpu: CpuConfig {
            cores: Some(vec![0]),
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
    };

    let (server, mut events) = ServerHandle::launch(&cfg, dir.path(), &java).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while server.status() != ServerStatus::Running {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("el servidor debería pasar a Running");
    // La entrada se anuncia dos veces (`logged in` y `joined the game`) y el
    // aviso debe llegar una sola.
    server.send_command("/say hola").await.unwrap();
    assert_eq!(players_until(&mut events, "<Steve> hola").await, (1, 0));
    assert_eq!(server.players(), vec!["Steve".to_string()]);
    server.send_command("kick Steve").await.unwrap();
    assert_eq!(
        players_until(&mut events, "Kicked Steve: Kicked by an operator").await,
        (0, 1)
    );
    assert!(server.players().is_empty());
    server.send_command("/say hola").await.unwrap();
    assert_eq!(players_until(&mut events, "<Steve> hola").await, (1, 0));

    // TPS: la respuesta se intercepta y se interpreta (1000 / 62,5 = 16).
    let command = tps::tps_command(&cfg.loader, &cfg.mc_version).unwrap();
    let lines = server
        .query(command, tps::is_tps_line, Duration::from_millis(500))
        .await
        .unwrap();
    match tps::parse_tps(&lines) {
        TpsReading::Tps(t) => assert!((t - 16.0).abs() < 0.01, "{t}"),
        other => panic!("{other:?} de {lines:?}"),
    }

    // Copia en caliente: save-off, save-all flush, copia y save-on.
    std::fs::create_dir_all(dir.path().join("world/region")).unwrap();
    std::fs::write(dir.path().join("world/level.dat"), b"nivel").unwrap();
    std::fs::write(dir.path().join("world/region/r.0.0.mca"), b"region").unwrap();
    let backups = dir.path().join("backups");
    let info = backup::create_live(
        &server,
        dir.path(),
        &backups,
        &cfg,
        BackupKind::Manual,
        None,
    )
    .await
    .unwrap();
    assert!(backups.join(&info.file_name).exists());
    assert_eq!(backup::list_backups(&backups).await.len(), 1);

    server.stop(Duration::from_secs(5)).await.unwrap();
    assert_eq!(server.status(), ServerStatus::Stopped);
    assert!(server.players().is_empty());
}
