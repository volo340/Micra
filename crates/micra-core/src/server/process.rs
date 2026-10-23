//! Proceso del servidor en ejecución: consola en vivo, comandos, jugadores
//! conectados y apagado ordenado.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::{broadcast, mpsc, oneshot, watch, Mutex};

use crate::java::JavaRuntime;
use crate::server::config::ServerConfig;
use crate::server::launcher::build_command;
use crate::server::sandbox::SandboxStatus;
use crate::{tr, Error, Result};

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

/// Por qué el servidor rechazó a alguien que intentaba entrar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RejectReason {
    NotWhitelisted,
    Banned,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerEvent {
    Log { stream: LogStream, line: String },
    Status { status: ServerStatus },
    PlayerJoined { name: String },
    PlayerLeft { name: String },
    JoinRejected { name: String, reason: RejectReason },
    Exited { code: Option<i32> },
}

/// Captura de líneas de la consola para comandos internos (p. ej. medir los
/// TPS): las líneas que encajan se entregan a quien preguntó y no aparecen en
/// la consola de la interfaz.
struct Interceptor {
    matcher: Box<dyn Fn(&str) -> bool + Send + Sync>,
    deadline: Instant,
    tx: mpsc::UnboundedSender<String>,
}

type Interceptors = Arc<StdMutex<Vec<Interceptor>>>;

pub struct ServerHandle {
    pid: Option<u32>,
    stdin: Mutex<Option<ChildStdin>>,
    events: broadcast::Sender<ServerEvent>,
    status: watch::Receiver<ServerStatus>,
    status_tx: Arc<watch::Sender<ServerStatus>>,
    players: Arc<StdMutex<BTreeSet<String>>>,
    kill: StdMutex<Option<oneshot::Sender<()>>>,
    interceptors: Interceptors,
    sandbox: SandboxStatus,
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
        let sandboxed = cfg.security.sandbox;
        if sandboxed {
            for dir in [
                crate::server::sandbox::HOME_DIR,
                crate::server::sandbox::TMP_DIR,
            ] {
                tokio::fs::create_dir_all(server_dir.join(dir)).await?;
            }
        }
        let mut cmd = build_command(cfg, server_dir, java);
        let (mut child, sandbox, guard) = spawn_sandboxed(&mut cmd, cfg, server_dir, java)?;
        if let Some(cores) = &cfg.cpu.cores {
            if let Err(e) = super::affinity::apply_after_spawn(&child, cores) {
                tracing::warn!("no se pudo fijar la afinidad de CPU: {e}");
            }
        }

