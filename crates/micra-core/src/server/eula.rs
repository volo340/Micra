use std::path::Path;

use crate::{Error, Result};

/// Escribe `eula.txt` con `eula=true`.
///
/// Importante: el EULA lo acepta la persona, no la app. El asistente debe
/// mostrar el enlace y una casilla; solo si la marca llega aquí `accepted = true`.
/// Así el paso es "automático" (sin editar archivos) pero sigue siendo legal.
pub async fn write_eula(server_dir: &Path, accepted: bool) -> Result<()> {
    if !accepted {
        return Err(Error::InvalidConfig(
            "debes aceptar el EULA de Minecraft (https://aka.ms/MinecraftEULA) para continuar"
                .into(),
        ));
    }
    let content = "# Aceptado por el usuario desde Micra.\n\
                   # https://aka.ms/MinecraftEULA\n\
                   eula=true\n";
    tokio::fs::create_dir_all(server_dir).await?;
    tokio::fs::write(server_dir.join("eula.txt"), content).await?;
    Ok(())
}
