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

const FAKE_JAVA: &str = r#"#!/bin/sh
echo "[12:00:00] [Server thread/INFO]: Starting minecraft server"
echo "[12:00:01] [Server thread/INFO]: Done (1.0s)! For help, type \"help\""
while read line; do
  case "$line" in
    say*) echo "[12:00:02] [Server thread/INFO]: Steve joined the game" ;;
    save-off) echo "[12:00:02] [Server thread/INFO]: Automatic saving is now disabled" ;;
    "save-all flush")
      echo "[12:00:02] [Server thread/INFO]: Saving the game (this may take a moment!)"
      echo "[12:00:02] [Server thread/INFO]: Saved the game" ;;
    save-on) echo "[12:00:02] [Server thread/INFO]: Automatic saving is now enabled" ;;
    "tick query")
      echo "[12:00:02] [Server thread/INFO]: The game is running normally"
      echo "[12:00:02] [Server thread/INFO]: Target tick rate: 20.0 per second."
      echo "[12:00:02] [Server thread/INFO]: Average time per tick: 62.5ms (Target: 50.0ms)" ;;
    stop) echo "[12:00:03] [Server thread/INFO]: Stopping server"; exit 0 ;;
  esac
done
"#;

#[tokio::test]
async fn launch_command_and_stop() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let java_bin = bin.join("java");
    std::fs::write(&java_bin, FAKE_JAVA).unwrap();
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
        mc_version: "1.21.1".into(),
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
    server.send_command("/say hola").await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !matches!(events.recv().await, Ok(ServerEvent::PlayerJoined { .. })) {}
    })
    .await
    .unwrap();
    assert_eq!(server.players(), vec!["Steve".to_string()]);

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
