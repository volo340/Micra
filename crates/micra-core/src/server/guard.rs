//! Protección básica contra ataques: bloqueo temporal de IPs que fallan
//! demasiados intentos de entrada y límite de jugadores por IP.
//!
//! Minecraft no tiene nada de esto, así que se hace leyendo la consola:
//!
//! - `Steve[/1.2.3.4:51234] logged in with entity id 7 at (...)` → entrada.
//! - `Disconnecting Steve (/1.2.3.4:51234): You are not white-listed...`,
//!   `Steve (/1.2.3.4:5) lost connection: ...` o `/1.2.3.4:5 lost connection: ...`
//!   antes de entrar → intento fallido.
//!
//! Las IPs locales (el propio equipo, la red de casa y el túnel, que llega
//! como 127.0.0.1) nunca se bloquean. No sustituye a una protección DDoS.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::path::Path;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::fsutil::write_atomic;
use crate::server::config::GuardPolicy;
use crate::Result;

const STATE_FILE: &str = ".micra/guard.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardAction {
    /// Bloquear la IP (`ban-ip`). `until = None` es permanente.
    BanIp {
        ip: IpAddr,
        attempts: usize,
        until: Option<DateTime<Utc>>,
    },
    /// Fin de un bloqueo temporal (`pardon-ip`).
    PardonIp { ip: IpAddr },
    /// Demasiados jugadores desde la misma IP.
    Kick { player: String, ip: IpAddr },
}

/// Bloqueos temporales vigentes, guardados para levantarlos aunque Micra
/// se cierre entre medias.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuardState {
    pub bans: Vec<TempBan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TempBan {
    pub ip: IpAddr,
    pub until: DateTime<Utc>,
}

impl GuardState {
    pub async fn load(server_dir: &Path) -> Self {
        tokio::fs::read(server_dir.join(STATE_FILE))
            .await
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or_default()
    }

    pub async fn save(&self, server_dir: &Path) -> Result<()> {
        crate::fsutil::internal_dir(server_dir, ".micra")?;
        write_atomic(
            &server_dir.join(STATE_FILE),
            &serde_json::to_vec_pretty(self)?,
        )
        .await
    }

    /// Saca los bloqueos ya vencidos.
    pub fn take_expired(&mut self, now: DateTime<Utc>) -> Vec<IpAddr> {
        let (expired, active): (Vec<_>, Vec<_>) = self.bans.drain(..).partition(|b| b.until <= now);
        self.bans = active;
        expired.into_iter().map(|b| b.ip).collect()
    }
}

pub struct Guard {
    policy: GuardPolicy,
    attempts: HashMap<IpAddr, VecDeque<Instant>>,
    online: HashMap<String, IpAddr>,
    pub state: GuardState,
}

impl Guard {
    pub fn new(policy: GuardPolicy, state: GuardState) -> Self {
        Self {
            policy,
            attempts: HashMap::new(),
            online: HashMap::new(),
            state,
        }
    }

    /// Procesa una línea de la consola (ya sin el prefijo de hora e hilo).
    pub fn on_line(&mut self, msg: &str, now: Instant, wall: DateTime<Utc>) -> Vec<GuardAction> {
        if !self.policy.enabled {
            return Vec::new();
        }
        if let Some((player, ip)) = parse_login(msg) {
            self.online.insert(player.clone(), ip);
            if self.policy.max_per_ip > 0 && !is_local(&ip) {
                let same = self.online.values().filter(|i| **i == ip).count();
                if same > self.policy.max_per_ip as usize {
                    return vec![GuardAction::Kick { player, ip }];
                }
            }
            return Vec::new();
        }
        if let Some(player) = parse_leave(msg) {
            self.online.remove(&player);
            return Vec::new();
        }
        let Some(ip) = parse_failure(msg) else {
            return Vec::new();
        };
        if is_local(&ip) || self.state.bans.iter().any(|b| b.ip == ip) {
            return Vec::new();
        }
        let window = Duration::from_secs(self.policy.window_secs as u64);
        let list = self.attempts.entry(ip).or_default();
        list.push_back(now);
        while list
            .front()
            .is_some_and(|t| now.duration_since(*t) > window)
        {
            list.pop_front();
        }
        if list.len() < self.policy.max_attempts as usize {
            return Vec::new();
        }
        let attempts = list.len();
        self.attempts.remove(&ip);
        let until = (self.policy.ban_minutes > 0)
            .then(|| wall + chrono::Duration::minutes(self.policy.ban_minutes as i64));
        if let Some(until) = until {
            self.state.bans.push(TempBan { ip, until });
        }
        vec![GuardAction::BanIp {
            ip,
            attempts,
            until,
        }]
    }

