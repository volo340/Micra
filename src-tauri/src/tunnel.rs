//! Túnel de playit.gg: un solo agente para todos los servidores que lo usan.

use std::time::Duration;

use micra_core::net::playit::{self, Agent, TunnelState};
use micra_core::secrets::SecretKey;
use micra_core::{tr, Error, Result};

use crate::state::{AppState, TunnelInfo};

/// Clave del agente guardada en el llavero.
pub async fn secret(state: &AppState) -> Result<String> {
    state
        .secrets
        .get(SecretKey::PlayitAgent)
        .await?
        .ok_or_else(|| {
            Error::InvalidConfig(tr!(
                "vincula tu cuenta de playit.gg en Ajustes para usar el túnel",
                "link your playit.gg account in Settings to use the tunnel"
            ))
        })
}

/// Arranca el agente si no está en marcha (o si se cayó).
pub async fn ensure_agent(state: &AppState, secret: &str) -> Result<()> {
    let mut agent = state.tunnel_agent.lock().await;
    if agent.as_mut().is_some_and(Agent::is_running) {
        return Ok(());
    }
    if let Some(old) = agent.take() {
        let tail = old.log_tail(5).join(" | ");
        tracing::warn!("el agente de playit se cerró; se vuelve a arrancar. Registro: {tail}");
        old.stop().await;
    }
    let tools = state.paths.tools();
    let bin = playit::ensure_agent(&state.http, &tools).await?;
    let mut started = Agent::start(&bin, &tools, secret).await?;
    // Si muere nada más arrancar (clave rechazada, sistema no compatible),
    // contarlo ahora con su registro.
    tokio::time::sleep(Duration::from_secs(2)).await;
    if !started.is_running() {
        let tail = started.log_tail(5).join(" | ");
        started.stop().await;
        return Err(Error::Api(tr!(
            "el agente de playit.gg se cerró al arrancar: {tail}",
            "the playit.gg agent closed right after starting: {tail}"
        )));
    }
    *agent = Some(started);
    Ok(())
}

/// Vuelve a arrancar el agente si se cayó.
pub async fn restart_if_down(state: &AppState) -> Result<()> {
    let secret = secret(state).await?;
    ensure_agent(state, &secret).await
}

/// Prepara el túnel de un servidor y espera (como mucho dos minutos) a que
/// playit.gg le asigne dirección.
pub async fn enable(state: &AppState, server_id: &str, port: u16) -> TunnelInfo {
    match try_enable(state, server_id, port).await {
        Ok(info) => info,
        Err(e) => TunnelInfo::Failed {
            error: e.to_string(),
        },
    }
}

async fn try_enable(state: &AppState, server_id: &str, port: u16) -> Result<TunnelInfo> {
    let secret = secret(state).await?;
    ensure_agent(state, &secret).await?;
    for _ in 0..24 {
        match playit::ensure_tunnel(&state.http, &secret, server_id, port).await? {
            TunnelState::Pending => tokio::time::sleep(Duration::from_secs(5)).await,
            other => return Ok(other.into()),
        }
    }
    Ok(TunnelInfo::Pending)
}

/// Vuelve a consultar el túnel (dirección asignada, desactivado...).
pub async fn refresh(state: &AppState, server_id: &str, port: u16) -> TunnelInfo {
    let result = async {
        let secret = secret(state).await?;
        ensure_agent(state, &secret).await?;
        playit::ensure_tunnel(&state.http, &secret, server_id, port).await
    }
    .await;
    match result {
        Ok(state) => state.into(),
        Err(e) => TunnelInfo::Failed {
            error: e.to_string(),
        },
    }
}

/// Detiene el agente si ya ningún servidor encendido usa el túnel.
pub async fn release(state: &AppState) {
    let still_needed = state
        .running
        .read()
        .await
        .values()
        .any(|s| s.handle.is_alive() && s.config.borrow().network.tunnel);
    if still_needed {
        return;
    }
    if let Some(agent) = state.tunnel_agent.lock().await.take() {
        agent.stop().await;
    }
}
