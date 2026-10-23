//! Ajustes que guarda el propio mundo y que `server.properties` ya no cambia
//! una vez creado: el modo extremo y, desde 1.21.9, las reglas de juego que
//! sustituyen a algunas propiedades.
//!
//! - `level.dat`: `Data.hardcore` hasta 1.21.11 y
//!   `Data.difficulty_settings.hardcore` desde 26.1; las reglas, en
//!   `Data.GameRules` hasta 1.21.11.
//! - Desde 26.1 las reglas están en `data/minecraft/game_rules.dat`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::nbt::{self, Tag};
use crate::server::{gamerules, properties};
use crate::{tr, Error, Result};

pub const LEVEL_DAT: &str = "level.dat";

/// Carpeta del mundo según `server.properties`.
pub fn world_dir(server_dir: &Path, props: &BTreeMap<String, String>) -> PathBuf {
    server_dir.join(properties::level_name(props))
}

fn game_rules_file(world: &Path) -> PathBuf {
    world.join("data").join("minecraft").join("game_rules.dat")
}

async fn load(path: &Path) -> Option<Tag> {
    let bytes = tokio::fs::read(path).await.ok()?;
    match nbt::read_gzip(&bytes) {
        Ok((_, root)) => Some(root),
        Err(e) => {
            tracing::warn!("no se pudo leer {}: {e}", path.display());
            None
        }
    }
}

/// Modo extremo del mundo (`None` si aún no existe).
pub async fn read_hardcore(world: &Path) -> Option<bool> {
    hardcore_of(&load(&world.join(LEVEL_DAT)).await?)
}

fn hardcore_of(root: &Tag) -> Option<bool> {
    let data = root.get("Data")?;
    data.get("difficulty_settings")
        .and_then(|d| d.get("hardcore"))
        .or_else(|| data.get("hardcore"))?
        .as_bool()
}

/// Cambia el modo extremo de un mundo ya creado. Solo con el servidor
/// apagado: al apagarse vuelve a escribir `level.dat` con lo que tenga en
/// memoria. Devuelve `false` si el mundo aún no existe.
pub async fn set_hardcore(world: &Path, value: bool) -> Result<bool> {
    let path = world.join(LEVEL_DAT);
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    let (name, mut root) = nbt::read_gzip(&bytes)?;
    set_hardcore_in(&mut root, value)?;
    crate::fsutil::write_atomic(&path, &nbt::write_gzip(&name, &root)?).await?;
    Ok(true)
}

fn set_hardcore_in(root: &mut Tag, value: bool) -> Result<()> {
    let Some(data @ Tag::Compound(_)) = root.get_mut("Data") else {
        return Err(Error::InvalidConfig(tr!(
            "level.dat no tiene el formato de un mundo de Minecraft",
            "level.dat does not look like a Minecraft world"
        )));
    };
    let flag = Tag::Byte(i8::from(value));
    match data.get_mut("difficulty_settings") {
        Some(settings @ Tag::Compound(_)) => settings.set("hardcore", flag),
        _ => data.set("hardcore", flag),
    }
    Ok(())
}

/// Valores guardados en el mundo de las reglas que sustituyen a estas
/// propiedades. Las que falten no salen: el mundo no existe todavía o la
/// regla nunca se cambió.
pub async fn read_rules(world: &Path, props: &[&str]) -> BTreeMap<String, bool> {
    let mut out = BTreeMap::new();
    if props.is_empty() {
        return out;
    }
    if let Some(root) = load(&game_rules_file(world)).await {
        collect_rules(&root, props, &mut out);
    }
    if out.len() < props.len() {
        if let Some(root) = load(&world.join(LEVEL_DAT)).await {
            let data = root.get("Data");
            if let Some(rules) =
                data.and_then(|d| d.get("GameRules").or_else(|| d.get("game_rules")))
            {
                collect_rules(rules, props, &mut out);
            }
        }
    }
    out
}

