//! Permisos mínimos para el proceso del servidor.
//!
//! Un mod malicioso se ejecuta con los mismos permisos que el servidor. Para
//! limitar el daño:
//!
//! - **Linux**: [Landlock](https://docs.kernel.org/userspace-api/landlock.html)
//!   (kernel 5.13 o posterior). El servidor solo puede escribir en su propia
//!   carpeta y en `/tmp`, y solo puede leer su carpeta, el Java de Micra y los
//!   archivos del sistema. Tu carpeta personal (navegadores, Discord, claves
//!   SSH, otros servidores, los secretos de Micra) queda fuera de su alcance.
//! - **Windows**: un *Job Object* que impide leer o cambiar el portapapeles,
//!   tocar la configuración del sistema o cerrar la sesión, y que cierra el
//!   servidor si Micra se cierra de golpe. De forma opcional (experimental),
//!   nivel de integridad bajo: el servidor no puede escribir fuera de su
//!   carpeta, lo que bloquea que un mod se instale para arrancar con Windows.
//!
//! El servidor y sus mods siguen teniendo red: la necesitan para funcionar.

use serde::Serialize;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
pub mod windows;

/// Lo que la interfaz muestra sobre el aislamiento del servidor en marcha.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SandboxStatus {
    /// Desactivado en los ajustes del servidor.
    Disabled,
    /// Linux: Landlock activo con esta versión de la ABI.
    Landlock { abi: u32 },
    /// Windows: Job Object; `low_integrity` si además va con integridad baja.
    #[serde(rename_all = "camelCase")]
    JobObject { low_integrity: bool },
    /// El sistema no lo permite (p. ej. kernel sin Landlock).
    Unavailable { reason: String },
}

/// Subcarpetas privadas del servidor que sustituyen a la carpeta personal y
/// a la temporal del sistema mientras está aislado.
pub const HOME_DIR: &str = ".micra/home";
pub const TMP_DIR: &str = ".micra/tmp";
