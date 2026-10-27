//! Utilidades de archivos compartidas: escritura atómica, copia de carpetas
//! con progreso, tamaño en disco y espacio libre.

use std::path::{Path, PathBuf};

use crate::{tr, Error, Result};

/// Escribe en `<path>.tmp` y renombra: un corte de luz a mitad nunca deja el
/// archivo vacío o a medias (importante para `micra.json` y los ajustes).
///
/// El temporal se crea siempre de cero (si había algo con su nombre, se
/// borra antes) y el `rename` sustituye un enlace en vez de seguirlo: Micra
/// nunca escribe a través de un enlace que haya dejado un mod en la carpeta
/// del servidor.
pub async fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let path = path.to_owned();
    let contents = contents.to_vec();
    tokio::task::spawn_blocking(move || write_atomic_sync(&path, &contents))
        .await
        .expect("la escritura no debería entrar en pánico")
}

pub fn write_atomic_sync(path: &Path, contents: &[u8]) -> Result<()> {
    write_new(path, contents, 0o666)
}

/// Como `write_atomic_sync`, pero solo el usuario puede leer el archivo, ya
/// desde que se crea (no después, cuando otro usuario del equipo podría
/// haberlo abierto).
pub fn write_private_sync(path: &Path, contents: &[u8]) -> Result<()> {
    write_new(path, contents, 0o600)
}

fn write_new(path: &Path, contents: &[u8], mode: u32) -> Result<()> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = tmp_path(path);
    remove_entry(&tmp)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, mode);
    #[cfg(not(unix))]
    let _ = mode;
    let written = options
        .open(&tmp)
        .and_then(|mut file| file.write_all(contents));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Borra lo que haya en `path` sin seguir enlaces: un archivo, un enlace (no
/// lo que apunta) o una carpeta entera.
pub fn remove_entry(path: &Path) -> std::io::Result<()> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if meta.is_dir() {
        return std::fs::remove_dir_all(path);
    }
    match std::fs::remove_file(path) {
        // En Windows, los enlaces a carpetas (y las uniones) se borran como carpetas.
        Err(e) if meta.file_type().is_symlink() => std::fs::remove_dir(path).map_err(|_| e),
        other => other,
    }
}

/// Error si alguna carpeta entre `base` y `path` es un enlace (simbólico o,
/// en Windows, una unión). Micra trabaja fuera del aislamiento: no debe
/// escribir, mover ni borrar nada siguiendo un enlace que un mod haya creado
/// dentro de la carpeta del servidor para apuntar a la carpeta personal.
/// El propio `path` no cuenta: borrar o renombrar un enlace no toca aquello
/// a lo que apunta.
pub fn check_no_links(base: &Path, path: &Path) -> Result<()> {
    let unsafe_path = || Error::UnsafePath(path.display().to_string());
    let rel = path.strip_prefix(base).map_err(|_| unsafe_path())?;
    let mut current = base.to_path_buf();
    let mut components = rel.components().peekable();
    while let Some(component) = components.next() {
        if components.peek().is_none() {
            break;
        }
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(unsafe_path()),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Deja `dir` como una carpeta de verdad: si alguien la cambió por un
/// enlace, se quita el enlace (no lo que apunta) y se crea la carpeta.
/// Para las carpetas internas de Micra (`.micra`), que nadie más usa.
pub fn ensure_real_dir(dir: &Path) -> Result<()> {
    if std::fs::symlink_metadata(dir).is_ok_and(|m| !m.is_dir()) {
        remove_entry(dir)?;
    }
    std::fs::create_dir_all(dir)?;
    Ok(())
}

/// Carpeta interna de Micra (`rel`, dentro de `.micra`) de un servidor,
/// creada como carpeta de verdad en todo el camino: si un mod cambió alguna
/// por un enlace, se quita el enlace.
pub fn internal_dir(server_dir: &Path, rel: &str) -> Result<PathBuf> {
    let mut current = server_dir.to_path_buf();
    for part in Path::new(rel).components() {
        current.push(part);
        ensure_real_dir(&current)?;
    }
    Ok(current)
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

    /// Un mod deja enlaces a la carpeta personal con los nombres que Micra
    /// va a escribir: ninguno se sigue.
    #[cfg(unix)]
    #[tokio::test]
    async fn writes_never_follow_links() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let home = tempfile::tempdir().unwrap();
        let server = tempfile::tempdir().unwrap();
        let s = server.path();
        let bashrc = home.path().join(".bashrc");
        std::fs::write(&bashrc, "original").unwrap();

        // El temporal es un enlace: se borra y se crea de cero.
        symlink(&bashrc, s.join("server.properties.tmp")).unwrap();
        write_atomic(&s.join("server.properties"), b"motd=x\n")
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&bashrc).unwrap(), "original");
        assert_eq!(
            std::fs::read_to_string(s.join("server.properties")).unwrap(),
            "motd=x\n"
        );

        // El propio archivo es un enlace: se sustituye el enlace.
        symlink(&bashrc, s.join("ops.json")).unwrap();
        write_atomic_sync(&s.join("ops.json"), b"[]").unwrap();
        assert_eq!(std::fs::read_to_string(&bashrc).unwrap(), "original");
        assert!(!std::fs::symlink_metadata(s.join("ops.json"))
            .unwrap()
            .file_type()
            .is_symlink());

        // Una carpeta intermedia es un enlace.
        symlink(home.path(), s.join("mods")).unwrap();
        assert!(check_no_links(s, &s.join("mods/x.jar")).is_err());
        assert!(check_no_links(s, &s.join("config/x.toml")).is_ok());
        assert!(check_no_links(s, &s.join("mods")).is_ok());
        assert!(check_no_links(s, Path::new("/etc/passwd")).is_err());

        // `.micra` cambiada por un enlace vuelve a ser una carpeta.
        symlink(home.path(), s.join(".micra")).unwrap();
        ensure_real_dir(&s.join(".micra")).unwrap();
        assert!(s.join(".micra").is_dir());
        assert!(home.path().exists() && bashrc.exists());

        write_private_sync(&s.join(".micra/secret"), b"x").unwrap();
        let mode = std::fs::metadata(s.join(".micra/secret"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(3 * 1024 * 1024 * 1024 / 2), "1.5 GB");
        assert_eq!(format_size(10), "1 MB");
    }
}
