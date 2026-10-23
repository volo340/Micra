//! Opciones de `server.properties` que Minecraft 1.21.9 convirtió en reglas de
//! juego del mundo (y las que 1.21.2 quitó sin sustituto). Micra las sigue
//! mostrando como interruptores y, en las versiones nuevas, las aplica con
//! `/gamerule`: en caliente si el servidor está encendido o al arrancar.
//!
//! Respuestas de la consola (siempre en inglés):
//! - `gamerule pvp` → `Gamerule pvp is currently set to: true`
//! - `gamerule pvp false` → `Gamerule pvp is now set to: false`; si lo
//!   escribe un operador en el juego, `[Steve: Gamerule pvp is now set to: false]`.

use crate::server::version::at_least;

/// Propiedad, regla en 1.21.9 y 1.21.10, regla desde 1.21.11.
const MOVED: &[(&str, &str, &str)] = &[
    ("pvp", "pvp", "pvp"),
    (
        "enable-command-block",
        "commandBlocksEnabled",
        "command_blocks_work",
    ),
    (
        "allow-nether",
        "allowEnteringNetherUsingPortals",
        "allow_entering_nether_using_portals",
    ),
    ("spawn-monsters", "spawnMonsters", "spawn_monsters"),
];

/// Quitadas en 1.21.2: el juego ya no las lee.
const REMOVED: &[&str] = &["spawn-animals", "spawn-npcs"];

/// Regla de juego que sustituye a la propiedad en esta versión.
pub fn rule_for(property: &str, mc_version: &str) -> Option<&'static str> {
    let (_, camel, snake) = MOVED.iter().find(|(p, ..)| *p == property)?;
    if at_least(mc_version, "1.21.11") {
        Some(snake)
    } else if at_least(mc_version, "1.21.9") {
        Some(camel)
    } else {
        None
    }
}

/// La propiedad ya no existe en esta versión y no tiene sustituto.
pub fn is_removed(property: &str, mc_version: &str) -> bool {
    REMOVED.contains(&property) && at_least(mc_version, "1.21.2")
}

/// Pares (propiedad, regla) de esta versión.
pub fn moved(mc_version: &str) -> Vec<(&'static str, &'static str)> {
    MOVED
        .iter()
        .filter_map(|(p, ..)| rule_for(p, mc_version).map(|r| (*p, r)))
        .collect()
}

pub fn removed(mc_version: &str) -> Vec<&'static str> {
    REMOVED
        .iter()
        .copied()
        .filter(|p| is_removed(p, mc_version))
        .collect()
}

/// Todas las reglas de juego van activadas en un mundo nuevo.
pub const DEFAULT: bool = true;

/// Nombres con que la regla puede estar guardada en el mundo, también los
/// de una versión anterior que aún no lo ha convertido.
pub fn aliases(property: &str) -> Vec<&'static str> {
    MOVED
        .iter()
        .filter(|(p, ..)| *p == property)
        .flat_map(|(_, camel, snake)| [*camel, *snake])
        .collect()
}

/// Propiedad a la que corresponde una regla (con o sin `minecraft:`).
pub fn property_of(rule: &str) -> Option<&'static str> {
    let rule = rule.strip_prefix("minecraft:").unwrap_or(rule);
    MOVED
        .iter()
        .find(|(_, camel, snake)| *camel == rule || *snake == rule)
        .map(|(p, ..)| *p)
}

pub fn query_command(rule: &str) -> String {
    format!("gamerule {rule}")
}

pub fn set_command(rule: &str, value: bool) -> String {
    format!("gamerule {rule} {value}")
}

/// `Gamerule pvp is currently set to: true` → `("pvp", true)`.
pub fn parse_query(msg: &str) -> Option<(String, bool)> {
    parse(msg, " is currently set to: ")
}

/// `Gamerule pvp is now set to: false`, también el eco que reciben los
/// operadores cuando otro cambia una regla: `[Steve: Gamerule ...]`.
pub fn parse_set(msg: &str) -> Option<(String, bool)> {
    parse(msg, " is now set to: ")
}