    /// Llamar cada cierto tiempo: levanta los bloqueos vencidos.
    pub fn tick(&mut self, wall: DateTime<Utc>) -> Vec<GuardAction> {
        self.state
            .take_expired(wall)
            .into_iter()
            .map(|ip| GuardAction::PardonIp { ip })
            .collect()
    }
}

/// IPs que nunca se bloquean: este equipo, la red local y el túnel.
pub fn is_local(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_local(&IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || (first & 0xfe00) == 0xfc00 // ULA
                || (first & 0xffc0) == 0xfe80 // enlace local
        }
    }
}

/// `/1.2.3.4:5678`, `/[::1]:5678` o `/0:0:0:0:0:0:0:1:5678` → IP.
fn parse_socket(text: &str) -> Option<IpAddr> {
    let s = text.trim().strip_prefix('/')?;
    let host = if let Some(rest) = s.strip_prefix('[') {
        rest.split(']').next()?
    } else {
        s.rsplit_once(':').map_or(s, |(h, _)| h)
    };
    host.parse().ok()
}

fn is_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 16 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `Steve[/1.2.3.4:51234] logged in with entity id 7 at (...)`
fn parse_login(msg: &str) -> Option<(String, IpAddr)> {
    let (head, _) = msg.split_once("] logged in with entity id ")?;
    let (name, addr) = head.split_once('[')?;
    if !is_name(name) {
        return None;
    }
    Some((name.to_string(), parse_socket(addr)?))
}

/// `Steve left the game` o `Steve lost connection: ...` (sin IP).
fn parse_leave(msg: &str) -> Option<String> {
    let name = msg
        .strip_suffix(" left the game")
        .or_else(|| msg.split_once(" lost connection: ").map(|(n, _)| n))?;
    is_name(name).then(|| name.to_string())
}

