//! Edición de `server.properties` conservando comentarios y claves que no tocamos.

use std::path::Path;

use crate::Result;

pub async fn set_properties(server_dir: &Path, values: &[(&str, String)]) -> Result<()> {
    let path = server_dir.join("server.properties");
    let current = tokio::fs::read_to_string(&path).await.unwrap_or_default();
    tokio::fs::write(&path, merge(&current, values)).await?;
    Ok(())
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
}
