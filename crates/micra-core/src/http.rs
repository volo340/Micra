use std::time::Duration;

use crate::Result;

/// Modrinth exige un User-Agent que identifique la app y un contacto.
pub const USER_AGENT: &str = concat!(
    "volo340/Micra/",
    env!("CARGO_PKG_VERSION"),
    " (github.com/volo340/Micra)"
);

pub fn build_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .build()?)
}
