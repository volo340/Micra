//! Ajustes globales de la app (`settings.json` en la carpeta de datos).
//!
//! Los secretos (webhook de Discord, claves) nunca van aquí: viven en el
//! llavero del sistema (ver `secrets`).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::fsutil::write_atomic;
use crate::i18n::Lang;
use crate::Result;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    /// `None` hasta que la persona elige: entonces se usa el idioma del sistema.
    pub language: Option<Lang>,
    pub notifications: NotificationSettings,
    pub updates: UpdateSettings,
}

/// Qué avisos enviar y por dónde. Discord solo se usa si además hay un
/// webhook guardado en el llavero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NotificationSettings {
    pub desktop: bool,
    pub discord: bool,
    pub player_join: bool,
    pub player_leave: bool,
    pub server_ready: bool,
    pub server_crash: bool,
    /// Accesos rechazados, IPs bloqueadas y malware.
    pub security: bool,
    pub backup_failed: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            desktop: true,
            discord: false,
            player_join: true,
            player_leave: false,
            server_ready: false,
            server_crash: true,
            security: true,
            backup_failed: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateSettings {
    /// Buscar una versión nueva de Micra al abrir la app.
    pub auto_check: bool,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self { auto_check: true }
    }
}

impl AppSettings {
    /// Lee los ajustes; si el archivo no existe o está dañado, usa los de
    /// fábrica (un ajuste roto nunca debe impedir abrir la app).
    pub async fn load(path: &Path) -> Self {
        match tokio::fs::read(path).await {
            Ok(raw) => serde_json::from_slice(&raw).unwrap_or_else(|e| {
                tracing::warn!("settings.json dañado ({e}); se usan los ajustes de fábrica");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub async fn save(&self, path: &Path) -> Result<()> {
        write_atomic(path, &serde_json::to_vec_pretty(self)?).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn defaults_and_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut s = AppSettings::load(&path).await;
        assert_eq!(s, AppSettings::default());
        assert!(s.notifications.desktop && !s.notifications.discord);

        s.language = Some(Lang::En);
        s.notifications.player_leave = true;
        s.save(&path).await.unwrap();
        assert_eq!(AppSettings::load(&path).await, s);

        // Campos que faltan (versiones antiguas) toman el valor por defecto.
        tokio::fs::write(&path, r#"{"language":"fr"}"#)
            .await
            .unwrap();
        let old = AppSettings::load(&path).await;
        assert_eq!(old.language, Some(Lang::Fr));
        assert!(old.updates.auto_check);

        tokio::fs::write(&path, b"{roto").await.unwrap();
        assert_eq!(AppSettings::load(&path).await, AppSettings::default());
    }
}
