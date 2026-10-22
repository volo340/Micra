//! Detecta cómo arrancar un servidor ya presente en disco (server packs de
//! CurseForge, o después de ejecutar el instalador de Forge/NeoForge).

use std::path::{Path, PathBuf};

use crate::server::config::{LaunchTarget, Loader};
use crate::Result;

const ARGS_FILE: &str = if cfg!(windows) {
    "win_args.txt"
} else {
    "unix_args.txt"
};

pub fn detect_launch_target(server_dir: &Path) -> Result<Option<LaunchTarget>> {
    // 1) Forge/NeoForge modernos: libraries/net/<org>/<loader>/<versión>/unix_args.txt
    for loader_dir in [
        "net/neoforged/neoforge",
        "net/neoforged/forge",
        "net/minecraftforge/forge",
    ] {
        let base = server_dir.join("libraries").join(loader_dir);
        if let Some(args) = newest_args_file(&base)? {
            let rel = args.strip_prefix(server_dir).unwrap_or(&args).to_path_buf();
            return Ok(Some(LaunchTarget::ArgsFile { path: rel }));
        }
    }

    // 2) Jars conocidos en la raíz, por orden de preferencia.
    let mut jars: Vec<String> = std::fs::read_dir(server_dir)?
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".jar") && !n.contains("installer"))
        .collect();
    jars.sort();
    let pick = |pred: &dyn Fn(&str) -> bool| jars.iter().find(|n| pred(n)).cloned();
    let jar = pick(&|n| n.starts_with("fabric-server"))
        .or_else(|| pick(&|n| n.starts_with("quilt-server-launch")))
        .or_else(|| pick(&|n| n.starts_with("forge-") || n.starts_with("neoforge-")))
        .or_else(|| pick(&|n| n == "server.jar"))
        .or_else(|| (jars.len() == 1).then(|| jars[0].clone()));

    Ok(jar.map(|j| LaunchTarget::Jar {
        path: PathBuf::from(j),
    }))
}

/// Loader instalado en una carpeta de servidor, a partir de las librerías
/// que deja cada instalador. `Vanilla` si no se reconoce ninguno.
pub fn detect_loader(server_dir: &Path) -> Loader {
    let libs = server_dir.join("libraries");
    if let Some(v) = newest_subdir(&libs.join("net/neoforged/neoforge")) {
        return Loader::NeoForge { version: v };
    }
    // NeoForge para 1.20.1 era un fork de Forge con su mismo formato.
    if let Some(v) = newest_subdir(&libs.join("net/neoforged/forge")) {
        let version = v.split_once('-').map_or(v.clone(), |(_, f)| f.to_string());
        return Loader::NeoForge { version };
    }
    if let Some(v) = newest_subdir(&libs.join("net/minecraftforge/forge")) {
        let version = v.split_once('-').map_or(v.clone(), |(_, f)| f.to_string());
        return Loader::Forge { version };
    }
    if let Some(v) = newest_subdir(&libs.join("org/quiltmc/quilt-loader")) {
        return Loader::Quilt { loader_version: v };
    }
    if let Some(v) = newest_subdir(&libs.join("net/fabricmc/fabric-loader")) {
        return Loader::Fabric { loader_version: v };
    }
    if let Some(props) = fabric_launcher_properties(server_dir) {
        if let Some(v) = props.get("fabric-loader-version") {
            return Loader::Fabric {
                loader_version: v.clone(),
            };
        }
    }
    Loader::Vanilla
}

/// `install.properties` del lanzador de Fabric (`fabric-server-launch.jar`
/// o `fabric-server-mc.<versión>-loader.<versión>-launcher.<versión>.jar`).
fn fabric_launcher_properties(
    server_dir: &Path,
) -> Option<std::collections::BTreeMap<String, String>> {
    let mut jars: Vec<PathBuf> = std::fs::read_dir(server_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("fabric-server") && n.ends_with(".jar"))
        })
        .collect();
    jars.sort();
    jars.iter()
        .find_map(|jar| jar_properties(jar, "install.properties"))
}

