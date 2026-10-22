//! Red: IP pública, comprobación desde internet y cuenta de playit.gg.

use micra_core::net::playit::{self, Claim, ClaimStatus};
use micra_core::secrets::SecretKey;
use micra_core::{tr, Error};
use serde::Serialize;
use tauri::{AppHandle, State};

use super::{open_url, CmdResult};
use crate::state::Shared;

#[tauri::command]
pub async fn public_ip(state: State<'_, Shared>) -> CmdResult<String> {
    Ok(micra_core::net::public_ip(&state.http).await?.to_string())
}

/// ¿Responde el servidor desde internet en `host:port`?
#[tauri::command]
pub async fn check_reachable(state: State<'_, Shared>, host: String, port: u16) -> CmdResult<bool> {
    micra_core::net::check_reachable(&state.http, host.trim(), port).await
}

/// Empieza a vincular la cuenta de playit.gg: abre la página para aceptar.
#[tauri::command]
pub fn playit_claim_start(app: AppHandle) -> CmdResult<Claim> {
    let claim = playit::new_claim();
    open_url(&app, &claim.url)?;
    Ok(claim)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimProgress {
    status: ClaimStatus,
    /// La clave ya está guardada en el llavero.
    linked: bool,
}

/// Se llama cada pocos segundos mientras la persona acepta en el navegador.
#[tauri::command]
pub async fn playit_claim_poll(state: State<'_, Shared>, code: String) -> CmdResult<ClaimProgress> {
    let status = playit::claim_status(&state.http, &code).await?;
    if status == ClaimStatus::UserRejected {
        return Err(Error::Api(tr!(
            "se rechazó la vinculación en playit.gg",
            "the link was rejected on playit.gg"
        )));
    }
    if status != ClaimStatus::UserAccepted {
        return Ok(ClaimProgress {
            status,
            linked: false,
        });
    }
    let Some(secret) = playit::claim_exchange(&state.http, &code).await? else {
        return Ok(ClaimProgress {
            status,
            linked: false,
        });
    };
    state.secrets.set(SecretKey::PlayitAgent, &secret).await?;
    Ok(ClaimProgress {
        status,
        linked: true,
    })
}
