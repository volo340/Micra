//! Registro de lo que instaló un modpack (`.micra/pack.json`), para saber
//! qué archivos quitar o sustituir al actualizarlo sin tocar lo que añadió
//! la persona por su cuenta.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::fsutil::write_atomic;
use crate::Result;

pub const MANIFEST: &str = ".micra/pack.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackManifest {
    pub name: String,
    pub version: String,
    /// Rutas relativas con `/`.
    pub files: Vec<String>,
}

impl PackManifest {
    pub async fn load(server_dir: &Path) -> Option<Self> {
        let raw = tokio::fs::read(server_dir.join(MANIFEST)).await.ok()?;
        serde_json::from_slice(&raw).ok()
    }

    pub async fn save(&self, server_dir: &Path) -> Result<()> {
        let path = server_dir.join(MANIFEST);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        write_atomic(&path, &serde_json::to_vec_pretty(self)?).await
    }
}