/// Busca por todo el árbol: según la versión, las reglas van con o sin
/// `minecraft:`, como texto o como byte y dentro o no de `data`.
fn collect_rules(tag: &Tag, props: &[&str], out: &mut BTreeMap<String, bool>) {
    let Tag::Compound(entries) = tag else {
        return;
    };
    for (name, value) in entries {
        if matches!(value, Tag::Compound(_)) {
            collect_rules(value, props, out);
            continue;
        }
        let Some(prop) = std::str::from_utf8(name)
            .ok()
            .and_then(gamerules::property_of)
        else {
            continue;
        };
        if let Some(v) = value.as_bool().filter(|_| props.contains(&prop)) {
            out.entry(prop.to_string()).or_insert(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbt::tests::{compound, string};

    /// `level.dat` de 1.21.9: reglas como texto y `hardcore` en `Data`.
    fn level_1_21_9(hardcore: i8) -> Tag {
        compound(vec![(
            "Data",
            compound(vec![
                ("LevelName", string("world")),
                ("hardcore", Tag::Byte(hardcore)),
                ("Difficulty", Tag::Byte(3)),
                (
                    "GameRules",
                    compound(vec![
                        ("keepInventory", string("false")),
                        ("pvp", string("false")),
                        ("spawnMonsters", string("true")),
                    ]),
                ),
            ]),
        )])
    }

    /// `level.dat` de 26.1: `difficulty_settings` y sin reglas.
    fn level_26_1(hardcore: i8) -> Tag {
        compound(vec![(
            "Data",
            compound(vec![
                (
                    "difficulty_settings",
                    compound(vec![
                        ("difficulty", string("normal")),
                        ("hardcore", Tag::Byte(hardcore)),
                        ("locked", Tag::Byte(0)),
                    ]),
                ),
                ("GameType", Tag::Int(0)),
            ]),
        )])
    }

    async fn write(path: &Path, root: &Tag) {
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(path, nbt::write_gzip(b"", root).unwrap())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn hardcore_in_old_and_new_worlds() {
        let dir = tempfile::tempdir().unwrap();
        let world = dir.path().join("world");
        assert_eq!(read_hardcore(&world).await, None);
        assert!(!set_hardcore(&world, true).await.unwrap());

        write(&world.join(LEVEL_DAT), &level_1_21_9(0)).await;
        assert_eq!(read_hardcore(&world).await, Some(false));
        assert!(set_hardcore(&world, true).await.unwrap());
        assert_eq!(read_hardcore(&world).await, Some(true));
        // El resto del archivo queda igual.
        let bytes = tokio::fs::read(world.join(LEVEL_DAT)).await.unwrap();
        assert_eq!(nbt::read_gzip(&bytes).unwrap().1, level_1_21_9(1));

        write(&world.join(LEVEL_DAT), &level_26_1(1)).await;
        assert_eq!(read_hardcore(&world).await, Some(true));
        set_hardcore(&world, false).await.unwrap();
        let bytes = tokio::fs::read(world.join(LEVEL_DAT)).await.unwrap();
        assert_eq!(nbt::read_gzip(&bytes).unwrap().1, level_26_1(0));
    }

    #[test]
    fn rejects_a_level_without_data() {
        let mut root = compound(vec![("Other", Tag::Int(1))]);
        assert!(set_hardcore_in(&mut root, true).is_err());
    }

    #[tokio::test]
    async fn rules_from_level_dat_and_game_rules_dat() {
        let dir = tempfile::tempdir().unwrap();
        let world = dir.path().join("world");
        let props = ["pvp", "spawn-monsters", "allow-nether"];
        assert!(read_rules(&world, &props).await.is_empty());

        write(&world.join(LEVEL_DAT), &level_1_21_9(0)).await;
        let rules = read_rules(&world, &props).await;
        assert_eq!(rules.get("pvp"), Some(&false));
        assert_eq!(rules.get("spawn-monsters"), Some(&true));
        assert_eq!(rules.get("allow-nether"), None);
        assert!(read_rules(&world, &[]).await.is_empty());

        // 26.1: archivo propio, con nombres de registro y bytes.
        write(
            &game_rules_file(&world),
            &compound(vec![(
                "data",
                compound(vec![
                    ("minecraft:pvp", Tag::Byte(1)),
                    (
                        "minecraft:allow_entering_nether_using_portals",
                        Tag::Byte(0),
                    ),
                    ("minecraft:spawn_radius", Tag::Int(10)),
                ]),
            )]),
        )
        .await;
        let rules = read_rules(&world, &props).await;
        assert_eq!(rules.get("pvp"), Some(&true));
        assert_eq!(rules.get("allow-nether"), Some(&false));
        assert_eq!(rules.get("spawn-monsters"), Some(&true));
        // Solo las pedidas.
        assert_eq!(read_rules(&world, &["pvp"]).await.len(), 1);
    }

    #[test]
    fn world_folder() {
        let mut props = BTreeMap::new();
        assert_eq!(
            world_dir(Path::new("s"), &props),
            Path::new("s").join("world")
        );
        props.insert("level-name".to_string(), "mundo".to_string());
        assert_eq!(
            world_dir(Path::new("s"), &props),
            Path::new("s").join("mundo")
        );
    }
}
