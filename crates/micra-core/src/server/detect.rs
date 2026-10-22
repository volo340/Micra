//! Detecta cómo arrancar un servidor ya presente en disco (server packs de
//! CurseForge, o después de ejecutar el instalador de Forge/NeoForge).

use std::path::{Path, PathBuf};

use crate::server::config::LaunchTarget;
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
    let jar = pick(&|n| n.starts_with("fabric-server-launch"))
        .or_else(|| pick(&|n| n.starts_with("quilt-server-launch")))
        .or_else(|| pick(&|n| n.starts_with("forge-") || n.starts_with("neoforge-")))
        .or_else(|| pick(&|n| n == "server.jar"))
        .or_else(|| (jars.len() == 1).then(|| jars[0].clone()));

    Ok(jar.map(|j| LaunchTarget::Jar {
        path: PathBuf::from(j),
    }))
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
    candidates.sort();
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
    }
}
