//! Utilidades de archivos compartidas: escritura atómica, copia de carpetas
//! con progreso, tamaño en disco y espacio libre.

use std::path::{Path, PathBuf};

use crate::{tr, Error, Result};

/// Escribe en `<path>.tmp` y renombra: un corte de luz a mitad nunca deja el
/// archivo vacío o a medias (importante para `micra.json` y los ajustes).
pub async fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = tmp_path(path);
    tokio::fs::write(&tmp, contents).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

pub fn write_atomic_sync(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = tmp_path(path);
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Lista recursiva de archivos (no carpetas) bajo `dir`, con su tamaño.
/// Los enlaces simbólicos se ignoran: nunca seguimos un enlace fuera de la
/// carpeta del servidor.
pub fn walk_files(dir: &Path) -> Result<Vec<(PathBuf, u64)>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() {
                out.push((entry.path(), entry.metadata()?.len()));
            }
        }
    }
    Ok(out)
}

/// Tamaño total de una carpeta (0 si no existe).
pub fn dir_size(dir: &Path) -> u64 {
    if dir.is_file() {
        return dir.metadata().map(|m| m.len()).unwrap_or(0);
    }
    walk_files(dir)
        .map(|files| files.iter().map(|(_, size)| size).sum())
        .unwrap_or(0)
}

/// Copia `from` dentro de `to` (que se crea), saltando las rutas relativas
/// para las que `skip` devuelve `true`. Llama a `progress(bytes_copiados, total)`.
pub fn copy_dir(
    from: &Path,
    to: &Path,
    skip: &dyn Fn(&Path) -> bool,
    progress: &dyn Fn(u64, u64),
) -> Result<()> {
    let files: Vec<(PathBuf, u64)> = walk_files(from)?
        .into_iter()
        .filter(|(path, _)| !skip(path.strip_prefix(from).unwrap_or(path)))
        .collect();
    let total: u64 = files.iter().map(|(_, size)| size).sum();
    let mut copied = 0u64;
    std::fs::create_dir_all(to)?;
    for (path, size) in files {
        let rel = path.strip_prefix(from).unwrap_or(&path);
        let dest = to.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&path, &dest)?;
        copied += size;
        progress(copied, total);
    }
    Ok(())
}

/// Espacio libre en el disco que contiene `path`, si se puede averiguar.
pub fn available_space(path: &Path) -> Option<u64> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    // La ruta puede no existir todavía: subimos hasta un antecesor que exista.
    let mut probe = path.to_path_buf();
    while !probe.exists() {
        probe = probe.parent()?.to_path_buf();
    }
    let probe = probe.canonicalize().ok()?;
    disks
        .list()
        .iter()
        .filter(|d| probe.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
}

/// Error si en el disco de `path` no caben `needed` bytes (con un margen del 5 %).
pub fn ensure_space(path: &Path, needed: u64) -> Result<()> {
    let Some(available) = available_space(path) else {
        return Ok(());
    };
    let needed_with_margin = needed + needed / 20;
    if available < needed_with_margin {
        let need = format_size(needed_with_margin);
        let free = format_size(available);
        return Err(Error::InvalidConfig(tr!(
            "no hay espacio suficiente en el disco: hacen falta {need} y quedan {free}",
            "not enough disk space: {need} needed and {free} free"
        )));
    }
    Ok(())
}

pub fn format_size(bytes: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else {
        format!("{:.0} MB", (b / MB).max(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_with_skip_and_size() {
        let src = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("world/region")).unwrap();
        std::fs::write(src.path().join("world/region/r.0.0.mca"), vec![1u8; 100]).unwrap();
        std::fs::create_dir_all(src.path().join("logs")).unwrap();
        std::fs::write(src.path().join("logs/latest.log"), b"hola").unwrap();
        assert_eq!(dir_size(src.path()), 104);

        let dst = tempfile::tempdir().unwrap();
        let target = dst.path().join("copia");
        let last = std::sync::Mutex::new((0, 0));
        copy_dir(
            src.path(),
            &target,
            &|rel| rel.starts_with("logs"),
            &|done, total| *last.lock().unwrap() = (done, total),
        )
        .unwrap();
        assert!(target.join("world/region/r.0.0.mca").is_file());
        assert!(!target.join("logs").exists());
        assert_eq!(*last.lock().unwrap(), (100, 100));
    }

    #[tokio::test]
    async fn atomic_write_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b.json");
        write_atomic(&path, b"1").await.unwrap();
        write_atomic(&path, b"2").await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"2");
        assert!(!dir.path().join("a/b.json.tmp").exists());
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(3 * 1024 * 1024 * 1024 / 2), "1.5 GB");
        assert_eq!(format_size(10), "1 MB");
    }
}