pub fn is_query_line(msg: &str) -> bool {
    parse_query(msg).is_some()
}

fn parse(msg: &str, separator: &str) -> Option<(String, bool)> {
    // Solo respuestas de la consola o el eco a operadores: el chat de los
    // jugadores empieza por `<nombre>` y no puede hacerse pasar por ellas.
    let body = match msg.strip_prefix('[') {
        Some(echo) => echo.split_once(": ")?.1.strip_suffix(']')?,
        None => msg,
    };
    let (name, value) = body.strip_prefix("Gamerule ")?.split_once(separator)?;
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    if name.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    let value = match value.trim() {
        "true" => true,
        "false" => false,
        _ => return None,
    };
    Some((name.to_string(), value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_by_version() {
        assert_eq!(rule_for("pvp", "1.21.8"), None);
        assert_eq!(rule_for("pvp", "1.21.9"), Some("pvp"));
        assert_eq!(rule_for("spawn-monsters", "1.21.10"), Some("spawnMonsters"));
        assert_eq!(
            rule_for("spawn-monsters", "1.21.11"),
            Some("spawn_monsters")
        );
        assert_eq!(
            rule_for("allow-nether", "26.3"),
            Some("allow_entering_nether_using_portals")
        );
        assert_eq!(
            rule_for("enable-command-block", "1.21.9"),
            Some("commandBlocksEnabled")
        );
        assert_eq!(rule_for("motd", "26.3"), None);
        assert_eq!(moved("1.20.1"), vec![]);
        assert_eq!(moved("26.1").len(), 4);
    }

    #[test]
    fn removed_properties() {
        assert!(!is_removed("spawn-animals", "1.21.1"));
        assert!(is_removed("spawn-animals", "1.21.2"));
        assert!(is_removed("spawn-npcs", "26.3"));
        assert!(!is_removed("spawn-monsters", "26.3"));
        assert_eq!(removed("1.20.4"), Vec::<&str>::new());
        assert_eq!(removed("1.21.4"), vec!["spawn-animals", "spawn-npcs"]);
    }

    #[test]
    fn names() {
        assert_eq!(
            aliases("allow-nether"),
            vec![
                "allowEnteringNetherUsingPortals",
                "allow_entering_nether_using_portals"
            ]
        );
        assert_eq!(
            property_of("minecraft:command_blocks_work"),
            Some("enable-command-block")
        );
        assert_eq!(property_of("spawnMonsters"), Some("spawn-monsters"));
        assert_eq!(property_of("keepInventory"), None);
        assert_eq!(set_command("pvp", false), "gamerule pvp false");
        assert_eq!(query_command("spawn_monsters"), "gamerule spawn_monsters");
    }

    #[test]
    fn parses_answers() {
        assert_eq!(
            parse_query("Gamerule pvp is currently set to: true"),
            Some(("pvp".into(), true))
        );
        assert_eq!(
            parse_query("Gamerule minecraft:spawn_monsters is currently set to: false"),
            Some(("spawn_monsters".into(), false))
        );
        assert_eq!(
            parse_set("Gamerule commandBlocksEnabled is now set to: false"),
            Some(("commandBlocksEnabled".into(), false))
        );
        assert_eq!(
            parse_set("[Steve: Gamerule pvp is now set to: true]"),
            Some(("pvp".into(), true))
        );
        // Reglas numéricas y textos que no son respuestas.
        assert_eq!(
            parse_query("Gamerule spawnRadius is currently set to: 10"),
            None
        );
        assert_eq!(parse_set("<Steve> Gamerule pvp is now set to: false"), None);
        assert_eq!(parse_set("[Steve] Gamerule pvp is now set to: false"), None);
        assert!(!is_query_line("Gamerule pvp is now set to: true"));
        assert!(is_query_line("Gamerule pvp is currently set to: false"));
    }
}
