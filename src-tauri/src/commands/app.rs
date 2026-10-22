//! La app: información, ajustes, secretos del llavero y actualizaciones.

use micra_core::hardware::HardwareInfo;
use micra_core::i18n::Lang;
use micra_core::java::JavaRuntime;
use micra_core::notify::{validate_webhook, NotifyEvent};
use micra_core::secrets::{SecretBackend, SecretKey};
use micra_core::settings::AppSettings;
use micra_core::{tr, Error};
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_updater::UpdaterExt;

use super::{open_path, CmdResult};
use crate::notifier;
use crate::state::{effective_lang, Shared};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: &'static str,
    data_dir: String,
    curseforge_enabled: bool,
    language: Lang,
    platform: &'static str,
}

#[tauri::command]
pub async fn app_info(state: State<'_, Shared>) -> CmdResult<AppInfo> {
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        data_dir: state.paths.root().display().to_string(),
        curseforge_enabled: state.curseforge().await.is_some(),
        language: state.lang().await,
        platform: if cfg!(windows) { "windows" } else { "linux" },
    })
}

#[tauri::command]
pub fn hardware_info() -> HardwareInfo {
    micra_core::hardware::hardware_info()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInfo {
    local_ip: Option<String>,
}

#[tauri::command]
pub fn network_info() -> NetworkInfo {
    NetworkInfo {
        local_ip: micra_core::net::local_ip().map(|ip| ip.to_string()),
    }
}

#[tauri::command]
pub fn list_java_runtimes(state: State<'_, Shared>) -> Vec<JavaRuntime> {
    state.java.list_installed()
}

#[tauri::command]
pub fn open_data_folder(app: AppHandle, state: State<'_, Shared>) -> CmdResult<()> {
    let dir = state.paths.root().to_path_buf();
    std::fs::create_dir_all(&dir)?;
    open_path(&app, &dir)
}

/// Qué secretos hay guardados (nunca su valor).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretsStatus {
    discord_webhook: bool,
    curseforge_api_key: bool,
    playit_agent: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    settings: AppSettings,
    /// Idioma en uso (el elegido o el del sistema).
    language: Lang,
    secrets: SecretsStatus,
    secret_backend: SecretBackend,
}

async fn settings_view(state: &Shared) -> SettingsView {
    let settings = state.settings.read().await.clone();
    SettingsView {
        language: effective_lang(&settings),
        settings,
        secrets: SecretsStatus {
            discord_webhook: state.secrets.has(SecretKey::DiscordWebhook).await,
            curseforge_api_key: state.secrets.has(SecretKey::CurseforgeApiKey).await,
            playit_agent: state.secrets.has(SecretKey::PlayitAgent).await,
        },
        secret_backend: state.secrets.backend(),
    }
}

#[tauri::command]
pub async fn get_settings(state: State<'_, Shared>) -> CmdResult<SettingsView> {
    Ok(settings_view(&state).await)
}

#[tauri::command]
pub async fn save_settings(
    state: State<'_, Shared>,
    settings: AppSettings,
) -> CmdResult<SettingsView> {
    settings.save(&state.paths.settings_file()).await?;
    micra_core::i18n::set_lang(effective_lang(&settings));
    *state.settings.write().await = settings;
    Ok(settings_view(&state).await)
}

#[tauri::command]
pub async fn set_secret(
    state: State<'_, Shared>,
    key: SecretKey,
    value: String,
) -> CmdResult<SettingsView> {
    let value = value.trim();
    match key {
        SecretKey::DiscordWebhook => validate_webhook(value)?,
        SecretKey::CurseforgeApiKey => {
            let ok = value.len() >= 20
                && value.len() <= 200
                && value.chars().all(|c| c.is_ascii_graphic());
            if !ok {
                return Err(Error::InvalidConfig(tr!(
                    "esa no parece una clave de API de CurseForge",
                    "that does not look like a CurseForge API key"
                )));
            }
        }
        // La clave del túnel solo la guarda la vinculación con playit.gg.
        SecretKey::PlayitAgent => {
            return Err(Error::InvalidConfig(tr!(
                "vincula la cuenta de playit.gg desde Ajustes",
                "link the playit.gg account from Settings"
            )))
        }
    }
    state.secrets.set(key, value).await?;
    if key == SecretKey::CurseforgeApiKey {
        state.reload_curseforge().await;
    }
    Ok(settings_view(&state).await)
}

#[tauri::command]
pub async fn delete_secret(state: State<'_, Shared>, key: SecretKey) -> CmdResult<SettingsView> {
    state.secrets.delete(key).await?;
    match key {
        SecretKey::CurseforgeApiKey => state.reload_curseforge().await,
        SecretKey::PlayitAgent => {
            if let Some(agent) = state.tunnel_agent.lock().await.take() {
                agent.stop().await;
            }
        }
        SecretKey::DiscordWebhook => {}
    }
    Ok(settings_view(&state).await)
}

/// Aviso de prueba por todos los canales configurados.
#[tauri::command]
pub async fn test_notification(app: AppHandle, state: State<'_, Shared>) -> CmdResult<()> {
    notifier::deliver(&app, &state, &NotifyEvent::Test, true).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdate {
    version: String,
    notes: Option<String>,
    date: Option<String>,
    /// Linux con .deb o .rpm: se actualiza volviendo a ejecutar el instalador.
    can_install: bool,
}

/// Las actualizaciones automáticas funcionan con el instalador de Windows y
/// con el AppImage de Linux.
fn can_self_update() -> bool {
    cfg!(windows) || std::env::var_os("APPIMAGE").is_some()
}

fn updater_error(e: tauri_plugin_updater::Error) -> Error {
    Error::Api(tr!(
        "no se pudo comprobar si hay una versión nueva: {e}",
        "could not check for a new version: {e}"
    ))
}

#[tauri::command]
pub async fn check_app_update(app: AppHandle) -> CmdResult<Option<AppUpdate>> {
    let update = app
        .updater()
        .map_err(updater_error)?
        .check()
        .await
        .map_err(updater_error)?;
    Ok(update.map(|u| AppUpdate {
        version: u.version.clone(),
        notes: u.body.clone(),
        date: u.date.map(|d| d.to_string()),
        can_install: can_self_update(),
    }))
}

/// Descarga la versión nueva, apaga los servidores y reinicia Micra.
#[tauri::command]
pub async fn install_app_update(app: AppHandle, state: State<'_, Shared>) -> CmdResult<()> {
    if !can_self_update() {
        return Err(Error::UnsupportedPlatform(tr!(
            "la actualización automática con este tipo de instalación; vuelve a ejecutar micra-install.sh",
            "automatic updates with this kind of installation; run micra-install.sh again"
        )));
    }
    let update = app
        .updater()
        .map_err(updater_error)?
        .check()
        .await
        .map_err(updater_error)?
        .ok_or_else(|| Error::NotFound(tr!("una versión más nueva", "a newer version")))?;
    let bytes = update
        .download(|_, _| {}, || {})
        .await
        .map_err(updater_error)?;
    // El instalador cierra Micra: antes, apagar los mundos con cuidado.
    state.stop_all().await;
    update.install(bytes).map_err(updater_error)?;
    app.restart();
}
