//! Túnel con [playit.gg](https://playit.gg) para quien no puede abrir puertos
//! (CGNAT, routers sin UPnP, redes de residencias o de empresa).
//!
//! 1. La persona vincula Micra con su cuenta de playit.gg: Micra genera un
//!    código, abre `https://playit.gg/claim/<código>` en el navegador y espera
//!    a que lo acepte. playit.gg entrega la clave del agente, que se guarda en
//!    el llavero del sistema.
//! 2. Por cada servidor con el túnel activado, Micra crea (una vez) un túnel
//!    «minecraft-java» hacia `127.0.0.1:<puerto>` y muestra su dirección.
//! 3. Mientras haya algún servidor con túnel en marcha, Micra ejecuta el
//!    agente oficial (`playitd`), descargado de las versiones publicadas en
//!    GitHub y comprobado con el SHA-256 que publica GitHub.
//!
//! Solo se publica el puerto del juego. Los jugadores que entran por el túnel
//! llegan desde `127.0.0.1`, así que el bloqueo por IP no puede distinguirlos:
//! la lista blanca sigue siendo la protección principal.

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::download::{download_file, hash_file, ExpectedHash, HashAlgo};
use crate::fsutil::{write_atomic_sync, write_private_sync};
use crate::{tr, Error, Result};

const API: &str = "https://api.playit.gg";
const RELEASES: &str = "https://api.github.com/repos/playit-cloud/playit-agent/releases/latest";
const DOWNLOAD_PREFIX: &str = "https://github.com/playit-cloud/playit-agent/releases/download/";
/// Cada cuánto buscar una versión nueva del agente.
const UPDATE_CHECK: chrono::Duration = chrono::Duration::days(7);
const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

// ---------------------------------------------------------------------------
// API de playit.gg
// ---------------------------------------------------------------------------

/// Respuesta de la API: `{"status": "success" | "fail" | "error", "data": ...}`.
#[derive(Deserialize)]
#[serde(tag = "status", content = "data", rename_all = "lowercase")]
enum ApiReply<T> {
    Success(T),
    Fail(Value),
    Error(Value),
}

/// Resultado de una llamada: éxito o el motivo («fail») que da la API.
enum Reply<T> {
    Ok(T),
    Fail(String),
}

async fn call<T: DeserializeOwned>(
    http: &reqwest::Client,
    secret: Option<&str>,
    path: &str,
    body: Value,
) -> Result<Reply<T>> {
    let mut req = http
        .post(format!("{API}{path}"))
        .timeout(Duration::from_secs(20))
        .json(&body);
    if let Some(secret) = secret {
        req = req.header(
            reqwest::header::AUTHORIZATION,
            format!("Agent-Key {secret}"),
        );
    }
    let res = req.send().await?;
    let status = res.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(Error::Api(tr!(
            "playit.gg pide esperar un poco antes de volver a intentarlo",
            "playit.gg asks to wait a bit before trying again"
        )));
    }
    let text = res.text().await?;
    let reply: ApiReply<T> = serde_json::from_str(&text).map_err(|_| {
        Error::Api(tr!(
            "respuesta inesperada de playit.gg ({status})",
            "unexpected answer from playit.gg ({status})"
        ))
    })?;
    match reply {
        ApiReply::Success(data) => Ok(Reply::Ok(data)),
        ApiReply::Fail(reason) => Ok(Reply::Fail(reason_text(&reason))),
        ApiReply::Error(error) => Err(api_error(&error)),
    }
}

fn reason_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Errores generales: `{"type": "auth" | "validation" | ..., "message": ...}`.
fn api_error(error: &Value) -> Error {
    let kind = error.get("type").and_then(Value::as_str).unwrap_or("");
    let message = error.get("message").map(reason_text).unwrap_or_default();
    if kind == "auth" {
        return Error::Api(tr!(
            "playit.gg no acepta la clave de este equipo ({message}). Vuelve a vincular la cuenta en Ajustes",
            "playit.gg does not accept this computer's key ({message}). Link the account again in Settings"
        ));
    }
    Error::Api(tr!(
        "playit.gg devolvió un error: {kind} {message}",
        "playit.gg returned an error: {kind} {message}"
    ))
}

