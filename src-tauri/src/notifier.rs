//! Avisos de escritorio y de Discord según los ajustes de la persona.

use micra_core::notify::NotifyEvent;
use micra_core::secrets::SecretKey;
use micra_core::Result;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::state::Shared;

/// Envía el aviso por los canales activados, sin bloquear a quien llama.
pub fn notify(app: &AppHandle, state: &Shared, event: NotifyEvent) {
    let app = app.clone();
    let state = state.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = deliver(&app, &state, &event, false).await {
            tracing::warn!("no se pudo enviar un aviso: {e}");
        }
    });
}

/// `force`: el aviso de prueba sale aunque el canal esté desactivado, para
/// que la persona compruebe que llega.
pub async fn deliver(
    app: &AppHandle,
    state: &Shared,
    event: &NotifyEvent,
    force: bool,
) -> Result<()> {
    let settings = state.settings.read().await.notifications.clone();
    if !force && !event.enabled_in(&settings) {
        return Ok(());
    }
    let text = event.text(state.lang().await);
    if force || settings.desktop {
        let shown = app
            .notification()
            .builder()
            .title(&text.title)
            .body(&text.body)
            .show();
        if let Err(e) = shown {
            tracing::warn!("no se pudo mostrar la notificación de escritorio: {e}");
        }
    }
    if force || settings.discord {
        if let Some(webhook) = state.secrets.get(SecretKey::DiscordWebhook).await? {
            state.discord.send(&webhook, &text, event.tone()).await?;
        }
    }
    Ok(())
}
