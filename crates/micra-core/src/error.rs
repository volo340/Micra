use std::fmt;

use serde::{Serialize, Serializer};

use crate::tr;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Http(reqwest::Error),
    Json(serde_json::Error),
    Zip(zip::result::ZipError),
    HashMismatch {
        file: String,
        expected: String,
        actual: String,
    },
    UnsupportedPlatform(String),
    NotFound(String),
    InvalidConfig(String),
    UnsafePath(String),
    DisallowedUrl(String),
    AlreadyRunning,
    NotRunning,
    /// La operación necesita el servidor apagado (restaurar, editar archivos...).
    MustBeStopped,
    PortInUse {
        port: u16,
        by: Option<String>,
    },
    /// El escáner encontró malware conocido; el texto nombra los archivos.
    Malware(String),
    /// El llavero del sistema falló al guardar o leer un secreto.
    Secret(String),
    Api(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            Error::Io(e) => tr!("error de archivo: {e}", "file error: {e}"),
            Error::Http(e) => http_message(e),
            Error::Json(e) => tr!("JSON inválido: {e}", "invalid JSON: {e}"),
            Error::Zip(e) => tr!("archivo ZIP inválido: {e}", "invalid ZIP file: {e}"),
            Error::HashMismatch {
                file,
                expected,
                actual,
            } => tr!(
                "el archivo {file} está dañado o fue alterado: su hash no coincide (esperado {expected}, obtenido {actual})",
                "the file {file} is damaged or was tampered with: its hash does not match (expected {expected}, got {actual})"
            ),
            Error::UnsupportedPlatform(what) => {
                tr!("no soportado: {what}", "not supported: {what}")
            }
            Error::NotFound(what) => tr!("no encontrado: {what}", "not found: {what}"),
            Error::InvalidConfig(why) => why.clone(),
            Error::UnsafePath(path) => tr!(
                "ruta insegura dentro del archivo: {path}",
                "unsafe path inside the archive: {path}"
            ),
            Error::DisallowedUrl(url) => tr!(
                "URL de descarga no permitida: {url}",
                "download URL not allowed: {url}"
            ),
            Error::AlreadyRunning => tr!(
                "el servidor ya está en marcha",
                "the server is already running"
            ),
            Error::NotRunning => tr!("el servidor no está en marcha", "the server is not running"),
            Error::MustBeStopped => tr!(
                "detén el servidor antes de hacer esto",
                "stop the server before doing this"
            ),
            Error::PortInUse { port, by: Some(by) } => tr!(
                "el puerto {port} ya lo usa «{by}»: cambia el puerto en Ajustes",
                "port {port} is already used by \"{by}\": change the port in Settings"
            ),
            Error::PortInUse { port, by: None } => tr!(
                "otro programa ya usa el puerto {port}: cierra ese programa o cambia el puerto en Ajustes",
                "another program is already using port {port}: close it or change the port in Settings"
            ),
            Error::Malware(files) => tr!(
                "se encontró malware conocido en {files}. Ponlo en cuarentena desde la pestaña Mods antes de arrancar",
                "known malware was found in {files}. Quarantine it from the Mods tab before starting"
            ),
            Error::Secret(e) => tr!(
                "no se pudo usar el llavero del sistema: {e}",
                "the system keychain could not be used: {e}"
            ),
            Error::Api(e) => e.clone(),
        };
        f.write_str(&msg)
    }
}

/// Los errores de red de reqwest son poco claros para el usuario; damos un
/// mensaje según el caso y añadimos el detalle técnico al final.
fn http_message(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        tr!(
            "la conexión tardó demasiado: {e}",
            "the connection timed out: {e}"
        )
    } else if e.is_connect() {
        tr!(
            "no hay conexión con el servidor de descargas (¿sin internet?): {e}",
            "could not reach the download server (no internet?): {e}"
        )
    } else if let Some(status) = e.status() {
        tr!(
            "el servidor respondió con un error ({status})",
            "the server answered with an error ({status})"
        )
    } else {
        tr!("error de red: {e}", "network error: {e}")
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Http(e) => Some(e),
            Error::Json(e) => Some(e),
            Error::Zip(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Http(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}

impl From<zip::result::ZipError> for Error {
    fn from(e: zip::result::ZipError) -> Self {
        Error::Zip(e)
    }
}

// Tauri necesita que los errores de los comandos sean serializables;
// enviamos el mensaje legible al frontend.
impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
