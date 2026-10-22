//! Extracción segura de archivos comprimidos (ZIP y tar.gz).
//!
//! Todo lo que se descarga de internet se trata como no confiable: cada ruta
//! se valida para que no pueda escapar del directorio destino ("zip slip").

use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::{Error, Result};

/// Une `base` con una ruta relativa que viene de un archivo externo,
/// rechazando rutas absolutas o con `..`.
pub fn safe_join(base: &Path, relative: &str) -> Result<PathBuf> {
    let rel = Path::new(relative);
    let mut out = base.to_path_buf();
    for component in rel.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => return Err(Error::UnsafePath(relative.to_string())),
        }
    }
    if out == base {
        return Err(Error::UnsafePath(relative.to_string()));
    }
    Ok(out)
}

/// Extrae un ZIP entero. Con `strip_single_root`, si todo el contenido cuelga
/// de una única carpeta (habitual en server packs de CurseForge), se elimina
/// ese nivel para que los archivos queden directamente en `dest`.
pub fn extract_zip(archive: &Path, dest: &Path, strip_single_root: bool) -> Result<()> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    let root = if strip_single_root { single_root(&mut zip) } else { None };

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().replace('\\', "/");
        let rel = match &root {
            Some(r) => match name.strip_prefix(r.as_str()) {
                Some(rest) => rest.trim_start_matches('/').to_string(),
                None => continue,
            },
            None => name,
        };
        if rel.is_empty() {
            continue;
        }
        let out = safe_join(dest, &rel)?;
        write_entry(&mut entry, &out)?;
    }
    Ok(())
}

/// Extrae solo las entradas que empiezan por `prefix` (p. ej. `overrides/`
/// en un `.mrpack`), quitando el prefijo. Devuelve cuántos archivos escribió.
pub fn extract_zip_prefix(archive: &Path, prefix: &str, dest: &Path) -> Result<usize> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    let mut count = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().replace('\\', "/");
        let Some(rel) = name.strip_prefix(prefix) else { continue };
        if rel.is_empty() {
            continue;
        }
        let out = safe_join(dest, rel)?;
        if write_entry(&mut entry, &out)? {
            count += 1;
        }
    }
    Ok(count)
}

pub fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<()> {
    let gz = flate2::read::GzDecoder::new(File::open(archive)?);
    let mut tar = tar::Archive::new(gz);
    std::fs::create_dir_all(dest)?;
    for entry in tar.entries()? {
        let mut entry = entry?;
        // `unpack_in` ya rechaza rutas con `..`, pero validamos igualmente.
        let path = entry.path()?.to_string_lossy().into_owned();
        safe_join(dest, &path)?;
        entry.unpack_in(dest)?;
    }
    Ok(())
}

fn write_entry<R: io::Read>(entry: &mut zip::read::ZipFile<'_, R>, out: &Path) -> Result<bool> {
    if entry.is_dir() {
        std::fs::create_dir_all(out)?;
        return Ok(false);
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = File::create(out)?;
    io::copy(entry, &mut file)?;
    #[cfg(unix)]
    if let Some(mode) = entry.unix_mode() {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(out, std::fs::Permissions::from_mode(mode & 0o755))?;
    }
    Ok(true)
}

fn single_root(zip: &mut zip::ZipArchive<File>) -> Option<String> {
    let mut root: Option<String> = None;
    for name in zip.file_names() {
        let name = name.replace('\\', "/");
        let (first, rest) = name.split_once('/')?;
        if rest.is_empty() && !name.ends_with('/') {
            return None;
        }
        match &root {
            None => root = Some(first.to_string()),
            Some(r) if r == first => {}
            Some(_) => return None,
        }
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_join_rejects_traversal() {
        let base = Path::new("/srv/mc");
        assert!(safe_join(base, "mods/a.jar").is_ok());
        assert!(safe_join(base, "../etc/passwd").is_err());
        assert!(safe_join(base, "mods/../../x").is_err());
        assert!(safe_join(base, "/etc/passwd").is_err());
        assert!(safe_join(base, "").is_err());
    }
}