/// Loader y versión de Minecraft que declara un server pack sin instalar:
/// el `manifest.json` de CurseForge o el `variables.txt` de los packs que
/// traen su propio script de arranque.
pub fn pack_declared_loader(server_dir: &Path) -> Option<(Option<String>, Loader)> {
    #[derive(serde::Deserialize)]
    struct Manifest {
        minecraft: ManifestMc,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ManifestMc {
        version: Option<String>,
        #[serde(default)]
        mod_loaders: Vec<ManifestLoader>,
    }
    #[derive(serde::Deserialize)]
    struct ManifestLoader {
        id: String,
        #[serde(default)]
        primary: bool,
    }

    if let Ok(raw) = std::fs::read(server_dir.join("manifest.json")) {
        if let Ok(m) = serde_json::from_slice::<Manifest>(&raw) {
            let entry = m
                .minecraft
                .mod_loaders
                .iter()
                .find(|l| l.primary)
                .or(m.minecraft.mod_loaders.first());
            if let Some(loader) = entry.and_then(|l| loader_from_id(&l.id)) {
                return Some((m.minecraft.version, loader));
            }
        }
    }
    let vars = std::fs::read_to_string(server_dir.join("variables.txt")).ok()?;
    let vars = crate::server::properties::parse(&vars);
    let get = |k: &str| {
        vars.get(k)
            .map(|v| v.trim().trim_matches('"').to_string())
            .filter(|v| !v.is_empty())
    };
    let name = get("MODLOADER")?.to_ascii_lowercase();
    let version = get("MODLOADER_VERSION")?;
    let loader = loader_from_id(&format!("{name}-{version}"))?;
    Some((get("MINECRAFT_VERSION"), loader))
}

/// `forge-47.2.0`, `neoforge-21.1.77`, `fabric-0.16.9`, `quilt-0.26.0`.
fn loader_from_id(id: &str) -> Option<Loader> {
    let (name, version) = id.split_once('-')?;
    let version = version.to_string();
    Some(match name.to_ascii_lowercase().as_str() {
        "forge" => Loader::Forge { version },
        "neoforge" => Loader::NeoForge { version },
        "fabric" => Loader::Fabric {
            loader_version: version,
        },
        "quilt" => Loader::Quilt {
            loader_version: version,
        },
        _ => return None,
    })
}

/// Versión de Minecraft de una carpeta de servidor, si se puede averiguar.
pub fn detect_mc_version(server_dir: &Path) -> Option<String> {
    let libs = server_dir.join("libraries");
    // Forge (y NeoForge 1.20.1): carpeta `<mc>-<versión del loader>`.
    for group in ["net/minecraftforge/forge", "net/neoforged/forge"] {
        if let Some(v) = newest_subdir(&libs.join(group)) {
            if let Some((mc, _)) = v.split_once('-') {
                return Some(mc.to_string());
            }
        }
    }
    if let Some(v) = newest_subdir(&libs.join("net/neoforged/neoforge")) {
        if let Some(mc) = neoforge_mc_version(&v) {
            return Some(mc);
        }
    }
    // Fabric y Quilt: la versión de "intermediary" es la del juego.
    for group in ["net/fabricmc/intermediary", "org/quiltmc/hashed"] {
        if let Some(v) = newest_subdir(&libs.join(group)) {
            return Some(v);
        }
    }
    if let Some(props) = fabric_launcher_properties(server_dir) {
        if let Some(v) = props.get("game-version") {
            return Some(v.clone());
        }
    }
    // Vanilla 1.14+: `version.json` dentro del jar del servidor.
    for jar in ["server.jar", "minecraft_server.jar"] {
        if let Some(id) = jar_version_json(&server_dir.join(jar)) {
            return Some(id);
        }
    }
    // Servidores 1.18+ extraen el jar real en `versions/<mc>/`.
    newest_subdir(&server_dir.join("versions"))
}

/// NeoForge numera sus versiones a partir de la del juego:
/// `21.1.77` → `1.21.1`, `21.0.3` → `1.21`, `20.4.237` → `1.20.4`.
/// Desde las versiones por año (26.1...) el prefijo es la propia versión.
pub fn neoforge_mc_version(version: &str) -> Option<String> {
    let mut parts = version.split(['.', '-']);
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    if major >= 26 {
        return Some(format!("{major}.{minor}"));
    }
    Some(if minor == 0 {
        format!("1.{major}")
    } else {
        format!("1.{major}.{minor}")
    })
}

/// La subcarpeta con el nombre "más alto" (las versiones se ordenan bien
/// con comparación por partes numéricas).
fn newest_subdir(dir: &Path) -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort_by(|a, b| crate::server::version::compare(a, b));
    names.pop()
}

fn jar_properties(jar: &Path, entry: &str) -> Option<std::collections::BTreeMap<String, String>> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(jar).ok()?).ok()?;
    let mut file = zip.by_name(entry).ok()?;
    let mut content = String::new();
    std::io::Read::read_to_string(&mut file, &mut content).ok()?;
    Some(crate::server::properties::parse(&content))
}

fn jar_version_json(jar: &Path) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct VersionJson {
        id: String,
    }
    let mut zip = zip::ZipArchive::new(std::fs::File::open(jar).ok()?).ok()?;
    let file = zip.by_name("version.json").ok()?;
    let parsed: VersionJson = serde_json::from_reader(file).ok()?;
    Some(parsed.id)
}