fn fail(path: &str, reason: &str) -> Error {
    let detail = match reason {
        "RequiresVerifiedAccount" => tr!(
            "verifica el correo de tu cuenta de playit.gg",
            "verify the email of your playit.gg account"
        ),
        "RequiresPlayitPremium" => tr!(
            "esta opción necesita playit.gg Premium",
            "this option needs playit.gg Premium"
        ),
        "AgentVersionTooOld" => tr!(
            "el agente es demasiado antiguo; reinicia Micra para actualizarlo",
            "the agent is too old; restart Micra to update it"
        ),
        other => other.to_string(),
    };
    Error::Api(tr!(
        "playit.gg rechazó la petición ({path}): {detail}",
        "playit.gg rejected the request ({path}): {detail}"
    ))
}

fn valid_secret(secret: &str) -> bool {
    !secret.is_empty() && secret.len() <= 256 && hex::decode(secret).is_ok()
}

// ---------------------------------------------------------------------------
// Vincular la cuenta
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Claim {
    pub code: String,
    /// Página que la persona abre para aceptar.
    pub url: String,
}

/// Código nuevo para vincular este equipo (5 bytes aleatorios en hex).
pub fn new_claim() -> Claim {
    let mut buf = [0u8; 5];
    getrandom::fill(&mut buf).expect("el sistema debe dar números aleatorios");
    let code = hex::encode(buf);
    Claim {
        url: format!("https://playit.gg/claim/{code}"),
        code,
    }
}