/// IP de un intento de entrada fallido.
fn parse_failure(msg: &str) -> Option<IpAddr> {
    // `/1.2.3.4:5 lost connection: ...`
    if msg.starts_with('/') {
        let (addr, _) = msg.split_once(" lost connection")?;
        return parse_socket(addr);
    }
    let is_disconnect = msg.starts_with("Disconnecting ");
    let rest = msg.strip_prefix("Disconnecting ").unwrap_or(msg);
    // `Disconnecting /1.2.3.4:5: Took too long to log in`
    if is_disconnect && rest.starts_with('/') {
        let addr = rest.split(": ").next()?;
        return parse_socket(addr);
    }
    // `<quién> (/1.2.3.4:5)` seguido de `: motivo` o ` lost connection: motivo`.
    let (who, after) = rest.split_once(" (/")?;
    let (addr, tail) = after.split_once(')')?;
    let who_ok = is_name(who)
        || who.starts_with("com.mojang.authlib.GameProfile")
        || who.starts_with("GameProfile");
    let shape_ok = if is_disconnect {
        tail.starts_with(": ")
    } else {
        tail.starts_with(" lost connection")
    };
    if !who_ok || !shape_ok {
        return None;
    }
    parse_socket(&format!("/{addr}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> GuardPolicy {
        GuardPolicy {
            enabled: true,
            max_attempts: 3,
            window_secs: 60,
            ban_minutes: 30,
            max_per_ip: 2,
        }
    }

    #[test]
    fn parses_console_lines() {
        assert_eq!(
            parse_login(
                "Steve[/203.0.113.5:51234] logged in with entity id 157 at (1.5, 64.0, -3.2)"
            ),
            Some(("Steve".into(), "203.0.113.5".parse().unwrap()))
        );
        assert_eq!(
            parse_login("Alex[/[2001:db8::1]:4000] logged in with entity id 1 at (0, 0, 0)"),
            Some(("Alex".into(), "2001:db8::1".parse().unwrap()))
        );
        let ip: IpAddr = "198.51.100.7".parse().unwrap();
        for line in [
            "Disconnecting Steve (/198.51.100.7:5555): You are not white-listed on this server!",
            "Steve (/198.51.100.7:5555) lost connection: You are banned from this server.",
            "com.mojang.authlib.GameProfile@1a2b[id=<null>,name=Steve,properties={},legacy=false] (/198.51.100.7:5555) lost connection: Disconnected",
            "/198.51.100.7:5555 lost connection: Internal Exception: java.io.IOException",
            "Disconnecting /198.51.100.7:5555: Took too long to log in",
        ] {
            assert_eq!(parse_failure(line), Some(ip), "{line}");
        }
        // Mensajes de chat que imitan el formato no cuentan.
        assert_eq!(
            parse_failure("<Troll> Disconnecting Steve (/198.51.100.7:5555): x"),
            None
        );
        assert_eq!(
            parse_failure("[Server] Steve (/198.51.100.7:1) lost connection: x"),
            None
        );
        assert_eq!(parse_leave("Steve left the game"), Some("Steve".into()));
        assert_eq!(
            parse_leave("Steve lost connection: Disconnected"),
            Some("Steve".into())
        );
    }

    #[test]
    fn bans_after_repeated_failures_and_pardons_later() {
        let mut guard = Guard::new(policy(), GuardState::default());
        let t0 = Instant::now();
        let wall = Utc::now();
        let line = "Disconnecting Bot (/198.51.100.7:1): You are not white-listed on this server!";
        assert!(guard.on_line(line, t0, wall).is_empty());
        assert!(guard
            .on_line(line, t0 + Duration::from_secs(1), wall)
            .is_empty());
        let actions = guard.on_line(line, t0 + Duration::from_secs(2), wall);
        assert!(matches!(
            actions.as_slice(),
            [GuardAction::BanIp {
                attempts: 3,
                until: Some(_),
                ..
            }]
        ));
        assert_eq!(guard.state.bans.len(), 1);
        // Ya bloqueada: no se repite.
        assert!(guard
            .on_line(line, t0 + Duration::from_secs(3), wall)
            .is_empty());
        assert!(guard.tick(wall + chrono::Duration::minutes(10)).is_empty());
        assert_eq!(
            guard.tick(wall + chrono::Duration::minutes(31)),
            vec![GuardAction::PardonIp {
                ip: "198.51.100.7".parse().unwrap()
            }]
        );
    }

    #[test]
    fn failures_outside_the_window_do_not_add_up() {
        let mut guard = Guard::new(policy(), GuardState::default());
        let t0 = Instant::now();
        let wall = Utc::now();
        let line = "/198.51.100.9:1 lost connection: Disconnected";
        for i in 0..6 {
            let at = t0 + Duration::from_secs(i * 40);
            assert!(guard.on_line(line, at, wall).is_empty());
        }
    }

    #[test]
    fn never_bans_local_addresses() {
        let mut guard = Guard::new(policy(), GuardState::default());
        let t0 = Instant::now();
        for ip in [
            "127.0.0.1",
            "192.168.1.20",
            "10.0.0.5",
            "[::1]",
            "[fd00::2]",
        ] {
            let line = format!("/{ip}:1 lost connection: x");
            for i in 0..10 {
                assert!(guard
                    .on_line(&line, t0 + Duration::from_millis(i), Utc::now())
                    .is_empty());
            }
        }
        assert!(is_local(&"::ffff:192.168.0.1".parse().unwrap()));
        assert!(!is_local(&"100.64.0.1".parse().unwrap()));
    }

    #[test]
    fn limits_players_per_ip() {
        let mut guard = Guard::new(policy(), GuardState::default());
        let t0 = Instant::now();
        let wall = Utc::now();
        for name in ["A", "B"] {
            let line = format!("{name}[/203.0.113.9:1] logged in with entity id 1 at (0, 0, 0)");
            assert!(guard.on_line(&line, t0, wall).is_empty());
        }
        let actions = guard.on_line(
            "C[/203.0.113.9:2] logged in with entity id 3 at (0, 0, 0)",
            t0,
            wall,
        );
        assert!(matches!(actions.as_slice(), [GuardAction::Kick { player, .. }] if player == "C"));
        guard.on_line("C lost connection: Kicked", t0, wall);
        guard.on_line("A left the game", t0, wall);
        assert!(guard
            .on_line(
                "D[/203.0.113.9:3] logged in with entity id 4 at (0, 0, 0)",
                t0,
                wall
            )
            .is_empty());
    }

    #[test]
    fn disabled_guard_does_nothing() {
        let mut p = policy();
        p.enabled = false;
        let mut guard = Guard::new(p, GuardState::default());
        let t0 = Instant::now();
        for i in 0..10 {
            assert!(guard
                .on_line(
                    "/198.51.100.7:1 lost connection: x",
                    t0 + Duration::from_millis(i),
                    Utc::now()
                )
                .is_empty());
        }
    }
}