/// Instaladores de Forge/NeoForge que algunos server packs traen sin
/// ejecutar (`forge-1.20.1-47.2.0-installer.jar`).
pub fn bundled_installer(server_dir: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(server_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                let n = n.to_ascii_lowercase();
                n.ends_with("-installer.jar")
                    && (n.starts_with("forge-") || n.starts_with("neoforge-"))
            })
        })
        .collect();
    found.sort();
    found.pop()
}

fn newest_args_file(base: &Path) -> Result<Option<PathBuf>> {
    let Ok(entries) = std::fs::read_dir(base) else {
        return Ok(None);
    };
    let mut candidates: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path().join(ARGS_FILE))
        .filter(|p| p.is_file())
        .collect();
    candidates.sort_by(|a, b| {
        let name = |p: &PathBuf| {
            p.parent()
                .and_then(|d| d.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        crate::server::version::compare(&name(a), &name(b))
    });
    Ok(candidates.pop())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_forge_args_and_jars() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("server.jar"), b"").unwrap();
        assert_eq!(
            detect_launch_target(dir.path()).unwrap(),
            Some(LaunchTarget::Jar {
                path: "server.jar".into()
            })
        );

        let v = dir.path().join("libraries/net/neoforged/neoforge/21.1.77");
        std::fs::create_dir_all(&v).unwrap();
        std::fs::write(v.join(ARGS_FILE), b"").unwrap();
        match detect_launch_target(dir.path()).unwrap() {
            Some(LaunchTarget::ArgsFile { path }) => assert!(path.ends_with(ARGS_FILE)),
            other => panic!("inesperado: {other:?}"),
        }
        assert_eq!(
            detect_loader(dir.path()),
            Loader::NeoForge {
                version: "21.1.77".into()
            }
        );
        assert_eq!(detect_mc_version(dir.path()).as_deref(), Some("1.21.1"));
    }

    #[test]
    fn detects_forge_and_fabric() {
        let dir = tempfile::tempdir().unwrap();
        for v in ["1.20.1-47.1.3", "1.20.1-47.2.20"] {
            std::fs::create_dir_all(
                dir.path()
                    .join("libraries/net/minecraftforge/forge")
                    .join(v),
            )
            .unwrap();
        }
        assert_eq!(
            detect_loader(dir.path()),
            Loader::Forge {
                version: "47.2.20".into()
            }
        );
        assert_eq!(detect_mc_version(dir.path()).as_deref(), Some("1.20.1"));

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(
            dir.path()
                .join("libraries/net/fabricmc/fabric-loader/0.16.9"),
        )
        .unwrap();
        std::fs::create_dir_all(
            dir.path()
                .join("libraries/net/fabricmc/intermediary/1.21.1"),
        )
        .unwrap();
        assert_eq!(
            detect_loader(dir.path()),
            Loader::Fabric {
                loader_version: "0.16.9".into()
            }
        );
        assert_eq!(detect_mc_version(dir.path()).as_deref(), Some("1.21.1"));
    }

    #[test]
    fn vanilla_version_from_jar() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let file = std::fs::File::create(dir.path().join("server.jar")).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("version.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(br#"{"id":"1.21.4","name":"1.21.4"}"#)
            .unwrap();
        zip.finish().unwrap();
        assert_eq!(detect_loader(dir.path()), Loader::Vanilla);
        assert_eq!(detect_mc_version(dir.path()).as_deref(), Some("1.21.4"));
    }

    #[test]
    fn declared_loaders() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(pack_declared_loader(dir.path()), None);
        std::fs::write(
            dir.path().join("variables.txt"),
            "MINECRAFT_VERSION=1.21.1\nMODLOADER=NeoForge\nMODLOADER_VERSION=21.1.77\n",
        )
        .unwrap();
        assert_eq!(
            pack_declared_loader(dir.path()),
            Some((
                Some("1.21.1".into()),
                Loader::NeoForge {
                    version: "21.1.77".into()
                }
            ))
        );
        std::fs::write(
            dir.path().join("manifest.json"),
            r#"{"minecraft":{"version":"1.20.1","modLoaders":[{"id":"forge-47.2.0","primary":true}]}}"#,
        )
        .unwrap();
        assert_eq!(
            pack_declared_loader(dir.path()),
            Some((
                Some("1.20.1".into()),
                Loader::Forge {
                    version: "47.2.0".into()
                }
            ))
        );
    }

    #[test]
    fn neoforge_versions() {
        assert_eq!(neoforge_mc_version("21.1.77").as_deref(), Some("1.21.1"));
        assert_eq!(neoforge_mc_version("21.0.3-beta").as_deref(), Some("1.21"));
        assert_eq!(neoforge_mc_version("20.4.237").as_deref(), Some("1.20.4"));
        assert_eq!(neoforge_mc_version("26.1.0.12").as_deref(), Some("26.1"));
    }
}
