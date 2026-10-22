use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("error de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("error HTTP: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON inválido: {0}")]
    Json(#[from] serde_json::Error),
    #[error("archivo ZIP inválido: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("el hash de {file} no coincide (esperado {expected}, obtenido {actual})")]
    HashMismatch {
        file: String,
        expected: String,
        actual: String,
    },
    #[error("plataforma no soportada: {0}")]
    UnsupportedPlatform(String),
    #[error("no encontrado: {0}")]
    NotFound(String),
    #[error("configuración inválida: {0}")]
    InvalidConfig(String),
    #[error("ruta insegura dentro del archivo: {0}")]
    UnsafePath(String),
    #[error("URL de descarga no permitida: {0}")]
    DisallowedUrl(String),
    #[error("el servidor ya está en ejecución")]
    AlreadyRunning,
    #[error("el servidor no está en ejecución")]
    NotRunning,
    #[error("error de la API: {0}")]
    Api(String),
}

pub type Result<T> = std::result::Result<T, Error>;

// Tauri necesita que los errores de los comandos sean serializables;
// enviamos el mensaje legible al frontend.
impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