fn valid_code(code: &str) -> bool {
    code.len() == 10 && code.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub enum ClaimStatus {
    /// La persona aún no abrió el enlace.
    WaitingForUserVisit,
    /// Lo abrió y falta que acepte.
    WaitingForUser,
    UserAccepted,
    UserRejected,
}

/// En qué punto está la vinculación (se consulta cada pocos segundos).
pub async fn claim_status(http: &reqwest::Client, code: &str) -> Result<ClaimStatus> {
    if !valid_code(code) {
        return Err(Error::InvalidConfig(tr!(
            "código de vinculación no válido",
            "invalid link code"
        )));
    }
    let body = json!({
        "code": code,
        "agent_type": "self-managed",
        "version": concat!("Micra ", env!("CARGO_PKG_VERSION")),
    });
    match call(http, None, "/claim/setup", body).await? {
        Reply::Ok(status) => Ok(status),
        Reply::Fail(reason) => Err(claim_error(&reason)),
    }
}

/// Recoge la clave del agente cuando la persona ya aceptó. `None` si todavía
/// no está lista (hay que seguir esperando).
pub async fn claim_exchange(http: &reqwest::Client, code: &str) -> Result<Option<String>> {
    if !valid_code(code) {
        return Err(Error::InvalidConfig(tr!(
            "código de vinculación no válido",
            "invalid link code"
        )));
    }
    #[derive(Deserialize)]
    struct SecretKey {
        secret_key: String,
    }
    match call::<SecretKey>(http, None, "/claim/exchange", json!({ "code": code })).await? {
        Reply::Ok(key) if valid_secret(key.secret_key.trim()) => {
            Ok(Some(key.secret_key.trim().to_string()))
        }
        Reply::Ok(_) => Err(Error::Api(tr!(
            "playit.gg entregó una clave con un formato inesperado",
            "playit.gg returned a key in an unexpected format"
        ))),
        Reply::Fail(reason) if reason == "NotAccepted" || reason == "NotSetup" => Ok(None),
        Reply::Fail(reason) => Err(claim_error(&reason)),
    }
}

fn claim_error(reason: &str) -> Error {
    Error::Api(match reason {
        "UserRejected" => tr!(
            "se rechazó la vinculación en playit.gg",
            "the link was rejected on playit.gg"
        ),
        "CodeExpired" | "CodeNotFound" | "InvalidCode" => tr!(
            "el enlace de vinculación caducó: vuelve a empezar",
            "the link expired: start again"
        ),
        other => tr!(
            "no se pudo vincular con playit.gg: {other}",
            "could not link with playit.gg: {other}"
        ),
    })
}

// ---------------------------------------------------------------------------
// Túneles
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct RunData {
    pub agent_id: String,
    #[serde(default)]
    pub account_status: Option<String>,
    #[serde(default)]
    pub tunnels: Vec<RemoteTunnel>,
    #[serde(default)]
    pub pending: Vec<PendingTunnel>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoteTunnel {
    pub id: String,
    pub name: Option<String>,
    pub port: PortRange,
    pub local_ip: IpAddr,
    pub local_port: u16,
    pub assigned_domain: String,
    pub custom_domain: Option<String>,
    /// `ByUser` o `BySystem` si está desactivado.
    pub disabled: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PortRange {
    pub from: u16,
    pub to: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PendingTunnel {
    pub id: String,
    pub name: Option<String>,
}

impl RemoteTunnel {
    /// Dirección que se da a los amigos.
    pub fn address(&self) -> String {
        let host = self
            .custom_domain
            .as_deref()
            .filter(|d| !d.is_empty())
            .unwrap_or(&self.assigned_domain);
        if self.port.from == 25565 {
            host.to_string()
        } else {
            format!("{host}:{}", self.port.from)
        }
    }
}

/// Estado del túnel de un servidor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TunnelState {
    Ready {
        address: String,
    },
    /// playit.gg está asignando la dirección (unos segundos).
    Pending,
    /// playit.gg lo desactivó (límites de la cuenta, por ejemplo).
    Disabled {
        reason: String,
    },
}

/// Nombre del túnel de un servidor en la cuenta de playit.gg.
pub fn tunnel_name(server_id: &str) -> String {
    let id: String = server_id.chars().take(40).collect();
    format!("micra-{id}")
}

pub async fn rundata(http: &reqwest::Client, secret: &str) -> Result<RunData> {
    match call(http, Some(secret), "/agents/rundata", json!({})).await? {
        Reply::Ok(data) => Ok(data),
        Reply::Fail(reason) => Err(fail("/agents/rundata", &reason)),
    }
}

/// Crea el túnel del servidor si no existe y lo apunta a su puerto.
pub async fn ensure_tunnel(
    http: &reqwest::Client,
    secret: &str,
    server_id: &str,
    local_port: u16,
) -> Result<TunnelState> {
    let name = tunnel_name(server_id);
    let data = rundata(http, secret).await?;
    if let Some(tunnel) = data
        .tunnels
        .iter()
        .find(|t| t.name.as_deref() == Some(name.as_str()))
    {
        let wrong_target = tunnel.local_port != local_port || tunnel.local_ip != LOCALHOST;
        if wrong_target || tunnel.disabled.is_some() {
            update_tunnel(http, secret, &tunnel.id, &data.agent_id, local_port).await?;
        }
        if let Some(reason) = &tunnel.disabled {
            if reason_text(reason) == "BySystem" {
                return Ok(TunnelState::Disabled {
                    reason: tr!(
                        "playit.gg desactivó el túnel; revisa tu cuenta en playit.gg",
                        "playit.gg disabled the tunnel; check your account on playit.gg"
                    ),
                });
            }
        }
        return Ok(TunnelState::Ready {
            address: tunnel.address(),
        });
    }
    if data
        .pending
        .iter()
        .any(|t| t.name.as_deref() == Some(name.as_str()))
    {
        return Ok(TunnelState::Pending);
    }
    let body = json!({
        "name": name,
        "tunnel_type": "minecraft-java",
        "port_type": "tcp",
        "port_count": 1,
        "origin": {
            "type": "agent",
            "data": {
                "agent_id": data.agent_id,
                "local_ip": LOCALHOST,
                "local_port": local_port,
            },
        },
        "enabled": true,
        "alloc": null,
        "firewall_id": null,
        "proxy_protocol": null,
    });
    match call::<Value>(http, Some(secret), "/tunnels/create", body).await? {
        Reply::Ok(_) => Ok(TunnelState::Pending),
        Reply::Fail(reason) => Err(fail("/tunnels/create", &reason)),
    }
}

async fn update_tunnel(
    http: &reqwest::Client,
    secret: &str,
    tunnel_id: &str,
    agent_id: &str,
    local_port: u16,
) -> Result<()> {
    let body = json!({
        "tunnel_id": tunnel_id,
        "local_ip": LOCALHOST,
        "local_port": local_port,
        "agent_id": agent_id,
        "enabled": true,
    });
    match call::<Value>(http, Some(secret), "/tunnels/update", body).await? {
        Reply::Ok(_) => Ok(()),
        Reply::Fail(reason) => Err(fail("/tunnels/update", &reason)),
    }
}

/// Borra el túnel del servidor (al desactivarlo o borrar el servidor).
pub async fn delete_tunnel(http: &reqwest::Client, secret: &str, server_id: &str) -> Result<()> {
    let name = tunnel_name(server_id);
    let data = rundata(http, secret).await?;
    let ids = data
        .tunnels
        .iter()
        .filter(|t| t.name.as_deref() == Some(name.as_str()))
        .map(|t| t.id.clone())
        .chain(
            data.pending
                .iter()
                .filter(|t| t.name.as_deref() == Some(name.as_str()))
                .map(|t| t.id.clone()),
        );
    for id in ids {
        match call::<Value>(
            http,
            Some(secret),
            "/tunnels/delete",
            json!({ "tunnel_id": id }),
        )
        .await?
        {
            Reply::Ok(_) => {}
            Reply::Fail(reason) if reason == "TunnelNotFound" => {}
            Reply::Fail(reason) => return Err(fail("/tunnels/delete", &reason)),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// El agente
// ---------------------------------------------------------------------------

/// Archivos del agente, en `<herramientas>/playit` (carpeta privada).
struct AgentFiles {
    dir: PathBuf,
}

impl AgentFiles {
    fn new(tools_dir: &Path) -> Self {
        Self {
            dir: tools_dir.join("playit"),
        }
    }

    fn bin(&self) -> PathBuf {
        self.dir.join(if cfg!(windows) {
            "playitd.exe"
        } else {
            "playitd"
        })
    }

    fn meta(&self) -> PathBuf {
        self.dir.join("agent.json")
    }

    fn secret(&self) -> PathBuf {
        self.dir.join("playit.toml")
    }

    fn log(&self) -> PathBuf {
        self.dir.join("playitd.log")
    }

    fn console_log(&self) -> PathBuf {
        self.dir.join("playitd-console.log")
    }

    #[cfg(windows)]
    fn socket(&self) -> String {
        r"\\.\pipe\micra-playitd".into()
    }

    #[cfg(not(windows))]
    fn socket(&self) -> String {
        let path = self.dir.join("playitd.sock");
        // Las rutas de un socket Unix no pueden pasar de ~108 bytes.
        if path.as_os_str().len() < 100 {
            return path.display().to_string();
        }
        match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(dir) => Path::new(&dir)
                .join("micra-playitd.sock")
                .display()
                .to_string(),
            None => path.display().to_string(),
        }
    }

    /// Crea la carpeta y la deja solo para el usuario: dentro están la
    /// clave mientras el agente funciona y su socket de control.
    fn ensure_dir(&self) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InstalledAgent {
    tag: String,
    sha256: String,
    checked_at: DateTime<Utc>,
}

struct AgentRelease {
    tag: String,
    url: String,
    sha256: String,
}

fn asset_name() -> Option<&'static str> {
    if cfg!(windows) {
        // También en Windows ARM, que ejecuta programas x86_64.
        Some("playit-windows-x86_64.exe")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("playit-linux-amd64")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("playit-linux-aarch64")
    } else {
        None
    }
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

/// Elige el archivo de esta plataforma. Sin SHA-256 publicado no se descarga.
fn pick_asset(release: GhRelease, name: &str) -> Result<AgentRelease> {
    let asset = release
        .assets
        .into_iter()
        .find(|a| a.name == name)
        .ok_or_else(|| Error::NotFound(format!("playit {} ({name})", release.tag_name)))?;
    if !asset.browser_download_url.starts_with(DOWNLOAD_PREFIX) {
        return Err(Error::DisallowedUrl(asset.browser_download_url));
    }
    let sha256 = asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
        .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| {
            let tag = &release.tag_name;
            Error::Api(tr!(
                "la versión {tag} de playit no publica su SHA-256; por seguridad no se descarga",
                "playit release {tag} does not publish its SHA-256; it is not downloaded for safety"
            ))
        })?;
    Ok(AgentRelease {
        tag: release.tag_name,
        url: asset.browser_download_url,
        sha256: sha256.to_ascii_lowercase(),
    })
}

async fn latest_release(http: &reqwest::Client) -> Result<AgentRelease> {
    let name = asset_name().ok_or_else(|| {
        Error::UnsupportedPlatform(tr!(
            "el túnel de playit.gg en este sistema",
            "the playit.gg tunnel on this system"
        ))
    })?;
    let release: GhRelease = http
        .get(RELEASES)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .timeout(Duration::from_secs(20))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    pick_asset(release, name)
}

fn read_installed(files: &AgentFiles) -> Option<InstalledAgent> {
    let raw = std::fs::read(files.meta()).ok()?;
    serde_json::from_slice(&raw).ok()
}

/// Descarga (o actualiza) el agente si hace falta y devuelve su ruta.
pub async fn ensure_agent(http: &reqwest::Client, tools_dir: &Path) -> Result<PathBuf> {
    let files = AgentFiles::new(tools_dir);
    files.ensure_dir()?;
    let bin = files.bin();

    let installed = match read_installed(&files) {
        Some(meta) if bin.exists() && hash_file(&bin, HashAlgo::Sha256).await? == meta.sha256 => {
            Some(meta)
        }
        _ => None,
    };
    if let Some(meta) = &installed {
        if Utc::now() - meta.checked_at < UPDATE_CHECK {
            return Ok(bin);
        }
    }

    let release = match latest_release(http).await {
        Ok(release) => release,
        // Sin conexión con GitHub: sirve el que ya está comprobado.
        Err(e) if installed.is_some() => {
            tracing::warn!("no se pudo buscar una versión nueva de playit: {e}");
            return Ok(bin);
        }
        Err(e) => return Err(e),
    };
    if installed
        .as_ref()
        .is_some_and(|m| m.sha256 == release.sha256)
    {
        save_meta(&files, &release)?;
        return Ok(bin);
    }

    let expected = ExpectedHash::new(HashAlgo::Sha256, &release.sha256);
    download_file(http, &release.url, &bin, Some(&expected), None).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700))?;
    }
    save_meta(&files, &release)?;
    Ok(bin)
}

fn save_meta(files: &AgentFiles, release: &AgentRelease) -> Result<()> {
    let meta = InstalledAgent {
        tag: release.tag.clone(),
        sha256: release.sha256.clone(),
        checked_at: Utc::now(),
    };
    write_atomic_sync(&files.meta(), &serde_json::to_vec_pretty(&meta)?)
}

/// El agente en marcha. Al soltarlo se detiene y se borra la copia de la
/// clave que necesita leer.
pub struct Agent {
    child: tokio::process::Child,
    secret_file: PathBuf,
    console_log: PathBuf,
    log: PathBuf,
    #[cfg(windows)]
    _job: Option<crate::server::sandbox::windows::Job>,
}

impl Agent {
    pub async fn start(bin: &Path, tools_dir: &Path, secret: &str) -> Result<Agent> {
        if !valid_secret(secret) {
            return Err(Error::InvalidConfig(tr!(
                "la clave guardada de playit.gg no es válida: vuelve a vincular la cuenta",
                "the saved playit.gg key is not valid: link the account again"
            )));
        }
        let files = AgentFiles::new(tools_dir);
        files.ensure_dir()?;
        let secret_file = files.secret();
        write_private(&secret_file, &format!("secret_key = \"{secret}\"\n"))?;

        let socket = files.socket();
        #[cfg(unix)]
        {
            // Un socket que quedó de una ejecución anterior impide arrancar.
            let _ = std::fs::remove_file(&socket);
        }
        let console = std::fs::File::create(files.console_log())?;
        let mut cmd = tokio::process::Command::new(bin);
        cmd.arg("--secret-path")
            .arg(&secret_file)
            .arg("--socket-path")
            .arg(&socket)
            .arg("--log-path")
            .arg(files.log())
            .current_dir(&files.dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(console.try_clone()?))
            .stderr(Stdio::from(console))
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(crate::server::launcher::CREATE_NO_WINDOW);
        #[cfg(target_os = "linux")]
        // SAFETY: tras `fork` solo se hace una llamada al sistema, sin reservar
        // memoria. Si Micra muere de golpe, el sistema cierra el agente.
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        let child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                let _ = std::fs::remove_file(&secret_file);
                return Err(e.into());
            }
        };
        #[cfg(windows)]
        let job = attach_job(&child);

        Ok(Agent {
            child,
            secret_file,
            console_log: files.console_log(),
            log: files.log(),
            #[cfg(windows)]
            _job: job,
        })
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub async fn stop(mut self) {
        let _ = self.child.kill().await;
        let _ = std::fs::remove_file(&self.secret_file);
    }

    /// Últimas líneas del registro del agente (para explicar un fallo).
    pub fn log_tail(&self, lines: usize) -> Vec<String> {
        let mut out = tail(&self.console_log, lines);
        if out.is_empty() {
            out = tail(&self.log, lines);
        }
        out
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.secret_file);
    }
}

