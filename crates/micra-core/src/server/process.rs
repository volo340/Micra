//! Proceso del servidor en ejecución: consola en vivo, comandos, jugadores
//! conectados y apagado ordenado.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::{broadcast, oneshot, watch, Mutex};

use crate::java::JavaRuntime;
use crate::server::config::ServerConfig;
use crate::server::launcher::build_command;
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerStatus {
    Starting,
    Running,
    Stopping,
    Stopped,
    Crashed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerEvent {
    Log { stream: LogStream, line: String },
    Status { status: ServerStatus },
    PlayerJoined { name: String },
    PlayerLeft { name: String },
    Exited { code: Option<i32> },
}

pub struct ServerHandle {
    pid: Option<u32>,
    stdin: Mutex<Option<ChildStdin>>,
    events: broadcast::Sender<ServerEvent>,
    status: watch::Receiver<ServerStatus>,
    status_tx: Arc<watch::Sender<ServerStatus>>,
    players: Arc<StdMutex<BTreeSet<String>>>,
    kill: StdMutex<Option<oneshot::Sender<()>>>,
}

impl ServerHandle {
    /// Lanza el servidor. Devuelve también un receptor creado antes de leer
    /// la salida, para no perder las primeras líneas de la consola; más
    /// oyentes se pueden añadir con `subscribe`.
    pub async fn launch(
        cfg: &ServerConfig,
        server_dir: &Path,
        java: &JavaRuntime,
    ) -> Result<(Self, broadcast::Receiver<ServerEvent>)> {
        let mut child = build_command(cfg, server_dir, java).spawn()?;
        if let Some(cores) = &cfg.cpu.cores {
            if let Err(e) = super::affinity::apply_after_spawn(&child, cores) {
                tracing::warn!("no se pudo fijar la afinidad de CPU: {e}");
            }
        }

        let (events, first_rx) = broadcast::channel(1024);
        let (status_tx, status) = watch::channel(ServerStatus::Starting);
        let status_tx = Arc::new(status_tx);
        let players = Arc::new(StdMutex::new(BTreeSet::new()));
        let (kill_tx, kill_rx) = oneshot::channel::<()>();

        let stdout = child.stdout.take().expect("stdout en tubería");
        let stderr = child.stderr.take().expect("stderr en tubería");
        let readers = [
            spawn_reader(
                stdout,
                LogStream::Stdout,
                events.clone(),
                status_tx.clone(),
                players.clone(),
            ),
            spawn_reader(
                stderr,
                LogStream::Stderr,
                events.clone(),
                status_tx.clone(),
                players.clone(),
            ),
        ];

        let _ = events.send(ServerEvent::Status {
            status: ServerStatus::Starting,
        });
        let pid = child.id();
        let stdin = child.stdin.take();

        // Tarea dueña del proceso: espera a que termine o a la orden de matarlo.
        {
            let events = events.clone();
            let status_tx = status_tx.clone();
            let players = players.clone();
            tokio::spawn(async move {
                let exit = tokio::select! {
                    res = child.wait() => res,
                    _ = kill_rx => {
                        let _ = child.kill().await;
                        child.wait().await
                    }
                };
                // Vaciar la salida pendiente antes de anunciar el cierre.
                for r in readers {
                    let _ = r.await;
                }
                let code = exit.ok().and_then(|s| s.code());
                let stopping = *status_tx.borrow() == ServerStatus::Stopping;
                let final_status = if stopping || code == Some(0) {
                    ServerStatus::Stopped
                } else {
                    ServerStatus::Crashed
                };
                players.lock().unwrap().clear();
                status_tx.send_replace(final_status);
                let _ = events.send(ServerEvent::Exited { code });
                let _ = events.send(ServerEvent::Status {
                    status: final_status,
                });
            });
        }

        let handle = Self {
            pid,
            stdin: Mutex::new(stdin),
            events,
            status,
            status_tx,
            players,
            kill: StdMutex::new(Some(kill_tx)),
        };
        Ok((handle, first_rx))
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn status(&self) -> ServerStatus {
        *self.status.borrow()
    }

    pub fn is_alive(&self) -> bool {
        !matches!(self.status(), ServerStatus::Stopped | ServerStatus::Crashed)
    }

    pub fn players(&self) -> Vec<String> {
        self.players.lock().unwrap().iter().cloned().collect()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.events.subscribe()
    }

    /// Escribe un comando en la consola del servidor (sin la barra inicial).
    pub async fn send_command(&self, command: &str) -> Result<()> {
        if !self.is_alive() {
            return Err(Error::NotRunning);
        }
        let mut guard = self.stdin.lock().await;
        let stdin = guard.as_mut().ok_or(Error::NotRunning)?;
        let line = command.trim().trim_start_matches('/');
        stdin.write_all(format!("{line}\n").as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Apagado ordenado: envía `stop` (guarda el mundo) y, si no termina en
    /// `timeout`, mata el proceso.
    pub async fn stop(&self, timeout: Duration) -> Result<()> {
        if !self.is_alive() {
            return Ok(());
        }
        self.status_tx.send_replace(ServerStatus::Stopping);
        let _ = self.events.send(ServerEvent::Status {
            status: ServerStatus::Stopping,
        });
        if self.send_command("stop").await.is_err() {
            self.kill();
        }
        let mut status = self.status.clone();
        let finished =
            status.wait_for(|s| matches!(s, ServerStatus::Stopped | ServerStatus::Crashed));
        if tokio::time::timeout(timeout, finished).await.is_err() {
            tracing::warn!("el servidor no respondió a `stop`; forzando cierre");
            self.kill();
        }
        Ok(())
    }

    pub fn kill(&self) {
        if let Some(tx) = self.kill.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }
}

fn spawn_reader(
    pipe: impl AsyncRead + Unpin + Send + 'static,
    stream: LogStream,
    events: broadcast::Sender<ServerEvent>,
    status_tx: Arc<watch::Sender<ServerStatus>>,
    players: Arc<StdMutex<BTreeSet<String>>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut reader = BufReader::new(pipe);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            // En Windows la consola de Java puede no ser UTF-8: mejor perder
            // algún carácter que perder la línea.
            let line = String::from_utf8_lossy(&buf).trim_end().to_string();
            match parse_line(&line) {
                Some(LineInfo::Ready) => {
                    if *status_tx.borrow() == ServerStatus::Starting {
                        status_tx.send_replace(ServerStatus::Running);
                        let _ = events.send(ServerEvent::Status {
                            status: ServerStatus::Running,
                        });
                    }
                }
                Some(LineInfo::Joined(name)) => {
                    players.lock().unwrap().insert(name.clone());
                    let _ = events.send(ServerEvent::PlayerJoined { name });
                }
                Some(LineInfo::Left(name)) => {
                    players.lock().unwrap().remove(&name);
                    let _ = events.send(ServerEvent::PlayerLeft { name });
                }
                None => {}
            }
            let _ = events.send(ServerEvent::Log { stream, line });
        }
    })
}

#[derive(Debug, PartialEq, Eq)]
enum LineInfo {
    Ready,
    Joined(String),
    Left(String),
}

/// Interpreta las líneas estándar del log de Minecraft, p. ej.:
/// `[12:00:00] [Server thread/INFO]: Done (3.2s)! For help, type "help"`
/// `[12:00:05] [Server thread/INFO]: Steve joined the game`
fn parse_line(line: &str) -> Option<LineInfo> {
    let msg = line.split_once("]: ").map_or(line, |(_, m)| m).trim();
    if msg.starts_with("Done (") && msg.contains("For help") {
        return Some(LineInfo::Ready);
    }
    let name_of = |rest: &str| {
        let name = rest.trim();
        let valid = !name.is_empty()
            && name.len() <= 16
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        valid.then(|| name.to_string())
    };
    if let Some(rest) = msg.strip_suffix(" joined the game") {
        return name_of(rest).map(LineInfo::Joined);
    }
    if let Some(rest) = msg.strip_suffix(" left the game") {
        return name_of(rest).map(LineInfo::Left);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_log_lines() {
        assert_eq!(
            parse_line(r#"[12:00:00] [Server thread/INFO]: Done (3.214s)! For help, type "help""#),
            Some(LineInfo::Ready)
        );
        assert_eq!(
            parse_line("[12:00:05] [Server thread/INFO]: Steve_01 joined the game"),
            Some(LineInfo::Joined("Steve_01".into()))
        );
        assert_eq!(
            parse_line(
                "[12:01:00] [Server thread/INFO] [minecraft/MinecraftServer]: Alex left the game"
            ),
            Some(LineInfo::Left("Alex".into()))
        );
        // Un mensaje de chat no debe confundirse con una entrada.
        assert_eq!(
            parse_line("[12:02:00] [Server thread/INFO]: <Steve> hola joined the game"),
            None
        );
    }
}
