use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use chrono::{DateTime, Local};
use micra_core::i18n::Lang;
use micra_core::java::JavaManager;
use micra_core::net::playit::{Agent, TunnelState};
use micra_core::net::upnp::PortMapping;
use micra_core::notify::DiscordSender;
use micra_core::paths::AppPaths;
use micra_core::providers::curseforge::CurseForgeClient;
use micra_core::providers::modrinth::ModrinthClient;
use micra_core::secrets::{SecretKey, SecretStore};
use micra_core::server::config::ServerConfig;
use micra_core::server::pack::PackUpdate;
use micra_core::server::process::ServerHandle;
use micra_core::server::sandbox::SandboxStatus;
use micra_core::settings::AppSettings;
use micra_core::{tr, Error, Result};
use serde::Serialize;
use tokio::sync::{watch, Mutex, RwLock};

/// Estado compartido por los comandos y las tareas de cada servidor.
pub type Shared = Arc<AppState>;

pub struct AppState {
    pub paths: AppPaths,
    pub http: reqwest::Client,
    pub java: JavaManager,
    pub modrinth: ModrinthClient,
    /// Se rehace cuando la persona guarda o borra su clave de CurseForge.
    pub curseforge: RwLock<Option<Arc<CurseForgeClient>>>,
    pub settings: RwLock<AppSettings>,
    pub secrets: SecretStore,
    pub discord: DiscordSender,
    /// Servidores lanzados en esta sesión, por id (también los ya apagados,
    /// para conservar su último estado).
    pub running: RwLock<HashMap<String, Arc<RunningServer>>>,
    /// Lo que la interfaz muestra de cada servidor además de su estado.
    pub live: StdMutex<HashMap<String, LiveInfo>>,
    /// Agente de playit.gg, compartido por todos los servidores con túnel.
    pub tunnel_agent: Mutex<Option<Agent>>,
    /// Servidores con una operación larga en curso (restaurar, actualizar).
    busy: StdMutex<HashSet<String>>,
}

/// Un servidor lanzado por Micra.
pub struct RunningServer {
    pub handle: Arc<ServerHandle>,
    pub dir: PathBuf,
    /// Configuración en vigor; las tareas se enteran de los cambios.
    pub config: watch::Sender<ServerConfig>,
    pub started_at: DateTime<Local>,
    /// Por qué se está apagando (para reiniciar o no después).
    pub stop_reason: StdMutex<Option<StopReason>>,
    /// `true` cuando la vigilancia terminó de recoger (puerto cerrado,
    /// estadísticas guardadas) tras apagarse.
    pub finished: watch::Sender<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    User,
    Restart,
    Idle,
    AppExit,
}

/// Estado de la apertura del puerto con UPnP.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UpnpInfo {
    Opening,
    Open { mapping: PortMapping },
    Failed { error: String },
}

/// Estado del túnel de playit.gg de un servidor.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TunnelInfo {
    Starting,
    Ready { address: String },
    Pending,
    Disabled { reason: String },
    Failed { error: String },
}

