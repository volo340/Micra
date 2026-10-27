//! Extracción segura de archivos comprimidos (ZIP y tar.gz).
//!
//! Todo lo que se descarga de internet se trata como no confiable: cada ruta
//! se valida para que no pueda escapar del directorio destino ("zip slip"),
//! nunca se escribe a través de un enlace que haya en el destino (un mod
//! pudo dejarlo en la carpeta del servidor apuntando a la carpeta personal)
//! y lo que ocupa descomprimido tiene que caber en el disco (bombas ZIP).

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

/// `safe_join` para escribir, mover o borrar dentro de la carpeta de un
/// servidor: además, ninguna carpeta del camino puede ser un enlace.
pub fn safe_join_no_links(base: &Path, relative: &str) -> Result<PathBuf> {
    let path = safe_join(base, relative)?;
    crate::fsutil::check_no_links(base, &path)?;
    Ok(path)
}

/// Extrae un ZIP entero. Con `strip_single_root`, si todo el contenido cuelga
/// de una única carpeta (habitual en server packs de CurseForge), se elimina
/// ese nivel para que los archivos queden directamente en `dest`. Devuelve
/// las rutas relativas (con `/`) de los archivos escritos.
pub fn extract_zip(archive: &Path, dest: &Path, strip_single_root: bool) -> Result<Vec<String>> {
    extract_zip_filtered(archive, dest, strip_single_root, &|_| false)
}

/// Como `extract_zip`, sin escribir las rutas para las que `skip` es `true`.
pub fn extract_zip_filtered(
    archive: &Path,
    dest: &Path,
    strip_single_root: bool,
    skip: &dyn Fn(&str) -> bool,
) -> Result<Vec<String>> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    check_size(&mut zip, dest)?;
    let root = if strip_single_root {
        single_root(&mut zip)
    } else {
        None
    };

    let mut written = Vec::new();
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
        if rel.is_empty() || skip(rel.trim_end_matches('/')) {
            continue;
        }
        let out = safe_join(dest, &rel)?;
        if write_entry(&mut entry, dest, &out)? {
            written.push(rel.trim_end_matches('/').to_string());
        }
    }
    Ok(written)
}

/// Extrae solo las entradas que empiezan por `prefix` (p. ej. `overrides/`
/// en un `.mrpack`), quitando el prefijo. Devuelve las rutas relativas de
/// los archivos escritos.
pub fn extract_zip_prefix(archive: &Path, prefix: &str, dest: &Path) -> Result<Vec<String>> {
    extract_zip_prefix_filtered(archive, prefix, dest, &|_| false)
}

/// Como `extract_zip_prefix`, sin escribir las rutas para las que `skip` es `true`.
pub fn extract_zip_prefix_filtered(
    archive: &Path,
    prefix: &str,
    dest: &Path,
    skip: &dyn Fn(&str) -> bool,
) -> Result<Vec<String>> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    check_size(&mut zip, dest)?;
    let mut written = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().replace('\\', "/");
        let Some(rel) = name.strip_prefix(prefix) else {
            continue;
        };
        if rel.is_empty() || skip(rel.trim_end_matches('/')) {
            continue;
        }
        let out = safe_join(dest, rel)?;
        if write_entry(&mut entry, dest, &out)? {
            written.push(rel.to_string());
        }
    }
    Ok(written)
}

/// Lista las rutas (sin extraer) de las entradas bajo `prefix`, ya sin él.
pub fn list_zip_prefix(archive: &Path, prefix: &str) -> Result<Vec<String>> {
    let zip = zip::ZipArchive::new(File::open(archive)?)?;
    Ok(zip
        .file_names()
        .map(|n| n.replace('\\', "/"))
        .filter(|n| !n.ends_with('/'))
        .filter_map(|n| n.strip_prefix(prefix).map(str::to_string))
        .filter(|n| !n.is_empty())
        .collect())
}

