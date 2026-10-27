//! Secretos de la app: webhook de Discord, clave de CurseForge y la clave del
//! agente del túnel.
//!
//! Se guardan en el llavero del sistema (Administrador de credenciales en
//! Windows, Secret Service —GNOME Keyring, KWallet— en Linux). Si el sistema no
//! tiene llavero (algunos escritorios mínimos), se usa un archivo que solo puede
//! leer el usuario y la interfaz lo indica.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::fsutil::write_private_sync;
use crate::{Error, Result};

const SERVICE: &str = "io.github.volo340.micra";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretKey {
    DiscordWebhook,
    CurseforgeApiKey,
    PlayitAgent,
}

impl SecretKey {
    fn name(self) -> &'static str {
        match self {
            SecretKey::DiscordWebhook => "discord-webhook",
            SecretKey::CurseforgeApiKey => "curseforge-api-key",
            SecretKey::PlayitAgent => "playit-agent-secret",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretBackend {
    Keyring,
    File,
}

pub struct SecretStore {
    file: PathBuf,
    use_keyring: AtomicBool,
    file_lock: std::sync::Mutex<()>,
}

impl SecretStore {
    pub fn new(file: PathBuf) -> Self {
        let available = keyring::Entry::store_status().is_ok();
        if !available {
            tracing::warn!("no hay llavero del sistema; los secretos irán a un archivo protegido");
        }
        Self {
            file,
            use_keyring: AtomicBool::new(available),
            file_lock: std::sync::Mutex::new(()),
        }
    }

    /// Solo archivo, sin tocar el llavero (pruebas).
    pub fn file_only(file: PathBuf) -> Self {
        Self {
            file,
            use_keyring: AtomicBool::new(false),
            file_lock: std::sync::Mutex::new(()),
        }
    }

    pub fn backend(&self) -> SecretBackend {
        if self.use_keyring.load(Ordering::Relaxed) {
            SecretBackend::Keyring
        } else {
            SecretBackend::File
        }
    }

    pub async fn get(&self, key: SecretKey) -> Result<Option<String>> {
        if self.use_keyring.load(Ordering::Relaxed) {
            match blocking(move || keyring_get(key)).await {
                Ok(Some(value)) => return Ok(Some(value)),
                // Puede que se guardara en el archivo cuando no había llavero.
                Ok(None) => {}
                Err(e) => self.keyring_failed(&e),
            }
        }
        Ok(self.read_file()?.remove(key.name()))
    }

    pub async fn set(&self, key: SecretKey, value: &str) -> Result<()> {
        if self.use_keyring.load(Ordering::Relaxed) {
            let owned = value.to_string();
            match blocking(move || keyring_set(key, &owned)).await {
                Ok(()) => {
                    // No dejar una copia vieja en el archivo de respaldo.
                    return self.remove_from_file(key);
                }
                Err(e) => self.keyring_failed(&e),
            }
        }
        let _guard = self.file_lock.lock().unwrap();
        let mut map = self.read_file()?;
        map.insert(key.name().to_string(), value.to_string());
        self.write_file(&map)
    }

    pub async fn delete(&self, key: SecretKey) -> Result<()> {
        if self.use_keyring.load(Ordering::Relaxed) {
            if let Err(e) = blocking(move || keyring_delete(key)).await {
                self.keyring_failed(&e);
            }
        }
        self.remove_from_file(key)
    }

    pub async fn has(&self, key: SecretKey) -> bool {
        matches!(self.get(key).await, Ok(Some(_)))
    }

    fn keyring_failed(&self, e: &Error) {
        tracing::warn!("el llavero del sistema falló ({e}); se usa el archivo protegido");
        self.use_keyring.store(false, Ordering::Relaxed);
    }

    fn remove_from_file(&self, key: SecretKey) -> Result<()> {
        let _guard = self.file_lock.lock().unwrap();
        let mut map = self.read_file()?;
        if map.remove(key.name()).is_some() {
            self.write_file(&map)?;
        }
        Ok(())
    }

    fn read_file(&self) -> Result<BTreeMap<String, String>> {
        match std::fs::read(&self.file) {
            Ok(raw) => Ok(serde_json::from_slice(&raw).unwrap_or_default()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(e.into()),
        }
    }

    fn write_file(&self, map: &BTreeMap<String, String>) -> Result<()> {
        if map.is_empty() {
            let _ = std::fs::remove_file(&self.file);
            return Ok(());
        }
        // Solo el usuario puede leerlo, desde que se crea (en Windows la
        // carpeta de datos ya es privada del usuario).
        write_private_sync(&self.file, &serde_json::to_vec_pretty(map)?)
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    // El llavero puede bloquear (D-Bus, diálogo de desbloqueo): fuera del
    // hilo asíncrono.
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::Secret(e.to_string()))?
}

fn entry(key: SecretKey) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, key.name()).map_err(|e| Error::Secret(e.to_string()))
}

fn keyring_get(key: SecretKey) -> Result<Option<String>> {
    match entry(key)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(Error::Secret(e.to_string())),
    }
}

fn keyring_set(key: SecretKey, value: &str) -> Result<()> {
    entry(key)?
        .set_password(value)
        .map_err(|e| Error::Secret(e.to_string()))
}

fn keyring_delete(key: SecretKey) -> Result<()> {
    match entry(key)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(Error::Secret(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn file_fallback_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("secrets.json");
        let store = SecretStore::file_only(file.clone());
        assert_eq!(store.backend(), SecretBackend::File);
        assert_eq!(store.get(SecretKey::DiscordWebhook).await.unwrap(), None);

        store
            .set(
                SecretKey::DiscordWebhook,
                "https://discord.com/api/webhooks/1/x",
            )
            .await
            .unwrap();
        store.set(SecretKey::PlayitAgent, "abcd").await.unwrap();
        assert_eq!(
            store.get(SecretKey::PlayitAgent).await.unwrap().as_deref(),
            Some("abcd")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        store.delete(SecretKey::PlayitAgent).await.unwrap();
        store.delete(SecretKey::DiscordWebhook).await.unwrap();
        assert!(!store.has(SecretKey::DiscordWebhook).await);
        // Sin secretos no queda archivo.
        assert!(!file.exists());
    }
}
