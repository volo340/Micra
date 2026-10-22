use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use micra_core::java::JavaManager;
use micra_core::paths::AppPaths;
use micra_core::providers::curseforge::CurseForgeClient;
use micra_core::providers::modrinth::ModrinthClient;
use micra_core::server::process::ServerHandle;
use tokio::sync::RwLock;

pub struct AppState {
    pub paths: AppPaths,
    pub http: reqwest::Client,
    pub java: JavaManager,
    pub modrinth: ModrinthClient,
    pub curseforge: Option<CurseForgeClient>,
    /// Servidores lanzados en esta sesión, por id.
    pub running: RwLock<HashMap<String, Arc<ServerHandle>>>,
}

impl AppState {
    pub fn new() -> micra_core::Result<Self> {
        let paths = AppPaths::from_system()?;
        let http = micra_core::http::build_client()?;
        Ok(Self {
            java: JavaManager::new(paths.clone(), http.clone()),
            modrinth: ModrinthClient::new(http.clone()),
            curseforge: CurseForgeClient::from_env(http.clone()),
            running: RwLock::new(HashMap::new()),
            paths,
            http,
        })
    }

    pub async fn stop_all(&self) {
        let handles: Vec<_> = self.running.read().await.values().cloned().collect();
        for handle in handles {
            let _ = handle.stop(Duration::from_secs(30)).await;
        }
    }
}
