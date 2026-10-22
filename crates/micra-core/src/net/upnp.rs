//! Apertura del puerto del juego en el router con UPnP (IGD).
//!
//! Solo se abre el puerto TCP del juego, con una concesión de una hora que se
//! renueva mientras el servidor está encendido y se cierra al apagarlo. Si el
//! router solo admite aperturas permanentes, se usa una y se cierra igual al
//! apagar; si Micra se cerrara de golpe, quedaría abierta hasta la próxima vez
//! que se abra y se cierre (nadie escucha en ese puerto mientras tanto).

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::Duration;

use igd_next::aio::tokio::{search_gateway, Tokio};
use igd_next::aio::Gateway;
use igd_next::{AddPortError, GatewayIpVersion, PortMappingProtocol, SearchOptions};
use serde::Serialize;

use super::{classify_wan, WanKind};
use crate::{tr, Error, Result};

/// Duración de cada apertura, en segundos.
pub const LEASE_SECS: u32 = 3600;
/// Cada cuánto renovarla (antes de que caduque).
pub const RENEW_EVERY: Duration = Duration::from_secs(30 * 60);
const SEARCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Lo que la interfaz muestra de una apertura.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortMapping {
    pub port: u16,
    /// IP de este equipo en la red del router.
    pub local_ip: IpAddr,
    /// IP del router hacia internet, si la dio.
    pub external_ip: Option<IpAddr>,
    /// `Cgnat` o `Private`: abrir el puerto no basta, hace falta el túnel.
    pub wan: Option<WanKind>,
    /// El router no admite concesiones con caducidad.
    pub permanent: bool,
}

/// Puerto abierto en el router. Hay que llamar a [`UpnpMapping::renew`] cada
/// [`RENEW_EVERY`] y a [`UpnpMapping::close`] al apagar el servidor.
pub struct UpnpMapping {
    gateway: Gateway<Tokio>,
    local: SocketAddr,
    description: String,
    pub info: PortMapping,
}

impl UpnpMapping {
    /// Busca el router y abre `port` (TCP) hacia este equipo.
    pub async fn open(port: u16, name: &str) -> Result<Self> {
        let gateway = find_gateway().await?;
        let local_ip = local_ip_towards(gateway.addr)?;
        let local = SocketAddr::new(local_ip, port);
        let mut description = description_for(name);
        let mut permanent = false;

        let mut res = add(&gateway, local, LEASE_SECS, &description).await;
        if matches!(res, Err(AddPortError::DescriptionTooLong)) {
            description = "Micra".into();
            res = add(&gateway, local, LEASE_SECS, &description).await;
        }
        if matches!(res, Err(AddPortError::OnlyPermanentLeasesSupported)) {
            permanent = true;
            res = add(&gateway, local, 0, &description).await;
        }
        res.map_err(|e| add_port_error(port, e))?;

        let external_ip = gateway.get_external_ip().await.ok();
        Ok(Self {
            gateway,
            local,
            description,
            info: PortMapping {
                port,
                local_ip,
                external_ip,
                wan: external_ip.map(classify_wan),
                permanent,
            },
        })
    }

    /// Renueva la concesión (y la vuelve a crear si el router se reinició).
    pub async fn renew(&self) -> Result<()> {
        let lease = if self.info.permanent { 0 } else { LEASE_SECS };
        add(&self.gateway, self.local, lease, &self.description)
            .await
            .map_err(|e| add_port_error(self.info.port, e))
    }

    pub async fn close(self) -> Result<()> {
        let port = self.info.port;
        self.gateway
            .remove_port(PortMappingProtocol::TCP, port)
            .await
            .map_err(|e| {
                Error::Api(tr!(
                    "no se pudo cerrar el puerto {port} en el router: {e}",
                    "could not close port {port} on the router: {e}"
                ))
            })
    }
}

async fn add(
    gateway: &Gateway<Tokio>,
    local: SocketAddr,
    lease: u32,
    description: &str,
) -> std::result::Result<(), AddPortError> {
    gateway
        .add_port(
            PortMappingProtocol::TCP,
            local.port(),
            local,
            lease,
            description,
        )
        .await
}

async fn find_gateway() -> Result<Gateway<Tokio>> {
    let mut options = SearchOptions::default();
    options.timeout = Some(SEARCH_TIMEOUT);
    options.gateway_ip_version = GatewayIpVersion::V4;
    search_gateway(options).await.map_err(|e| {
        Error::Api(tr!(
            "el router no respondió a UPnP ({e}). Actívalo en la configuración del router, abre el puerto a mano o usa el túnel",
            "the router did not answer UPnP ({e}). Turn it on in the router settings, forward the port by hand or use the tunnel"
        ))
    })
}

/// IP de este equipo hacia el router (la del adaptador correcto si hay
/// varios, como Wi-Fi y cable). `connect` en UDP no envía nada.
fn local_ip_towards(gateway: SocketAddr) -> Result<IpAddr> {
    let socket = UdpSocket::bind(("0.0.0.0", 0))?;
    socket.connect(gateway)?;
    let ip = socket.local_addr()?.ip();
    if ip.is_unspecified() {
        return Err(Error::Api(tr!(
            "no se encontró la IP de este equipo en la red del router",
            "could not find this computer's IP on the router's network"
        )));
    }
    Ok(ip)
}

/// Descripción que se ve en la página del router: «Micra: <servidor>», en
/// ASCII y corta (muchos routers no aceptan más).
fn description_for(name: &str) -> String {
    let clean: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
        .take(24)
        .collect();
    let clean = clean.trim();
    if clean.is_empty() {
        "Micra".into()
    } else {
        format!("Micra: {clean}")
    }
}

fn add_port_error(port: u16, e: AddPortError) -> Error {
    Error::Api(match e {
        AddPortError::PortInUse => tr!(
            "el router ya tiene el puerto {port} abierto hacia otro equipo de tu red: cambia el puerto del servidor",
            "the router already forwards port {port} to another device on your network: change the server port"
        ),
        AddPortError::ActionNotAuthorized => tr!(
            "el router no deja abrir puertos con UPnP: actívalo en su configuración, abre el puerto a mano o usa el túnel",
            "the router does not allow opening ports with UPnP: turn it on in its settings, forward the port by hand or use the tunnel"
        ),
        other => tr!(
            "no se pudo abrir el puerto {port} en el router: {other}",
            "could not open port {port} on the router: {other}"
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_are_short_ascii() {
        assert_eq!(description_for("Survival"), "Micra: Survival");
        assert_eq!(description_for("Mundo de Ñandú 🌍"), "Micra: Mundo de and");
        assert_eq!(description_for("🌍"), "Micra");
        assert!(description_for(&"x".repeat(100)).len() <= 31);
    }

    #[test]
    fn port_errors_are_explained() {
        let msg = add_port_error(25565, AddPortError::PortInUse).to_string();
        assert!(msg.contains("25565"), "{msg}");
    }
}