impl From<TunnelState> for TunnelInfo {
    fn from(state: TunnelState) -> Self {
        match state {
            TunnelState::Ready { address } => TunnelInfo::Ready { address },
            TunnelState::Pending => TunnelInfo::Pending,
            TunnelState::Disabled { reason } => TunnelInfo::Disabled { reason },
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveInfo {
    pub tps: Option<f32>,
    pub upnp: Option<UpnpInfo>,
    pub tunnel: Option<TunnelInfo>,
    pub restart_at: Option<DateTime<Local>>,
    pub pack_update: Option<PackUpdate>,
    pub mod_updates: usize,
    pub sandbox: Option<SandboxStatus>,
    /// BlueMap pide aceptar la descarga de recursos del juego.
    pub map_needs_accept: bool,
    pub last_backup_error: Option<String>,
}

impl AppState {
    pub async fn new() -> Result<Self> {
        let paths = AppPaths::from_system()?;
        let http = micra_core::http::build_client()?;
        let settings = AppSettings::load(&paths.settings_file()).await;
        micra_core::i18n::set_lang(effective_lang(&settings));
        let secrets = SecretStore::new(paths.secrets_file());
        let curseforge = curseforge_client(&http, &secrets).await;
        Ok(Self {
            java: JavaManager::new(paths.clone(), http.clone()),
            modrinth: ModrinthClient::new(http.clone()),
            curseforge: RwLock::new(curseforge),
            settings: RwLock::new(settings),
            secrets,
            discord: DiscordSender::new(http.clone()),
            running: RwLock::new(HashMap::new()),
            live: StdMutex::new(HashMap::new()),
            tunnel_agent: Mutex::new(None),
            busy: StdMutex::new(HashSet::new()),
            paths,
            http,
        })
    }

    pub async fn curseforge(&self) -> Option<Arc<CurseForgeClient>> {
        self.curseforge.read().await.clone()
    }

    pub async fn reload_curseforge(&self) {
        *self.curseforge.write().await = curseforge_client(&self.http, &self.secrets).await;
    }

    pub async fn lang(&self) -> Lang {
        effective_lang(&*self.settings.read().await)
    }

    pub async fn server(&self, id: &str) -> Option<Arc<RunningServer>> {
        self.running.read().await.get(id).cloned()
    }

    /// El servidor en marcha (error si está apagado).
    pub async fn alive(&self, id: &str) -> Result<Arc<RunningServer>> {
        self.server(id)
            .await
            .filter(|s| s.handle.is_alive())
            .ok_or(Error::NotRunning)
    }

    pub async fn is_alive(&self, id: &str) -> bool {
        self.server(id).await.is_some_and(|s| s.handle.is_alive())
    }

    /// Error si el servidor está encendido (para operaciones con archivos).
    pub async fn ensure_stopped(&self, id: &str) -> Result<()> {
        if self.is_alive(id).await {
            return Err(Error::MustBeStopped);
        }
        Ok(())
    }

    pub fn live(&self, id: &str) -> LiveInfo {
        self.live
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .unwrap_or_default()
    }

    /// Cambia la información en vivo y devuelve la nueva para emitirla.
    pub fn update_live(&self, id: &str, change: impl FnOnce(&mut LiveInfo)) -> LiveInfo {
        let mut map = self.live.lock().unwrap();
        let entry = map.entry(id.to_string()).or_default();
        change(entry);
        entry.clone()
    }

    /// Marca el servidor como ocupado mientras viva el guardián devuelto.
    pub fn begin_busy(self: &Arc<Self>, id: &str) -> Result<BusyGuard> {
        let mut busy = self.busy.lock().unwrap();
        if !busy.insert(id.to_string()) {
            return Err(Error::InvalidConfig(tr!(
                "ya hay una operación en curso con este servidor; espera a que termine",
                "an operation is already running on this server; wait for it to finish"
            )));
        }
        Ok(BusyGuard {
            state: self.clone(),
            id: id.to_string(),
        })
    }

    pub fn is_busy(&self, id: &str) -> bool {
        self.busy.lock().unwrap().contains(id)
    }

    /// Al cerrar la app: apagar los servidores con `stop` (para no dañar los
    /// mundos) y esperar a que se cierren los puertos del router.
    pub async fn stop_all(&self) {
        let servers: Vec<_> = self.running.read().await.values().cloned().collect();
        // En paralelo: cada servidor puede tardar en guardar su mundo.
        let stops: Vec<_> = servers
            .into_iter()
            .filter(|s| s.handle.is_alive())
            .map(|server| {
                tauri::async_runtime::spawn(async move {
                    *server.stop_reason.lock().unwrap() = Some(StopReason::AppExit);
                    let _ = server.handle.stop(Duration::from_secs(30)).await;
                    let mut finished = server.finished.subscribe();
                    let _ = tokio::time::timeout(
                        Duration::from_secs(10),
                        finished.wait_for(|done| *done),
                    )
                    .await;
                })
            })
            .collect();
        for stop in stops {
            let _ = stop.await;
        }
        if let Some(agent) = self.tunnel_agent.lock().await.take() {
            agent.stop().await;
        }
    }
}

pub struct BusyGuard {
    state: Arc<AppState>,
    id: String,
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.state.busy.lock().unwrap().remove(&self.id);
    }
}

/// El idioma elegido o, si no se eligió, el del sistema (inglés si la app
/// no tiene el del sistema).
pub fn effective_lang(settings: &AppSettings) -> Lang {
    settings.language.unwrap_or_else(|| {
        sys_locale::get_locale()
            .as_deref()
            .and_then(Lang::from_tag)
            .unwrap_or(Lang::En)
    })
}

/// La clave de CurseForge guardada por la persona tiene prioridad sobre la
/// incluida al compilar.
async fn curseforge_client(
    http: &reqwest::Client,
    secrets: &SecretStore,
) -> Option<Arc<CurseForgeClient>> {
    let saved = secrets
        .get(SecretKey::CurseforgeApiKey)
        .await
        .ok()
        .flatten()
        .filter(|k| !k.trim().is_empty());
    match saved {
        Some(key) => Some(Arc::new(CurseForgeClient::new(http.clone(), key.trim()))),
        None => CurseForgeClient::from_env(http.clone()).map(Arc::new),
    }
}
