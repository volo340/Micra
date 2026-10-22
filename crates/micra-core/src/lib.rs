//! Núcleo de Micra.
//!
//! Este crate no depende de Tauri: contiene toda la lógica de negocio
//! (descargas, runtimes de Java, proveedores de modpacks y gestión del proceso
//! del servidor) para que se pueda probar y reutilizar sin interfaz gráfica.

pub mod archive;
pub mod download;
pub mod error;
pub mod fsutil;
pub mod hardware;
pub mod http;
pub mod i18n;
pub mod import;
pub mod java;
pub mod net;
pub mod notify;
pub mod paths;
pub mod providers;
pub mod provision;
pub mod secrets;
pub mod server;
pub mod settings;

pub use error::{Error, Result};
