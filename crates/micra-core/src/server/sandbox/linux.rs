//! Landlock mediante llamadas directas al kernel (sin dependencias).
//!
//! El conjunto de reglas se crea en el proceso de Micra antes del `fork`; en
//! el hijo, justo antes del `exec`, solo se llama a `prctl` y a
//! `landlock_restrict_self`, que son seguras tras un `fork` (no reservan
//! memoria). Las restricciones se heredan: también afectan a cualquier
//! programa que lance el servidor.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

const EXECUTE: u64 = 1 << 0;
const WRITE_FILE: u64 = 1 << 1;
const READ_FILE: u64 = 1 << 2;
const READ_DIR: u64 = 1 << 3;
const REFER: u64 = 1 << 13;
const TRUNCATE: u64 = 1 << 14;
const IOCTL_DEV: u64 = 1 << 15;
/// Derechos de la ABI 1 (bits 0 a 12).
const ABI1_ALL: u64 = (1 << 13) - 1;

const CREATE_RULESET_VERSION: u32 = 1;
const RULE_PATH_BENEATH: u32 = 1;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
}

#[repr(C, packed)]
struct PathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

/// Versión de la ABI de Landlock del kernel, o `None` si no está disponible
/// (kernel antiguo o desactivado en el arranque).
pub fn abi_version() -> Option<u32> {
    // SAFETY: con atributos nulos y el flag de versión, la llamada solo
    // devuelve un número y no toca memoria.
    let v = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<RulesetAttr>(),
            0usize,
            CREATE_RULESET_VERSION,
        )
    };
    (v > 0).then_some(v as u32)
}

/// Conjunto de reglas listo para aplicarse en el hijo.
pub struct Prepared {
    ruleset: OwnedFd,
    pub abi: u32,
}

/// Carpetas del sistema que el servidor puede leer (y ejecutar). Las que no
/// existen se ignoran.
const SYSTEM_READ_EXEC: &[&str] = &[
    "/usr",
    "/lib",
    "/lib32",
    "/lib64",
    "/bin",
    "/sbin",
    "/etc",
    "/opt",
    "/nix/store",
];
const SYSTEM_READ: &[&str] = &["/proc", "/sys", "/run"];

pub fn prepare(server_dir: &Path, java_home: &Path) -> io::Result<Option<Prepared>> {
    let Some(abi) = abi_version() else {
        return Ok(None);
    };
    let mut handled = ABI1_ALL;
    if abi >= 2 {
        handled |= REFER;
    }
    if abi >= 3 {
        handled |= TRUNCATE;
    }
    if abi >= 5 {
        handled |= IOCTL_DEV;
    }

    let attr = RulesetAttr {
        handled_access_fs: handled,
    };
    // SAFETY: `attr` vive durante la llamada y su tamaño es el que se indica.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &attr as *const RulesetAttr,
            std::mem::size_of::<RulesetAttr>(),
            0u32,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: el kernel acaba de devolver este descriptor y nadie más lo tiene.
    let ruleset = unsafe { OwnedFd::from_raw_fd(fd as i32) };

    let read_exec = EXECUTE | READ_FILE | READ_DIR;
    let read = READ_FILE | READ_DIR;
    let dev = (READ_FILE | WRITE_FILE | READ_DIR | IOCTL_DEV) & handled;

    // La carpeta del servidor y /tmp: todo (crear, borrar, renombrar...).
    add_rule(&ruleset, server_dir, handled)?;
    add_rule_if_exists(&ruleset, Path::new("/tmp"), handled)?;
    add_rule(&ruleset, java_home, read_exec)?;
    for dir in SYSTEM_READ_EXEC {
        add_rule_if_exists(&ruleset, Path::new(dir), read_exec)?;
    }
    for dir in SYSTEM_READ {
        add_rule_if_exists(&ruleset, Path::new(dir), read)?;
    }
    // /dev/null, /dev/urandom...: leer y escribir, sin crear nada.
    add_rule_if_exists(&ruleset, Path::new("/dev"), dev)?;

    Ok(Some(Prepared { ruleset, abi }))
}

fn add_rule_if_exists(ruleset: &OwnedFd, path: &Path, access: u64) -> io::Result<()> {
    if path.is_dir() {
        add_rule(ruleset, path, access)?;
    }
    Ok(())
}

fn add_rule(ruleset: &OwnedFd, path: &Path, access: u64) -> io::Result<()> {
    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "ruta con un byte nulo"))?;
    // SAFETY: `c_path` es una cadena C válida durante la llamada.
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: descriptor recién abierto, propiedad exclusiva.
    let dir = unsafe { OwnedFd::from_raw_fd(fd) };
    let attr = PathBeneathAttr {
        allowed_access: access,
        parent_fd: dir.as_raw_fd(),
    };
    // SAFETY: `attr` vive durante la llamada; ambos descriptores son válidos.
    let res = unsafe {
        libc::syscall(
            libc::SYS_landlock_add_rule,
            ruleset.as_raw_fd(),
            RULE_PATH_BENEATH,
            &attr as *const PathBeneathAttr,
            0u32,
        )
    };
    if res < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Registra la restricción para que se aplique en el hijo antes del `exec`.
/// `prepared` debe seguir vivo hasta que `spawn` termine.
pub fn apply_pre_exec(cmd: &mut tokio::process::Command, prepared: &Prepared) {
    let fd = prepared.ruleset.as_raw_fd();
    // SAFETY: el cierre se ejecuta en el hijo tras `fork` y solo hace llamadas
    // al sistema seguras en ese contexto, sin reservar memoria.
    unsafe {
        cmd.pre_exec(move || {
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::syscall(libc::SYS_landlock_restrict_self, fd, 0u32) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lanza `sh` aislado y comprueba que puede escribir en su carpeta pero
    /// no leer ni escribir fuera de ella.
    #[tokio::test]
    async fn blocks_files_outside_server_dir() {
        if abi_version().is_none() {
            eprintln!("Landlock no disponible: se omite la prueba");
            return;
        }
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let server = root.path().join("server");
        let secret_dir = root.path().join("personal");
        std::fs::create_dir_all(&server).unwrap();
        std::fs::create_dir_all(&secret_dir).unwrap();
        std::fs::write(secret_dir.join("passwords.txt"), "secreto").unwrap();
        // "Java" falso: usamos /bin como JAVA_HOME para poder ejecutar sh.
        let prepared = prepare(&server, Path::new("/bin")).unwrap().unwrap();

        let script = format!(
            "echo ok > {s}/world.txt && cat {s}/world.txt; \
             cat {p}/passwords.txt && echo LEIDO; \
             echo x > {p}/autostart.sh && echo ESCRITO; true",
            s = server.display(),
            p = secret_dir.display()
        );
        let mut cmd = tokio::process::Command::new("/bin/sh");
        cmd.arg("-c").arg(script).current_dir(&server);
        apply_pre_exec(&mut cmd, &prepared);
        let out = cmd.output().await.unwrap();
        drop(prepared);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("ok"),
            "debería escribir en su carpeta: {stdout}"
        );
        assert!(!stdout.contains("LEIDO"), "no debería leer fuera: {stdout}");
        assert!(
            !stdout.contains("ESCRITO"),
            "no debería escribir fuera: {stdout}"
        );
        assert!(!secret_dir.join("autostart.sh").exists());
    }
}
