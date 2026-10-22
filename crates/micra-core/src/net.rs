//! Información de red para el panel. La apertura de puertos (UPnP) y el túnel
//! llegarán aquí en la fase 2.

use std::net::{IpAddr, UdpSocket};

/// IP de este equipo en la red local (la que deben usar los amigos conectados
/// al mismo router). No envía ningún paquete: `connect` en UDP solo elige la
/// interfaz de salida.
pub fn local_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("8.8.8.8", 80)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified()).then_some(ip)
}
