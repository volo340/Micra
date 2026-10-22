//! Núcleo de Micra.
//!
//! Este crate no depende de Tauri: contiene toda la lógica de negocio
//! (descargas, runtimes de Java, proveedores de modpacks y gestión del proceso
//! del servidor) para que se pueda probar y reutilizar sin interfaz gráfica.

pub mod archive;
pub mod download;
pub mod error;
pub mod hardware;
pub mod http;
pub mod java;
pub mod paths;
pub mod provision;
pub mod providers;
pub mod server;

pub use error::{Error, Result};