/// Rutas de los archivos de un ZIP (sin extraer), quitando la carpeta raíz
/// única si la hay, igual que `extract_zip` con `strip_single_root`.
pub fn list_zip(archive: &Path, strip_single_root: bool) -> Result<Vec<String>> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    let root = if strip_single_root {
        single_root(&mut zip)
    } else {
        None
    };
    Ok(zip
        .file_names()
        .map(|n| n.replace('\\', "/"))
        .filter(|n| !n.ends_with('/'))
        .filter_map(|n| match &root {
            Some(r) => n
                .strip_prefix(r.as_str())
                .map(|rest| rest.trim_start_matches('/').to_string()),
            None => Some(n),
        })
        .filter(|n| !n.is_empty())
        .collect())
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

/// Más entradas que esto no es un pack, es un ataque (los server packs más
/// grandes tienen unas decenas de miles).
const MAX_ENTRIES: usize = 500_000;

/// Lo que ocupa el ZIP descomprimido (según su índice) tiene que caber en el
/// disco; `write_entry` impide luego que una entrada ocupe más de lo que dice.
fn check_size<R: io::Read + io::Seek>(zip: &mut zip::ZipArchive<R>, dest: &Path) -> Result<()> {
    if zip.len() > MAX_ENTRIES {
        return Err(too_big());
    }
    let mut total = 0u64;
    for i in 0..zip.len() {
        total = total.saturating_add(zip.by_index_raw(i)?.size());
    }
    std::fs::create_dir_all(dest)?;
    crate::fsutil::ensure_space(dest, total)
}

fn too_big() -> Error {
    Error::InvalidConfig(crate::tr!(
        "el archivo comprimido dice ocupar menos de lo que ocupa: puede ser una bomba ZIP y no se extrae",
        "the archive claims to be smaller than it is: it may be a ZIP bomb, so it is not extracted"
    ))
}

fn write_entry<R: io::Read>(
    entry: &mut zip::read::ZipFile<'_, R>,
    dest: &Path,
    out: &Path,
) -> Result<bool> {
    crate::fsutil::check_no_links(dest, out)?;
    if entry.is_dir() {
        std::fs::create_dir_all(out)?;
        return Ok(false);
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Si ya hay algo (también un enlace), se sustituye sin seguirlo.
    crate::fsutil::remove_entry(out)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out)?;
    let declared = entry.size();
    let copied = io::copy(&mut io::Read::take(&mut *entry, declared + 1), &mut file)?;
    if copied > declared {
        drop(file);
        let _ = std::fs::remove_file(out);
        return Err(too_big());
    }
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

    /// Un mod deja `config` apuntando a otra carpeta; al actualizar el pack,
    /// Micra no escribe a través del enlace.
    #[cfg(unix)]
    #[test]
    fn never_writes_through_links() {
        use std::io::Write;
        let root = tempfile::tempdir().unwrap();
        let server = root.path().join("server");
        let outside = root.path().join("autostart");
        std::fs::create_dir_all(&server).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let zip_path = root.path().join("pack.zip");
        let mut zip = zip::ZipWriter::new(File::create(&zip_path).unwrap());
        for (name, body) in [("mods/a.jar", "jar"), ("config/evil.desktop", "x")] {
            zip.start_file::<_, ()>(name, Default::default()).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();

        std::os::unix::fs::symlink(&outside, server.join("config")).unwrap();
        assert!(extract_zip(&zip_path, &server, false).is_err());
        assert!(!outside.join("evil.desktop").exists());

        // Un enlace en el lugar de un archivo se sustituye, sin tocar su destino.
        std::fs::remove_file(server.join("config")).unwrap();
        let target = outside.join("bashrc");
        std::fs::write(&target, "original").unwrap();
        std::fs::remove_file(server.join("mods/a.jar")).unwrap();
        std::os::unix::fs::symlink(&target, server.join("mods/a.jar")).unwrap();
        extract_zip(&zip_path, &server, false).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
        assert_eq!(
            std::fs::read_to_string(server.join("mods/a.jar")).unwrap(),
            "jar"
        );
    }
}
