//! Medición de los TPS (ticks por segundo; 20 = va perfecto) con el comando
//! que tenga cada servidor:
//!
//! - Forge (y NeoForge 1.20.1): `forge tps` →
//!   `Overall: Mean tick time: 2.345 ms. Mean TPS: 20.000`
//! - NeoForge: `neoforge tps` → `Overall: 20.000 TPS (2.345 ms/tick)`
//! - Vanilla, Fabric y Quilt 1.20.3+: `tick query` →
//!   `Average time per tick: 2.3ms (Target: 50.0ms)`
//!
//! Las respuestas se interceptan para que no llenen la consola.

use crate::server::config::Loader;
use crate::server::version::at_least;

/// Comando que da los TPS en este servidor, si existe.
pub fn tps_command(loader: &Loader, mc_version: &str) -> Option<&'static str> {
    match loader {
        Loader::Forge { .. } => Some("forge tps"),
        Loader::NeoForge { .. } if mc_version == "1.20.1" => Some("forge tps"),
        Loader::NeoForge { .. } => Some("neoforge tps"),
        _ if at_least(mc_version, "1.20.3") => Some("tick query"),
        _ => None,
    }
}

/// Líneas que forman parte de la respuesta (se ocultan de la consola).
pub fn is_tps_line(msg: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "Overall",
        "Dim ",
        "Mean tick time",
        "The game is running",
        "The game is frozen",
        "The game is sprinting",
        "Target tick rate",
        "Average time per tick",
        "Percentiles",
        "Unknown or incomplete command",
        "<--[HERE]",
    ];
    PREFIXES.iter().any(|p| msg.starts_with(p)) || msg.ends_with("<--[HERE]")
}

/// Resultado de una medición.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TpsReading {
    Tps(f32),
    /// El comando no existe en este servidor: no volver a preguntar.
    Unsupported,
    /// No llegó una respuesta reconocible (servidor ocupado, por ejemplo).
    Unknown,
}

pub fn parse_tps(lines: &[String]) -> TpsReading {
    if lines
        .iter()
        .any(|l| l.starts_with("Unknown or incomplete command"))
    {
        return TpsReading::Unsupported;
    }
    // Forge / NeoForge: la línea "Overall".
    if let Some(line) = lines.iter().find(|l| l.starts_with("Overall")) {
        if let Some(tps) = number_after(line, "Mean TPS:") {
            return TpsReading::Tps(tps.min(20.0));
        }
        if let Some(tps) = number_before(line, " TPS") {
            return TpsReading::Tps(tps.min(20.0));
        }
    }
    // `tick query`: tiempo medio por tick y frecuencia objetivo.
    let target = lines
        .iter()
        .find_map(|l| number_after(l, "Target tick rate:"))
        .unwrap_or(20.0);
    if let Some(mspt) = lines
        .iter()
        .find_map(|l| number_after(l, "Average time per tick:"))
    {
        let tps = if mspt <= 0.0 {
            target
        } else {
            (1000.0 / mspt).min(target)
        };
        return TpsReading::Tps(tps);
    }
    TpsReading::Unknown
}

fn parse_number(s: &str) -> Option<f32> {
    let num: String = s
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
        .collect();
    num.replace(',', ".").trim_end_matches('.').parse().ok()
}

fn number_after(line: &str, label: &str) -> Option<f32> {
    let (_, rest) = line.split_once(label)?;
    parse_number(rest)
}

fn number_before(line: &str, label: &str) -> Option<f32> {
    let (head, _) = line.split_once(label)?;
    let start = head
        .rfind(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
        .map_or(0, |i| i + 1);
    parse_number(&head[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn commands_per_loader() {
        let forge = Loader::Forge {
            version: "47.2.0".into(),
        };
        let neo = Loader::NeoForge {
            version: "21.1.77".into(),
        };
        assert_eq!(tps_command(&forge, "1.20.1"), Some("forge tps"));
        assert_eq!(tps_command(&neo, "1.21.1"), Some("neoforge tps"));
        assert_eq!(tps_command(&neo, "1.20.1"), Some("forge tps"));
        assert_eq!(tps_command(&Loader::Vanilla, "1.21.4"), Some("tick query"));
        assert_eq!(tps_command(&Loader::Vanilla, "26.1"), Some("tick query"));
        assert_eq!(tps_command(&Loader::Vanilla, "1.20.1"), None);
    }

    #[test]
    fn parses_each_format() {
        assert_eq!(
            parse_tps(&lines(&[
                "Dim minecraft:overworld (minecraft:overworld): Mean tick time: 3.1 ms. Mean TPS: 20.000",
                "Overall: Mean tick time: 62.500 ms. Mean TPS: 16.000",
            ])),
            TpsReading::Tps(16.0)
        );
        assert_eq!(
            parse_tps(&lines(&[
                "Overall : Mean tick time: 1.2 ms. Mean TPS: 20.000"
            ])),
            TpsReading::Tps(20.0)
        );
        assert_eq!(
            parse_tps(&lines(&["Overall: 19.500 TPS (51.282 ms/tick)"])),
            TpsReading::Tps(19.5)
        );
        assert_eq!(
            parse_tps(&lines(&[
                "The game is running normally",
                "Target tick rate: 20.0 per second.",
                "Average time per tick: 2.3ms (Target: 50.0ms)",
            ])),
            TpsReading::Tps(20.0)
        );
        match parse_tps(&lines(&["Average time per tick: 80.0ms (Target: 50.0ms)"])) {
            TpsReading::Tps(t) => assert!((t - 12.5).abs() < 0.01),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            parse_tps(&lines(&[
                "Unknown or incomplete command, see below for error"
            ])),
            TpsReading::Unsupported
        );
        assert_eq!(parse_tps(&[]), TpsReading::Unknown);
        assert!(is_tps_line("Average time per tick: 2.3ms (Target: 50.0ms)"));
        assert!(!is_tps_line("Steve joined the game"));
    }
}
