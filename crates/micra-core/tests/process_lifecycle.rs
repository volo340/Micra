//! Prueba el ciclo de vida del proceso con un "java" falso (script de shell)
//! que imita la salida de un servidor de Minecraft.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use micra_core::java::JavaRuntime;
use micra_core::server::config::*;
use micra_core::server::process::{ServerEvent, ServerHandle, ServerStatus};

const FAKE_JAVA: &str = r#"#!/bin/sh
echo "[12:00:00] [Server thread/INFO]: Starting minecraft server"
echo "[12:00:01] [Server thread/INFO]: Done (1.0s)! For help, type \"help\""
while read line; do
  case "$line" in
    say*) echo "[12:00:02] [Server thread/INFO]: Steve joined the game" ;;
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

    let java = JavaRuntime { major: 21, release_name: "fake".into(), home: dir.path().into(), java_bin };
    let cfg = ServerConfig {
        id: "t".into(),
        name: "t".into(),
        mc_version: "1.21.1".into(),
        loader: Loader::Vanilla,
        java_major: 21,
        memory: MemoryConfig { min_mb: 512, max_mb: 1024 },
        cpu: CpuConfig { cores: Some(vec![0]) },
        port: 25565,
        eula_accepted: true,
        launch: LaunchTarget::Jar { path: "server.jar".into() },
        use_aikar_flags: true,
        extra_jvm_args: vec![],
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

    server.stop(Duration::from_secs(5)).await.unwrap();
    assert_eq!(server.status(), ServerStatus::Stopped);
    assert!(server.players().is_empty());
}
