//! Red: IP local, apertura del puerto en el router (UPnP), túnel para
//! quien está detrás de CGNAT (playit.gg) y comprobaciones de alcance.
//!
//! Solo se expone el puerto del juego: nunca RCON ni query.

pub mod playit;
pub mod upnp;

use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{tr, Error, Result};

/// IP de este equipo en la red local (la que deben usar los amigos conectados
/// al mismo router). No envía ningún paquete: `connect` en UDP solo elige la
/// interfaz de salida.
pub fn local_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("8.8.8.8", 80)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified()).then_some(ip)
}

/// Qué IP tiene el router hacia internet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WanKind {
    /// IP pública: abrir el puerto basta.
    Public,
    /// CGNAT (100.64.0.0/10): la compañía comparte la IP entre clientes y
    /// abrir el puerto en casa no sirve; hace falta el túnel.
    Cgnat,
    /// Otra red privada delante (doble router): tampoco basta con UPnP.
    Private,
}

pub fn classify_wan(ip: IpAddr) -> WanKind {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            if a == 100 && (64..128).contains(&b) {
                WanKind::Cgnat
            } else if v4.is_private()
                || v4.is_link_local()
                || v4.is_loopback()
                || v4.is_unspecified()
            {
                WanKind::Private
            } else {
                WanKind::Public
            }
        }
        IpAddr::V6(_) => WanKind::Public,
    }
}

/// IP pública de esta conexión. Solo se consulta cuando la persona lo pide
/// (le dice la IP a un servicio externo).
pub async fn public_ip(http: &reqwest::Client) -> Result<Ipv4Addr> {
    let mut last = None;
    for url in ["https://api.ipify.org", "https://ipv4.icanhazip.com"] {
        let res = http
            .get(url)
            .timeout(Duration::from_secs(8))
            .send()
            .await
            .and_then(|r| r.error_for_status());
        match res {
            Ok(res) => {
                let text = res.text().await?;
                if let Ok(ip) = text.trim().parse() {
                    return Ok(ip);
                }
                last = Some(Error::Api(tr!(
                    "respuesta inesperada al consultar la IP pública",
                    "unexpected answer when looking up the public IP"
                )));
            }
            Err(e) => last = Some(e.into()),
        }
    }
    Err(last.expect("siempre hay al menos un intento"))
}

#[derive(Deserialize)]
struct McStatus {
    online: bool,
}

/// Pregunta a un servicio externo (mcstatus.io) si el servidor responde
/// desde internet en `host:port`.
pub async fn check_reachable(http: &reqwest::Client, host: &str, port: u16) -> Result<bool> {
    let host_ok = !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
    if !host_ok {
        return Err(Error::InvalidConfig(tr!(
            "dirección no válida: {host}",
            "invalid address: {host}"
        )));
    }
    let status: McStatus = http
        .get(format!(
            "https://api.mcstatus.io/v2/status/java/{host}:{port}"
        ))
        .timeout(Duration::from_secs(15))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(status.online)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wan_kinds() {
        assert_eq!(classify_wan("100.64.0.1".parse().unwrap()), WanKind::Cgnat);
        assert_eq!(
            classify_wan("100.127.255.254".parse().unwrap()),
            WanKind::Cgnat
        );
        assert_eq!(
            classify_wan("100.128.0.1".parse().unwrap()),
            WanKind::Public
        );
        assert_eq!(
            classify_wan("192.168.0.2".parse().unwrap()),
            WanKind::Private
        );
        assert_eq!(
            classify_wan("203.0.113.7".parse().unwrap()),
            WanKind::Public
        );
    }
}
