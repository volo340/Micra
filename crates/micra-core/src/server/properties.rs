//! Lectura y edición de `server.properties` conservando comentarios y claves
//! que no tocamos, más la validación de los valores que se editan desde la app.

use std::collections::BTreeMap;
use std::path::Path;

use crate::{tr, Error, Result};

pub const FILE: &str = "server.properties";

pub async fn set_properties(server_dir: &Path, values: &[(&str, String)]) -> Result<()> {
    let path = server_dir.join(FILE);
    let current = tokio::fs::read_to_string(&path).await.unwrap_or_default();
    crate::fsutil::write_atomic(&path, merge(&current, values).as_bytes()).await
}

/// Todas las claves del archivo (vacío si todavía no existe: el servidor lo
/// crea en el primer arranque).
pub async fn read_properties(server_dir: &Path) -> Result<BTreeMap<String, String>> {
    match tokio::fs::read_to_string(server_dir.join(FILE)).await {
        Ok(content) => Ok(parse(&content)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(e.into()),
    }
}

pub fn read_properties_sync(server_dir: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(server_dir.join(FILE))
        .map(|c| parse(&c))
        .unwrap_or_default()
}

/// Nombre de la carpeta del mundo (`level-name`, por defecto `world`).
pub fn level_name(props: &BTreeMap<String, String>) -> String {
    props
        .get("level-name")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("world")
        .to_string()
}

/// Valor booleano con el mismo criterio que Minecraft (solo "true" es cierto).
pub fn flag(props: &BTreeMap<String, String>, key: &str, default: bool) -> bool {
    props
        .get(key)
        .map(|v| v.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(default)
}

/// Analiza un `.properties` de Java: comentarios con `#` o `!`, separador
/// `=` o `:`, líneas que continúan con `\` y escapes `\uXXXX`.
pub fn parse(content: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut logical = String::new();
    for raw in content.lines() {
        let line = raw.trim_start();
        if logical.is_empty() && (line.is_empty() || line.starts_with('#') || line.starts_with('!'))
        {
            continue;
        }
        // Un número impar de `\` al final continúa la línea.
        let trailing = line.chars().rev().take_while(|c| *c == '\\').count();
        if trailing % 2 == 1 {
            logical.push_str(&line[..line.len() - 1]);
            continue;
        }
        logical.push_str(line);
        if let Some((k, v)) = split_key_value(&logical) {
            out.insert(unescape(&k), unescape(v.trim_start()));
        }
        logical.clear();
    }
    out
}

fn split_key_value(line: &str) -> Option<(String, &str)> {
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '=' | ':' => return Some((line[..i].trim_end().to_string(), &line[i + 1..])),
            c if c.is_whitespace() => {
                // "clave valor" también es válido; "clave = valor" también.
                let rest = line[i..].trim_start();
                let rest = rest
                    .strip_prefix('=')
                    .or_else(|| rest.strip_prefix(':'))
                    .unwrap_or(rest);
                return Some((line[..i].to_string(), rest));
            }
            _ => {}
        }
    }
    (!line.is_empty()).then(|| (line.to_string(), ""))
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut pending_high: Option<u16> = None;
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('f') => out.push('\u{c}'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                let Ok(unit) = u16::from_str_radix(&hex, 16) else {
                    continue;
                };
                // Pares sustitutos UTF-16 (emojis) llegan como dos \u.
                if (0xD800..0xDC00).contains(&unit) {
                    pending_high = Some(unit);
                    continue;
                }
                let units: Vec<u16> = match pending_high.take() {
                    Some(high) => vec![high, unit],
                    None => vec![unit],
                };
                out.extend(char::decode_utf16(units).map(|r| r.unwrap_or('\u{fffd}')));
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

fn merge(current: &str, values: &[(&str, String)]) -> String {
    let mut pending: Vec<(&str, &String)> = values.iter().map(|(k, v)| (*k, v)).collect();
    let mut out = String::new();
    for line in current.lines() {
        let key = line.split_once('=').map(|(k, _)| k.trim());
        match key.and_then(|k| pending.iter().position(|(pk, _)| *pk == k)) {
            Some(i) if !line.trim_start().starts_with('#') => {
                let (k, v) = pending.remove(i);
                out.push_str(&format!("{k}={}\n", escape(v)));
            }
            _ => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    for (k, v) in pending {
        out.push_str(&format!("{k}={}\n", escape(v)));
    }
    out
}

/// `server.properties` es un archivo `.properties` de Java: los caracteres no
/// ASCII (p. ej. "ñ" en el MOTD) deben ir como `\uXXXX`.
fn escape(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            c => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out
}

/// Tipo de cada propiedad que la app sabe editar.
enum Kind {
    Bool,
    Int(i64, i64),
    Choice(&'static [&'static str]),
    Text(usize),
}

fn kind_of(key: &str) -> Option<Kind> {
    use Kind::*;
    Some(match key {
        "difficulty" => Choice(&["peaceful", "easy", "normal", "hard"]),
        "gamemode" => Choice(&["survival", "creative", "adventure", "spectator"]),
        "level-type" => Choice(&[
            "minecraft:normal",
            "minecraft:flat",
            "minecraft:large_biomes",
            "minecraft:amplified",
            "minecraft:single_biome_surface",
            "normal",
            "flat",
            "large_biomes",
            "amplified",
            "default",
        ]),
        "pvp"
        | "online-mode"
        | "white-list"
        | "enforce-whitelist"
        | "hardcore"
        | "allow-flight"
        | "allow-nether"
        | "spawn-monsters"
        | "spawn-animals"
        | "spawn-npcs"
        | "generate-structures"
        | "force-gamemode"
        | "enable-command-block"
        | "enable-rcon"
        | "enable-query"
        | "enable-status"
        | "hide-online-players"
        | "enforce-secure-profile"
        | "prevent-proxy-connections"
        | "broadcast-console-to-ops"
        | "broadcast-rcon-to-ops"
        | "sync-chunk-writes"
        | "use-native-transport"
        | "log-ips"
        | "accepts-transfers"
        | "require-resource-pack" => Bool,
        "max-players" => Int(1, 1000),
        "view-distance" => Int(2, 32),
        "simulation-distance" => Int(2, 32),
        "spawn-protection" => Int(0, 10_000),
        "server-port" | "rcon.port" | "query.port" => Int(1024, 65535),
        "player-idle-timeout" => Int(0, 1440),
        "max-world-size" => Int(1, 29_999_984),
        "op-permission-level" | "function-permission-level" => Int(1, 4),
        "rate-limit" => Int(0, 100_000),
        "max-tick-time" => Int(-1, i64::MAX),
        "network-compression-threshold" => Int(-1, 65535),
        "entity-broadcast-range-percentage" => Int(10, 1000),
        "motd" => Text(150),
        "level-seed" => Text(100),
        "level-name" => Text(80),
        "resource-pack" | "resource-pack-prompt" => Text(500),
        "resource-pack-sha1" => Text(40),
        "server-ip" => Text(64),
        "rcon.password" => Text(128),
        _ => return None,
    })
}

/// Valida un valor que llega desde el editor. Las claves desconocidas se
/// aceptan si tienen un nombre razonable (mods y versiones nuevas añaden
/// las suyas), pero nunca un salto de línea.
pub fn validate(key: &str, value: &str) -> Result<()> {
    let key_ok = !key.is_empty()
        && key.len() <= 64
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'));
    if !key_ok {
        return Err(Error::InvalidConfig(tr!(
            "nombre de propiedad no válido: {key}",
            "invalid property name: {key}"
        )));
    }
    if value.chars().any(|c| c == '\n' || c == '\r') {
        return Err(Error::InvalidConfig(tr!(
            "«{key}» no puede tener saltos de línea",
            "\"{key}\" cannot contain line breaks"
        )));
    }
    let invalid = || {
        Error::InvalidConfig(tr!(
            "valor no válido para «{key}»: {value}",
            "invalid value for \"{key}\": {value}"
        ))
    };
    match kind_of(key) {
        Some(Kind::Bool) if !matches!(value, "true" | "false") => Err(invalid()),
        Some(Kind::Int(min, max)) => match value.trim().parse::<i64>() {
            Ok(n) if (min..=max).contains(&n) => Ok(()),
            _ => Err(Error::InvalidConfig(tr!(
                "«{key}» debe ser un número entre {min} y {max}",
                "\"{key}\" must be a number between {min} and {max}"
            ))),
        },
        Some(Kind::Choice(options)) if !options.contains(&value) => Err(invalid()),
        Some(Kind::Text(max)) if value.chars().count() > max => Err(Error::InvalidConfig(tr!(
            "«{key}» admite como mucho {max} caracteres",
            "\"{key}\" allows at most {max} characters"
        ))),
        _ => Ok(()),
    }
}

/// Contraseña aleatoria fuerte (para RCON): 32 caracteres alfanuméricos.
pub fn random_password() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut out = String::with_capacity(32);
    let mut buf = [0u8; 64];
    while out.len() < 32 {
        getrandom::fill(&mut buf).expect("el sistema debe dar números aleatorios");
        for b in buf {
            // Rechazo para no sesgar: 256 no es múltiplo del alfabeto.
            let limit = 256 - (256 % ALPHABET.len());
            if (b as usize) < limit && out.len() < 32 {
                out.push(ALPHABET[b as usize % ALPHABET.len()] as char);
            }
        }
    }
    out
}

/// Una contraseña de RCON se considera débil si es corta.
pub fn is_weak_password(password: &str) -> bool {
    password.chars().count() < 16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_unknown_and_updates_known() {
        let current = "#Minecraft server properties\nserver-port=25565\ndifficulty=easy\n";
        let out = merge(
            current,
            &[
                ("server-port", "25570".into()),
                ("motd", "Año nuevo".into()),
            ],
        );
        assert_eq!(
            out,
            "#Minecraft server properties\nserver-port=25570\ndifficulty=easy\nmotd=A\\u00f1o nuevo\n"
        );
    }

    #[test]
    fn parses_java_properties() {
        let content = "#Minecraft server properties\n\
                       #Thu Oct 22 15:00:00 CEST 2026\n\
                       motd=A\\u00f1o \\u2764\\ufe0f en el servidor\n\
                       level-seed=\n\
                       server-ip\\:x=1\n\
                       long=uno \\\n   dos\n\
                       spaced = valor con espacios\n\
                       emoji=\\ud83d\\ude00\n\
                       ! comentario\n";
        let props = parse(content);
        assert_eq!(props["motd"], "Año ❤️ en el servidor");
        assert_eq!(props["level-seed"], "");
        assert_eq!(props["server-ip:x"], "1");
        assert_eq!(props["long"], "uno dos");
        assert_eq!(props["spaced"], "valor con espacios");
        assert_eq!(props["emoji"], "😀");
        assert_eq!(props.len(), 6);
    }

    #[test]
    fn roundtrip_escape() {
        let out = merge("", &[("motd", "Hola ñandú 😀 \\o/".into())]);
        assert_eq!(parse(&out)["motd"], "Hola ñandú 😀 \\o/");
    }

    #[test]
    fn validation() {
        assert!(validate("difficulty", "hard").is_ok());
        assert!(validate("difficulty", "imposible").is_err());
        assert!(validate("pvp", "yes").is_err());
        assert!(validate("max-players", "20").is_ok());
        assert!(validate("max-players", "0").is_err());
        assert!(validate("server-port", "80").is_err());
        assert!(validate("motd", "hola\nstop").is_err());
        assert!(validate("some-mod.option", "x").is_ok());
        assert!(validate("bad key", "x").is_err());
    }

    #[test]
    fn passwords() {
        let a = random_password();
        let b = random_password();
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
        assert!(is_weak_password("minecraft"));
        assert!(!is_weak_password(&a));
    }

    #[test]
    fn helpers() {
        let props = parse("level-name=mundo\nonline-mode=TRUE\n");
        assert_eq!(level_name(&props), "mundo");
        assert!(flag(&props, "online-mode", false));
        assert!(!flag(&props, "pvp", false));
        assert_eq!(level_name(&BTreeMap::new()), "world");
    }
}
