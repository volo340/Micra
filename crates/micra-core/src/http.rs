use std::time::Duration;

use crate::Result;

/// Modrinth exige un User-Agent que identifique la app y un contacto.
pub const USER_AGENT: &str = concat!(
    "volo340/Micra/",
    env!("CARGO_PKG_VERSION"),
    " (github.com/volo340/Micra)"
);

/// Cliente para todo lo que se descarga de internet. Solo HTTPS (también
/// tras una redirección) y con certificados verificados por rustls.
pub fn build_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        // Sin datos durante un minuto = conexión colgada.
        .read_timeout(Duration::from_secs(60))
        .build()?)
}
