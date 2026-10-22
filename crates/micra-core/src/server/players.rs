//! Gestión de jugadores: lista blanca, operadores, baneos y expulsiones.
//!
//! - Con el servidor encendido se usan sus propios comandos (`whitelist add`,
//!   `op`, `ban`...), que además actualizan los JSON al momento.
//! - Con el servidor apagado se editan directamente `whitelist.json`,
//!   `ops.json`, `banned-players.json` y `banned-ips.json`, que es lo mismo
//!   que harían esos comandos.
//!
//! Todo lo que llega a la consola se valida antes: un nombre con un salto de
//! línea permitiría inyectar comandos.

use std::net::IpAddr;
use std::path::Path;

use md5::{Digest, Md5};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::fsutil::write_atomic;
use crate::{tr, Error, Result};

pub const WHITELIST: &str = "whitelist.json";
pub const OPS: &str = "ops.json";
pub const BANNED_PLAYERS: &str = "banned-players.json";
pub const BANNED_IPS: &str = "banned-ips.json";
const USER_CACHE: &str = "usercache.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhitelistEntry {
    pub uuid: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpEntry {
    pub uuid: String,
    pub name: String,
    pub level: u8,
    #[serde(default)]
    pub bypasses_player_limit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BannedPlayer {
    pub uuid: String,
    pub name: String,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub source: String,
    #[serde(default = "forever")]
    pub expires: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BannedIp {
    pub ip: String,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub source: String,
    #[serde(default = "forever")]
    pub expires: String,
    #[serde(default)]
    pub reason: String,
}

fn forever() -> String {
    "forever".into()
}

/// Jugadores que han entrado alguna vez (de `usercache.json`), para
/// ofrecerlos al añadir a la lista blanca o a operadores.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownPlayer {
    pub name: String,
    pub uuid: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerLists {
    pub whitelist: Vec<WhitelistEntry>,
    pub ops: Vec<OpEntry>,
    pub banned_players: Vec<BannedPlayer>,
    pub banned_ips: Vec<BannedIp>,
    pub known: Vec<KnownPlayer>,
}

pub async fn read_lists(server_dir: &Path) -> PlayerLists {
    let mut known: Vec<KnownPlayer> = read_json(&server_dir.join(USER_CACHE)).await;
    known.sort_by_key(|p| p.name.to_lowercase());
    known.dedup_by(|a, b| a.uuid == b.uuid);
    PlayerLists {
        whitelist: read_json(&server_dir.join(WHITELIST)).await,
        ops: read_json(&server_dir.join(OPS)).await,
        banned_players: read_json(&server_dir.join(BANNED_PLAYERS)).await,
        banned_ips: read_json(&server_dir.join(BANNED_IPS)).await,
        known,
    }
}

/// Lee una lista JSON; si falta o está dañada, lista vacía (igual que hace
/// el propio servidor).
async fn read_json<T: DeserializeOwned>(path: &Path) -> Vec<T> {
    match tokio::fs::read(path).await {
        Ok(raw) => serde_json::from_slice(&raw).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

async fn write_json<T: Serialize>(path: &Path, list: &[T]) -> Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(list)?).await
}

/// Nombres de Minecraft: 1–16 caracteres, letras, números y `_`.
pub fn validate_name(name: &str) -> Result<&str> {
    let valid = !name.is_empty()
        && name.len() <= 16
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if valid {
        Ok(name)
    } else {
        Err(Error::InvalidConfig(tr!(
            "«{name}» no es un nombre de Minecraft válido (1–16 letras, números o _)",
            "\"{name}\" is not a valid Minecraft name (1–16 letters, numbers or _)"
        )))
    }
}

pub fn validate_ip(ip: &str) -> Result<IpAddr> {
    ip.trim().parse().map_err(|_| {
        Error::InvalidConfig(tr!(
            "«{ip}» no es una dirección IP válida",
            "\"{ip}\" is not a valid IP address"
        ))
    })
}

/// Motivo de baneo/expulsión sin saltos de línea ni caracteres de control,
/// y de longitud razonable.
pub fn sanitize_reason(reason: &str) -> String {
    reason
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect::<String>()
        .trim()
        .to_string()
}

/// UUID que usa un servidor en modo offline: MD5 de `OfflinePlayer:<nombre>`
/// con la versión 3 y la variante RFC 4122 (igual que `UUID.nameUUIDFromBytes`).
pub fn offline_uuid(name: &str) -> String {
    let mut hash: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    hash[6] = (hash[6] & 0x0f) | 0x30;
    hash[8] = (hash[8] & 0x3f) | 0x80;
    dashed_uuid(&hex::encode(hash))
}

/// `069a79f444e94726a5befca90e38aaf5` → `069a79f4-44e9-4726-a5be-fca90e38aaf5`.
pub fn dashed_uuid(hex32: &str) -> String {
    let h = hex32.replace('-', "").to_ascii_lowercase();
    if h.len() != 32 {
        return h;
    }
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

#[derive(Deserialize)]
struct MojangProfile {
    id: String,
    name: String,
}

/// Busca el UUID oficial de una cuenta de Minecraft (modo online). Devuelve
/// también el nombre con las mayúsculas correctas. Si el servicio nuevo de
/// Mojang falla (no si dice que la cuenta no existe), se prueba el antiguo.
pub async fn lookup_profile(http: &reqwest::Client, name: &str) -> Result<KnownPlayer> {
    validate_name(name)?;
    let urls = [
        format!("https://api.minecraftservices.com/minecraft/profile/lookup/name/{name}"),
        format!("https://api.mojang.com/users/profiles/minecraft/{name}"),
    ];
    let mut last_error = None;
    for url in urls {
        let res = match http.get(&url).send().await {
            Ok(res) => res,
            Err(e) => {
                last_error = Some(e.into());
                continue;
            }
        };
        if matches!(
            res.status(),
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::NO_CONTENT
        ) {
            return Err(Error::NotFound(tr!(
                "no existe ninguna cuenta de Minecraft llamada «{name}»",
                "there is no Minecraft account called \"{name}\""
            )));
        }
        match res.error_for_status() {
            Ok(res) => {
                let profile: MojangProfile = res.json().await?;
                return Ok(KnownPlayer {
                    name: profile.name,
                    uuid: dashed_uuid(&profile.id),
                });
            }
            Err(e) => last_error = Some(e.into()),
        }
    }
    Err(last_error.expect("siempre hay al menos un intento"))
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PlayerAction {
    WhitelistAdd,
    WhitelistRemove,
    Op,
    Deop,
    Ban {
        #[serde(default)]
        reason: String,
    },
    Pardon,
    BanIp {
        #[serde(default)]
        reason: String,
    },
    PardonIp,
    Kick {
        #[serde(default)]
        reason: String,
    },
}

impl PlayerAction {
    fn targets_ip(&self) -> bool {
        matches!(self, PlayerAction::BanIp { .. } | PlayerAction::PardonIp)
    }
}

/// Comando de consola equivalente, con el objetivo ya validado.
pub fn command_for(action: &PlayerAction, target: &str) -> Result<String> {
    let target = if action.targets_ip() {
        validate_ip(target)?.to_string()
    } else {
        validate_name(target)?.to_string()
    };
    let with_reason = |base: String, reason: &str| {
        let reason = sanitize_reason(reason);
        if reason.is_empty() {
            base
        } else {
            format!("{base} {reason}")
        }
    };
    Ok(match action {
        PlayerAction::WhitelistAdd => format!("whitelist add {target}"),
        PlayerAction::WhitelistRemove => format!("whitelist remove {target}"),
        PlayerAction::Op => format!("op {target}"),
        PlayerAction::Deop => format!("deop {target}"),
        PlayerAction::Ban { reason } => with_reason(format!("ban {target}"), reason),
        PlayerAction::Pardon => format!("pardon {target}"),
        PlayerAction::BanIp { reason } => with_reason(format!("ban-ip {target}"), reason),
        PlayerAction::PardonIp => format!("pardon-ip {target}"),
        PlayerAction::Kick { reason } => with_reason(format!("kick {target}"), reason),
    })
}

/// Fecha en el formato de los JSON de Minecraft (`2026-10-22 15:04:05 +0200`).
fn now_stamp() -> String {
    chrono::Local::now()
        .format("%Y-%m-%d %H:%M:%S %z")
        .to_string()
}

/// Aplica la acción editando los JSON (servidor apagado). `profile` es el
/// jugador ya resuelto (UUID oficial u offline); no se usa para las IPs.
pub async fn apply_offline(
    server_dir: &Path,
    action: &PlayerAction,
    target: &str,
    profile: Option<&KnownPlayer>,
) -> Result<()> {
    let needs_profile = || {
        profile.cloned().ok_or_else(|| {
            Error::InvalidConfig(tr!(
                "falta el UUID del jugador",
                "the player's UUID is missing"
            ))
        })
    };
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    match action {
        PlayerAction::WhitelistAdd => {
            let p = needs_profile()?;
            let path = server_dir.join(WHITELIST);
            let mut list: Vec<WhitelistEntry> = read_json(&path).await;
            list.retain(|e| !same(&e.name, &p.name) && e.uuid != p.uuid);
            list.push(WhitelistEntry {
                uuid: p.uuid,
                name: p.name,
            });
            write_json(&path, &list).await
        }
        PlayerAction::WhitelistRemove => {
            validate_name(target)?;
            let path = server_dir.join(WHITELIST);
            let mut list: Vec<WhitelistEntry> = read_json(&path).await;
            list.retain(|e| !same(&e.name, target));
            write_json(&path, &list).await
        }
        PlayerAction::Op => {
            let p = needs_profile()?;
            let path = server_dir.join(OPS);
            let mut list: Vec<OpEntry> = read_json(&path).await;
            list.retain(|e| !same(&e.name, &p.name) && e.uuid != p.uuid);
            list.push(OpEntry {
                uuid: p.uuid,
                name: p.name,
                level: 4,
                bypasses_player_limit: false,
            });
            write_json(&path, &list).await
        }
        PlayerAction::Deop => {
            validate_name(target)?;
            let path = server_dir.join(OPS);
            let mut list: Vec<OpEntry> = read_json(&path).await;
            list.retain(|e| !same(&e.name, target));
            write_json(&path, &list).await
        }
        PlayerAction::Ban { reason } => {
            let p = needs_profile()?;
            let path = server_dir.join(BANNED_PLAYERS);
            let mut list: Vec<BannedPlayer> = read_json(&path).await;
            list.retain(|e| !same(&e.name, &p.name) && e.uuid != p.uuid);
            let reason = sanitize_reason(reason);
            list.push(BannedPlayer {
                uuid: p.uuid,
                name: p.name,
                created: now_stamp(),
                source: "Micra".into(),
                expires: forever(),
                reason: if reason.is_empty() {
                    "Banned by an operator.".into()
                } else {
                    reason
                },
            });
            write_json(&path, &list).await
        }
        PlayerAction::Pardon => {
            validate_name(target)?;
            let path = server_dir.join(BANNED_PLAYERS);
            let mut list: Vec<BannedPlayer> = read_json(&path).await;
            list.retain(|e| !same(&e.name, target));
            write_json(&path, &list).await
        }
        PlayerAction::BanIp { reason } => {
            let ip = validate_ip(target)?.to_string();
            ban_ip_offline(server_dir, &ip, reason, "Micra").await
        }
        PlayerAction::PardonIp => {
            let ip = validate_ip(target)?.to_string();
            pardon_ip_offline(server_dir, &ip).await
        }
        PlayerAction::Kick { .. } => Err(Error::NotRunning),
    }
}

pub async fn ban_ip_offline(server_dir: &Path, ip: &str, reason: &str, source: &str) -> Result<()> {
    let path = server_dir.join(BANNED_IPS);
    let mut list: Vec<BannedIp> = read_json(&path).await;
    list.retain(|e| e.ip != ip);
    let reason = sanitize_reason(reason);
    list.push(BannedIp {
        ip: ip.to_string(),
        created: now_stamp(),
        source: source.to_string(),
        expires: forever(),
        reason: if reason.is_empty() {
            "Banned by an operator.".into()
        } else {
            reason
        },
    });
    write_json(&path, &list).await
}

pub async fn pardon_ip_offline(server_dir: &Path, ip: &str) -> Result<()> {
    let path = server_dir.join(BANNED_IPS);
    let mut list: Vec<BannedIp> = read_json(&path).await;
    let before = list.len();
    list.retain(|e| e.ip != ip);
    if list.len() != before {
        write_json(&path, &list).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_ips() {
        assert!(validate_name("Steve_01").is_ok());
        for bad in ["", "a b", "x\nstop", "toolongname_12345", "ñandú"] {
            assert!(validate_name(bad).is_err(), "{bad:?}");
        }
        assert!(validate_ip("192.168.1.10").is_ok());
        assert!(validate_ip("::1").is_ok());
        assert!(validate_ip("1.2.3.4; stop").is_err());
    }

    #[test]
    fn offline_uuid_matches_java() {
        // Valor de referencia de UUID.nameUUIDFromBytes("OfflinePlayer:Notch").
        assert_eq!(
            offline_uuid("Notch"),
            "b50ad385-829d-3141-a216-7e7d7539ba7f"
        );
    }

    #[test]
    fn commands_are_safe() {
        assert_eq!(
            command_for(&PlayerAction::WhitelistAdd, "Alex").unwrap(),
            "whitelist add Alex"
        );
        assert_eq!(
            command_for(
                &PlayerAction::Ban {
                    reason: "griefing\nstop".into()
                },
                "Alex"
            )
            .unwrap(),
            "ban Alex griefingstop"
        );
        assert_eq!(
            command_for(
                &PlayerAction::BanIp {
                    reason: String::new()
                },
                "10.0.0.7"
            )
            .unwrap(),
            "ban-ip 10.0.0.7"
        );
        assert!(command_for(&PlayerAction::Op, "Alex\nstop").is_err());
        assert!(command_for(&PlayerAction::PardonIp, "Alex").is_err());
    }

    #[tokio::test]
    async fn offline_edits() {
        let dir = tempfile::tempdir().unwrap();
        let alex = KnownPlayer {
            name: "Alex".into(),
            uuid: offline_uuid("Alex"),
        };
        apply_offline(dir.path(), &PlayerAction::WhitelistAdd, "Alex", Some(&alex))
            .await
            .unwrap();
        // Repetir no duplica.
        apply_offline(dir.path(), &PlayerAction::WhitelistAdd, "Alex", Some(&alex))
            .await
            .unwrap();
        apply_offline(dir.path(), &PlayerAction::Op, "Alex", Some(&alex))
            .await
            .unwrap();
        apply_offline(
            dir.path(),
            &PlayerAction::BanIp {
                reason: "spam".into(),
            },
            "203.0.113.9",
            None,
        )
        .await
        .unwrap();

        let lists = read_lists(dir.path()).await;
        assert_eq!(lists.whitelist.len(), 1);
        assert_eq!(lists.ops[0].level, 4);
        assert_eq!(lists.banned_ips[0].ip, "203.0.113.9");
        assert_eq!(lists.banned_ips[0].reason, "spam");

        apply_offline(dir.path(), &PlayerAction::WhitelistRemove, "alex", None)
            .await
            .unwrap();
        apply_offline(dir.path(), &PlayerAction::PardonIp, "203.0.113.9", None)
            .await
            .unwrap();
        let lists = read_lists(dir.path()).await;
        assert!(lists.whitelist.is_empty());
        assert!(lists.banned_ips.is_empty());
        assert!(apply_offline(dir.path(), &PlayerAction::Op, "Steve", None)
            .await
            .is_err());
    }
}
