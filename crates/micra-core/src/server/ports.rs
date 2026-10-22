//! Puertos de los servidores: detectar conflictos antes de arrancar y
//! proponer uno libre al crear un servidor nuevo.

use std::net::{Ipv4Addr, TcpListener};

/// `true` si ningún programa escucha en `port` (TCP, todas las interfaces).
pub fn is_port_free(port: u16) -> bool {
    TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).is_ok()
}

/// Primer puerto a partir de `start` que no usa ningún otro servidor de
/// Micra (`taken`) y que está libre en el sistema.
pub fn suggest_port(start: u16, taken: &[u16]) -> u16 {
    let mut port = start.max(1024);
    loop {
        if !taken.contains(&port) && is_port_free(port) {
            return port;
        }
        port = if port == u16::MAX { 1024 } else { port + 1 };
        if port == start {
            return start;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_busy_port_and_suggests_next() {
        let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        let busy = listener.local_addr().unwrap().port();
        assert!(!is_port_free(busy));
        let suggested = suggest_port(busy, &[]);
        assert_ne!(suggested, busy);
        let skip_taken = suggest_port(busy + 1, &[busy + 1]);
        assert_ne!(skip_taken, busy + 1);
    }
}