/// El agente vive dentro de un Job Object: si Micra se cierra de golpe,
/// Windows lo cierra también.
#[cfg(windows)]
fn attach_job(child: &tokio::process::Child) -> Option<crate::server::sandbox::windows::Job> {
    use crate::server::sandbox::windows as win;
    let handle = child.raw_handle()? as windows_sys::Win32::Foundation::HANDLE;
    let job = win::create_job().ok()?;
    // SAFETY: `handle` es el del hijo, que sigue vivo (lo tenemos en `child`).
    unsafe { win::assign(&job, handle).ok()? };
    Some(job)
}

fn write_private(path: &Path, content: &str) -> Result<()> {
    write_private_sync(path, content.as_bytes())
}

fn tail(path: &Path, lines: usize) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let all: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    all[all.len().saturating_sub(lines)..]
        .iter()
        .map(|l| l.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_and_secrets() {
        let claim = new_claim();
        assert!(valid_code(&claim.code));
        assert_eq!(claim.url, format!("https://playit.gg/claim/{}", claim.code));
        assert!(!valid_code("zzzzzzzzzz"));
        assert!(!valid_code("../../etc"));
        assert!(valid_secret("0a1b2c3d"));
        assert!(!valid_secret("0a1b\"\nx = 1"));
        assert!(!valid_secret(""));
    }

    #[test]
    fn parses_api_replies() {
        let ok: ApiReply<ClaimStatus> =
            serde_json::from_str(r#"{"status":"success","data":"WaitingForUser"}"#).unwrap();
        assert!(matches!(ok, ApiReply::Success(ClaimStatus::WaitingForUser)));
        assert_eq!(
            serde_json::to_value(ClaimStatus::UserAccepted).unwrap(),
            json!("userAccepted")
        );
        let failed: ApiReply<Value> =
            serde_json::from_str(r#"{"status":"fail","data":"NotAccepted"}"#).unwrap();
        assert!(matches!(failed, ApiReply::Fail(Value::String(ref s)) if s == "NotAccepted"));
        let error: ApiReply<Value> = serde_json::from_str(
            r#"{"status":"error","data":{"type":"auth","message":"InvalidAgentKey"}}"#,
        )
        .unwrap();
        let ApiReply::Error(e) = error else {
            panic!("debería ser un error")
        };
        assert!(api_error(&e).to_string().contains("InvalidAgentKey"));
    }

    #[test]
    fn tunnel_addresses() {
        let data: RunData = serde_json::from_value(json!({
            "agent_id": "8d1c5b0e-0000-4000-8000-000000000001",
            "agent_type": "self-managed",
            "account_status": "ready",
            "tunnels": [{
                "id": "t1", "internal_id": 1, "name": "micra-survival", "ip_num": 1,
                "region_num": 0, "port": {"from": 41234, "to": 41235}, "proto": "tcp",
                "local_ip": "127.0.0.1", "local_port": 25565, "tunnel_type": "minecraft-java",
                "assigned_domain": "abc.gl.joinmc.link", "custom_domain": null,
                "disabled": null, "proxy_protocol": null, "agent_config": {"fields": []}
            }],
            "pending": [{"id": "t2", "name": "micra-creative", "proto": "tcp", "port_count": 1,
                         "tunnel_type": "minecraft-java", "is_disabled": false, "region_num": 0}],
            "account_features": {"regional_tunnels": false}
        }))
        .unwrap();
        assert_eq!(data.tunnels[0].address(), "abc.gl.joinmc.link:41234");
        assert_eq!(data.pending[0].name.as_deref(), Some("micra-creative"));
        let mut custom = data.tunnels[0].clone();
        custom.custom_domain = Some("mc.example.org".into());
        custom.port.from = 25565;
        assert_eq!(custom.address(), "mc.example.org");
        assert_eq!(tunnel_name("survival"), "micra-survival");
    }

    #[test]
    fn release_assets_need_a_hash() {
        let release = |digest: Option<&str>, url: &str| GhRelease {
            tag_name: "v1.0.12".into(),
            assets: vec![GhAsset {
                name: "playit-linux-amd64".into(),
                browser_download_url: url.into(),
                digest: digest.map(String::from),
            }],
        };
        let url = "https://github.com/playit-cloud/playit-agent/releases/download/v1.0.12/playit-linux-amd64";
        let hash = "a".repeat(64);
        let ok = pick_asset(
            release(Some(&format!("sha256:{hash}")), url),
            "playit-linux-amd64",
        )
        .unwrap();
        assert_eq!(ok.sha256, hash);
        assert!(pick_asset(release(None, url), "playit-linux-amd64").is_err());
        assert!(pick_asset(
            release(
                Some(&format!("sha256:{hash}")),
                "https://evil.example/playit"
            ),
            "playit-linux-amd64"
        )
        .is_err());
        assert!(pick_asset(
            release(Some(&format!("sha256:{hash}")), url),
            "playit-windows-x86_64.exe"
        )
        .is_err());
    }

    #[test]
    fn log_tail_skips_blank_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log");
        std::fs::write(&path, "a\n\nb\nc\n").unwrap();
        assert_eq!(tail(&path, 2), vec!["b", "c"]);
        assert!(tail(&dir.path().join("missing"), 2).is_empty());
    }
}