        let (events, first_rx) = broadcast::channel(1024);
        let (status_tx, status) = watch::channel(ServerStatus::Starting);
        let status_tx = Arc::new(status_tx);
        let players = Arc::new(StdMutex::new(BTreeSet::new()));
        let interceptors: Interceptors = Arc::new(StdMutex::new(Vec::new()));
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
                interceptors.clone(),
            ),
            spawn_reader(
                stderr,
                LogStream::Stderr,
                events.clone(),
                status_tx.clone(),
                players.clone(),
                interceptors.clone(),
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
                // El Job Object (Windows) debe vivir tanto como el proceso.
                let _guard = guard;
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
            interceptors,
            sandbox,
        };
        Ok((handle, first_rx))
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn status(&self) -> ServerStatus {
        *self.status.borrow()
    }

    /// Para esperar cambios de estado (p. ej. a que pase a "en línea").
    pub fn status_watch(&self) -> watch::Receiver<ServerStatus> {
        self.status.clone()
    }

    pub fn is_alive(&self) -> bool {
        !matches!(self.status(), ServerStatus::Stopped | ServerStatus::Crashed)
    }

    pub fn players(&self) -> Vec<String> {
        self.players.lock().unwrap().iter().cloned().collect()
    }

    pub fn sandbox(&self) -> &SandboxStatus {
        &self.sandbox
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.events.subscribe()
    }

    /// Escribe un comando en la consola del servidor (sin la barra inicial).
    /// Los saltos de línea se rechazan: cada llamada es un único comando.
    pub async fn send_command(&self, command: &str) -> Result<()> {
        if !self.is_alive() {
            return Err(Error::NotRunning);
        }
        let line = command.trim().trim_start_matches('/');
        if line.contains(['\n', '\r']) {
            return Err(Error::InvalidConfig(tr!(
                "un comando no puede tener saltos de línea",
                "a command cannot contain line breaks"
            )));
        }
        let mut guard = self.stdin.lock().await;
        let stdin = guard.as_mut().ok_or(Error::NotRunning)?;
        stdin.write_all(format!("{line}\n").as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Envía un comando interno y recoge durante `window` las líneas de
    /// respuesta que reconoce `matcher` (texto tras `[hora] [hilo/INFO]: `).
    /// Esas líneas no se muestran en la consola de la interfaz.
    pub async fn query(
        &self,
        command: &str,
        matcher: impl Fn(&str) -> bool + Send + Sync + 'static,
        window: Duration,
    ) -> Result<Vec<String>> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let deadline = Instant::now() + window;
        self.interceptors.lock().unwrap().push(Interceptor {
            matcher: Box::new(matcher),
            deadline,
            tx,
        });
        self.send_command(command).await?;
        let mut lines = Vec::new();
        let until = tokio::time::Instant::from_std(deadline);
        while let Ok(Some(line)) = tokio::time::timeout_at(until, rx.recv()).await {
            lines.push(line);
        }
        Ok(lines)
    }

    /// Envía un comando y espera a que aparezca en la consola una línea que
    /// cumpla `pred` (sin ocultarla). Devuelve `false` si se agota el tiempo.
    pub async fn command_and_wait(
        &self,
        command: &str,
        pred: impl Fn(&str) -> bool,
        timeout: Duration,
    ) -> Result<bool> {
        let mut rx = self.subscribe();
        self.send_command(command).await?;
        let wait = async {
            loop {
                match rx.recv().await {
                    Ok(ServerEvent::Log { line, .. }) if pred(message_of(&line)) => return true,
                    Ok(ServerEvent::Exited { .. }) | Err(broadcast::error::RecvError::Closed) => {
                        return false
                    }
                    _ => {}
                }
            }
        };
        Ok(tokio::time::timeout(timeout, wait).await.unwrap_or(false))
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

/// Lo que hay que mantener vivo mientras corre el servidor (en Windows, el
/// Job Object).
type SandboxGuard = Option<Box<dyn Send + Sync>>;

#[cfg(target_os = "linux")]
fn spawn_sandboxed(
    cmd: &mut tokio::process::Command,
    cfg: &ServerConfig,
    server_dir: &Path,
    java: &JavaRuntime,
) -> Result<(tokio::process::Child, SandboxStatus, SandboxGuard)> {
    use crate::server::sandbox::linux;

    if !cfg.security.sandbox {
        return Ok((cmd.spawn()?, SandboxStatus::Disabled, None));
    }
    match linux::prepare(server_dir, &java.home) {
        Ok(Some(prepared)) => {
            linux::apply_pre_exec(cmd, &prepared);
            let child = cmd.spawn()?;
            let abi = prepared.abi;
            drop(prepared);
            Ok((child, SandboxStatus::Landlock { abi }, None))
        }
        Ok(None) => Ok((
            cmd.spawn()?,
            SandboxStatus::Unavailable {
                reason: tr!(
                    "este kernel no tiene Landlock (hace falta Linux 5.13 o posterior con Landlock activado)",
                    "this kernel has no Landlock (Linux 5.13 or later with Landlock enabled is needed)"
                ),
            },
            None,
        )),
        Err(e) => {
            tracing::warn!("no se pudo preparar Landlock: {e}");
            Ok((
                cmd.spawn()?,
                SandboxStatus::Unavailable {
                    reason: e.to_string(),
                },
                None,
            ))
        }
    }
}

#[cfg(windows)]
fn spawn_sandboxed(
    cmd: &mut tokio::process::Command,
    cfg: &ServerConfig,
    server_dir: &Path,
    _java: &JavaRuntime,
) -> Result<(tokio::process::Child, SandboxStatus, SandboxGuard)> {
    use crate::server::sandbox::windows as win;
    use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

    let low = cfg.security.sandbox && cfg.security.low_integrity;
    if low {
        win::label_low_integrity(server_dir)?;
        cmd.creation_flags(crate::server::launcher::CREATE_NO_WINDOW | CREATE_SUSPENDED);
    }
    let child = cmd.spawn()?;
    // Con el proceso suspendido, cualquier fallo debe matarlo: nunca puede
    // quedarse un servidor congelado y sin aislar.
    let setup = || -> std::io::Result<(SandboxStatus, SandboxGuard)> {
        let handle = child
            .raw_handle()
            .ok_or_else(|| std::io::Error::other("el proceso ya terminó"))?
            as windows_sys::Win32::Foundation::HANDLE;
        let job = win::create_job()?;
        // SAFETY: `handle` es el del hijo, que sigue vivo (lo tenemos en `child`).
        unsafe { win::assign(&job, handle)? };
        if low {
            // SAFETY: igual que arriba.
            unsafe { win::lower_integrity(handle)? };
            win::resume_process(child.id().unwrap_or_default())?;
        }
        let status = if cfg.security.sandbox {
            SandboxStatus::JobObject { low_integrity: low }
        } else {
            SandboxStatus::Disabled
        };
        Ok((status, Some(Box::new(job) as Box<dyn Send + Sync>)))
    };
    match setup() {
        Ok((status, guard)) => Ok((child, status, guard)),
        Err(e) if low => {
            let mut child = child;
            let _ = child.start_kill();
            Err(Error::Api(tr!(
                "no se pudo aislar el servidor ({e}). Desactiva la integridad baja en Ajustes",
                "the server could not be isolated ({e}). Turn off low integrity in Settings"
            )))
        }
        Err(e) => {
            tracing::warn!("no se pudo crear el Job Object: {e}");
            Ok((
                child,
                SandboxStatus::Unavailable {
                    reason: e.to_string(),
                },
                None,
            ))
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
fn spawn_sandboxed(
    cmd: &mut tokio::process::Command,
    _cfg: &ServerConfig,
    _server_dir: &Path,
    _java: &JavaRuntime,
) -> Result<(tokio::process::Child, SandboxStatus, SandboxGuard)> {
    Ok((
        cmd.spawn()?,
        SandboxStatus::Unavailable {
            reason: "plataforma no soportada".into(),
        },
        None,
    ))
}

fn spawn_reader(
    pipe: impl AsyncRead + Unpin + Send + 'static,
    stream: LogStream,
    events: broadcast::Sender<ServerEvent>,
    status_tx: Arc<watch::Sender<ServerStatus>>,
    players: Arc<StdMutex<BTreeSet<String>>>,
    interceptors: Interceptors,
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
            if intercept(&interceptors, &line) {
                continue;
            }
            match parse_line(&line) {
                Some(LineInfo::Ready) => {
                    if *status_tx.borrow() == ServerStatus::Starting {
                        status_tx.send_replace(ServerStatus::Running);
                        let _ = events.send(ServerEvent::Status {
                            status: ServerStatus::Running,
                        });
                    }
                }
                // Minecraft avisa más de una vez de cada entrada y salida
                // (`logged in` y `joined the game`): solo cuenta la primera.
                Some(LineInfo::Joined(name)) => {
                    if players.lock().unwrap().insert(name.clone()) {
                        let _ = events.send(ServerEvent::PlayerJoined { name });
                    }
                }
                Some(LineInfo::Left(name)) => {
                    if players.lock().unwrap().remove(&name) {
                        let _ = events.send(ServerEvent::PlayerLeft { name });
                    }
                }
                Some(LineInfo::Rejected(name, reason)) => {
                    let _ = events.send(ServerEvent::JoinRejected { name, reason });
                }
                None => {}
            }
            let _ = events.send(ServerEvent::Log { stream, line });
        }
    })
}

/// Entrega la línea al primer interceptor vigente que la reconozca.
fn intercept(interceptors: &Interceptors, line: &str) -> bool {
    let mut list = interceptors.lock().unwrap();
    if list.is_empty() {
        return false;
    }
    let now = Instant::now();
    list.retain(|i| i.deadline > now && !i.tx.is_closed());
    let msg = message_of(line);
    if let Some(i) = list.iter().find(|i| (i.matcher)(msg)) {
        let _ = i.tx.send(msg.to_string());
        return true;
    }
    false
}

/// Prefijo que Minecraft 26.3 y posteriores ponen en el log a los mensajes del
/// sistema: entradas y salidas, muertes, respuestas a comandos...
const SYSTEM_CHAT: &str = "System chat: ";

/// Texto de una línea del log sin el prefijo `[hora] [hilo/NIVEL]: ` ni el
/// `System chat: ` de las versiones nuevas.
pub fn message_of(line: &str) -> &str {
    let msg = line.split_once("]: ").map_or(line, |(_, m)| m).trim();
    msg.strip_prefix(SYSTEM_CHAT).unwrap_or(msg)
}

#[derive(Debug, PartialEq, Eq)]
enum LineInfo {
    Ready,
    Joined(String),
    Left(String),
    Rejected(String, RejectReason),
}

fn valid_name(name: &str) -> Option<String> {
    let name = name.trim();
    let valid = !name.is_empty()
        && name.len() <= 16
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    valid.then(|| name.to_string())
}

/// Interpreta las líneas estándar del log de Minecraft, p. ej.:
/// `[12:00:00] [Server thread/INFO]: Done (3.2s)! For help, type "help"`
/// `[12:00:05] [Server thread/INFO]: Steve joined the game`
/// `[12:00:05] [Server thread/INFO]: System chat: Steve joined the game` (26.3+)
fn parse_line(line: &str) -> Option<LineInfo> {
    let msg = message_of(line);
    if msg.starts_with("Done (") && msg.contains("For help") {
        return Some(LineInfo::Ready);
    }
    if let Some(rest) = msg.strip_suffix(" joined the game") {
        // `Steve (formerly known as Steve2) joined the game` tras cambiar de nombre.
        let name = rest
            .strip_suffix(')')
            .and_then(|r| r.split_once(" (formerly known as "))
            .map_or(rest, |(name, _)| name);
        return valid_name(name).map(LineInfo::Joined);
    }
    if let Some(rest) = msg.strip_suffix(" left the game") {
        return valid_name(rest).map(LineInfo::Left);
    }
    parse_rejection(msg).or_else(|| parse_connection(msg))
}

/// Líneas que el servidor escribe al entrar y salir aunque no lo anuncie en el
/// chat (hay mods y plugins que ocultan esos avisos):
/// `Steve[/1.2.3.4:5] logged in with entity id 7 at (0.5, 64.0, 0.5)`
/// `Steve lost connection: Disconnected`
fn parse_connection(msg: &str) -> Option<LineInfo> {
    if let Some((head, _)) = msg.split_once("] logged in with entity id ") {
        let (name, _) = head.split_once('[')?;
        return valid_name(name).map(LineInfo::Joined);
    }
    // Quien aún no había entrado sale como `Steve (/1.2.3.4:5) lost connection`
    // o `/1.2.3.4:5 lost connection`, que no son nombres válidos.
    let (name, _) = msg.split_once(" lost connection: ")?;
    valid_name(name).map(LineInfo::Left)
}

/// Intentos de entrada rechazados, en los formatos de distintas versiones:
/// `Disconnecting Steve (/1.2.3.4:5): You are not white-listed on this server!`
/// `Steve (/1.2.3.4:5) lost connection: You are banned from this server.`
/// `Disconnecting com.mojang.authlib.GameProfile@1a[id=<null>,name=Steve,...] (/1.2.3.4:5): ...`
fn parse_rejection(msg: &str) -> Option<LineInfo> {
    let lower = msg.to_ascii_lowercase();
    let reason = if lower.contains("not white-listed") || lower.contains("not whitelisted") {
        RejectReason::NotWhitelisted
    } else if lower.contains("you are banned") {
        RejectReason::Banned
    } else {
        return None;
    };
    let name = if let Some(i) = msg.find("name=") {
        let rest = &msg[i + 5..];
        rest.split([',', ']']).next().unwrap_or_default()
    } else {
        let head = msg.split(" (/").next().unwrap_or_default();
        head.strip_prefix("Disconnecting ").unwrap_or(head)
    };
    valid_name(name).map(|n| LineInfo::Rejected(n, reason))
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

    #[test]
    fn parses_system_chat_lines() {
        // Minecraft 26.3 antepone `System chat: ` a los avisos del servidor.
        assert_eq!(
            parse_line("[09:06:29] [Server thread/INFO]: System chat: Mokkq joined the game"),
            Some(LineInfo::Joined("Mokkq".into()))
        );
        assert_eq!(
            parse_line("[09:10:02] [Server thread/INFO]: System chat: Mokkq left the game"),
            Some(LineInfo::Left("Mokkq".into()))
        );
        assert_eq!(
            parse_line(
                "[09:06:29] [Server thread/INFO]: System chat: Alex (formerly known as Alex2) joined the game"
            ),
            Some(LineInfo::Joined("Alex".into()))
        );
        assert_eq!(
            message_of("[09:11:00] [Server thread/INFO]: System chat: Saved the game"),
            "Saved the game"
        );
        // Ni el chat de un jugador ni un `say` pueden hacerse pasar por el sistema.
        assert_eq!(
            parse_line(
                "[09:12:00] [Server thread/INFO]: <Steve> System chat: Alex joined the game"
            ),
            None
        );
        assert_eq!(
            parse_line("[09:12:00] [Server thread/INFO]: [Steve] System chat: Alex left the game"),
            None
        );
    }

    #[test]
    fn parses_connection_lines() {
        assert_eq!(
            parse_line("[09:06:29] [Server thread/INFO]: Mokkq[/192.168.1.20:51234] logged in with entity id 42 at ([minecraft:overworld]-3.5, 64.0, 12.5)"),
            Some(LineInfo::Joined("Mokkq".into()))
        );
        assert_eq!(
            parse_line("[09:06:29] [Server thread/INFO]: Alex[/[2001:db8::1]:4000] logged in with entity id 1 at (0.0, 64.0, 0.0)"),
            Some(LineInfo::Joined("Alex".into()))
        );
        assert_eq!(
            parse_line("[09:10:02] [Server thread/INFO]: Mokkq lost connection: Disconnected"),
            Some(LineInfo::Left("Mokkq".into()))
        );
        // Conexiones que nunca llegaron a entrar en el mundo.
        assert_eq!(
            parse_line("[09:10:02] [Server thread/INFO]: /192.168.1.20:51234 lost connection: Disconnected"),
            None
        );
        assert_eq!(
            parse_line("[09:10:02] [Server thread/INFO]: Steve (/192.168.1.20:51234) lost connection: Disconnected"),
            None
        );
        assert_eq!(
            parse_line("[09:10:02] [Server thread/INFO]: <Steve> Alex lost connection: jaja"),
            None
        );
        assert_eq!(
            parse_line("[09:10:02] [Server thread/INFO]: <Steve> Alex[/1.2.3.4:5] logged in with entity id 1 at (0, 0, 0)"),
            None
        );
    }

    #[test]
    fn parses_rejections() {
        assert_eq!(
            parse_line("[12:00:00] [Server thread/INFO]: Disconnecting Steve (/192.168.1.20:51234): You are not white-listed on this server!"),
            Some(LineInfo::Rejected("Steve".into(), RejectReason::NotWhitelisted))
        );
        assert_eq!(
            parse_line("[12:00:00] [Server thread/INFO]: Alex (/10.0.0.2:4000) lost connection: You are not whitelisted on this server!"),
            Some(LineInfo::Rejected("Alex".into(), RejectReason::NotWhitelisted))
        );
        assert_eq!(
            parse_line("[12:00:00] [User Authenticator #1/INFO]: Disconnecting com.mojang.authlib.GameProfile@5f2a1b[id=069a79f4-44e9-4726-a5be-fca90e38aaf5,name=Notch,properties={},legacy=false] (/1.2.3.4:5): You are banned from this server."),
            Some(LineInfo::Rejected("Notch".into(), RejectReason::Banned))
        );
        // Un jugador que escribe la frase en el chat no cuenta.
        assert_eq!(
            parse_line("[12:00:00] [Server thread/INFO]: <Steve> you are not white-listed lol"),
            None
        );
    }

    #[test]
    fn interceptor_consumes_matching_lines() {
        let interceptors: Interceptors = Arc::new(StdMutex::new(Vec::new()));
        let (tx, mut rx) = mpsc::unbounded_channel();
        interceptors.lock().unwrap().push(Interceptor {
            matcher: Box::new(|m| m.starts_with("Average time per tick")),
            deadline: Instant::now() + Duration::from_secs(5),
            tx,
        });
        assert!(intercept(
            &interceptors,
            "Average time per tick: 3.2ms (Target: 50.0ms)"
        ));
        assert!(!intercept(
            &interceptors,
            "[12:00:00] [Server thread/INFO]: Steve joined the game"
        ));
        assert_eq!(
            rx.try_recv().unwrap(),
            "Average time per tick: 3.2ms (Target: 50.0ms)"
        );
    }
}
