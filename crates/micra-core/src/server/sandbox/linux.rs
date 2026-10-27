//! Aislamiento en Linux mediante llamadas directas al kernel (sin
//! dependencias):
//!
//! - **Landlock** limita los archivos: el servidor solo escribe en su carpeta
//!   y solo lee su Java y los archivos del sistema que necesita.
//! - **seccomp** cierra lo que Landlock no cubre: conectarse a sockets Unix
//!   (D-Bus, el escritorio, el agente SSH, los de otros programas), crear
//!   espacios de nombres, io_uring, el llavero del kernel, espiar otros
//!   procesos o escribir en la terminal de la persona.
//!
//! Todo se prepara en el proceso de Micra antes del `fork`; en el hijo, justo
//! antes del `exec`, solo se hacen llamadas al sistema seguras tras un `fork`
//! (no reservan memoria). Las restricciones se heredan: también afectan a
//! cualquier programa que lance el servidor.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
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
/// ABI 6: ni señales ni sockets Unix abstractos hacia fuera del aislamiento.
const SCOPE_ABSTRACT_UNIX_SOCKET: u64 = 1 << 0;
const SCOPE_SIGNAL: u64 = 1 << 1;

const CREATE_RULESET_VERSION: u32 = 1;
const RULE_PATH_BENEATH: u32 = 1;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
    handled_access_net: u64,
    scoped: u64,
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

/// Lo que se aplica en el hijo.
pub struct Prepared {
    /// Reglas de Landlock, si el kernel lo tiene.
    ruleset: Option<OwnedFd>,
    pub abi: Option<u32>,
    /// Programa de seccomp, si el kernel y la arquitectura lo permiten.
    filter: Option<Vec<libc::sock_filter>>,
}

impl Prepared {
    pub fn seccomp(&self) -> bool {
        self.filter.is_some()
    }
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
/// Solo lectura: lo que consulta la JVM sobre el equipo (CPUs, memoria,
/// cgroups).
const SYSTEM_READ: &[&str] = &["/proc", "/sys"];
/// Dispositivos que puede usar. Nada más de /dev: ni /dev/shm (la memoria
/// compartida de otros programas) ni las terminales.
const DEVICES: &[&str] = &[
    "/dev/null",
    "/dev/zero",
    "/dev/full",
    "/dev/random",
    "/dev/urandom",
];
/// Archivos de configuración que suelen ser enlaces a /run (resolv.conf con
/// systemd-resolved o NetworkManager): se permite leer su destino, no todo
/// /run, donde están las unidades de red montadas, los USB y los sockets de
/// la sesión.
const SYSTEM_FILES: &[&str] = &[
    "/etc/resolv.conf",
    "/etc/hosts",
    "/etc/nsswitch.conf",
    "/etc/host.conf",
    "/etc/gai.conf",
    "/etc/localtime",
];

/// Prepara el aislamiento. Devuelve `None` si el kernel no tiene ni Landlock
/// ni seccomp.
pub fn prepare(server_dir: &Path, java_home: &Path) -> io::Result<Option<Prepared>> {
    let abi = abi_version();
    let ruleset = match abi {
        Some(abi) => Some(landlock_rules(abi, server_dir, java_home)?),
        None => None,
    };
    let filter = seccomp_available().then(seccomp_filter).flatten();
    if ruleset.is_none() && filter.is_none() {
        return Ok(None);
    }
    Ok(Some(Prepared {
        ruleset,
        abi,
        filter,
    }))
}

fn landlock_rules(abi: u32, server_dir: &Path, java_home: &Path) -> io::Result<OwnedFd> {
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
        handled_access_net: 0,
        scoped: if abi >= 6 {
            SCOPE_ABSTRACT_UNIX_SOCKET | SCOPE_SIGNAL
        } else {
            0
        },
    };
    // Los kernels antiguos solo conocen el primer campo (o los dos primeros).
    let size = match abi {
        ..=3 => 8,
        4 | 5 => 16,
        _ => std::mem::size_of::<RulesetAttr>(),
    };
    // SAFETY: `attr` vive durante la llamada y `size` no pasa de su tamaño.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &attr as *const RulesetAttr,
            size,
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

    // Su carpeta: todo (crear, borrar, renombrar...). Su carpeta temporal y
    // su "carpeta personal" están dentro (ver `launcher`).
    add_rule(&ruleset, server_dir, handled)?;
    add_rule(&ruleset, java_home, read_exec)?;
    for dir in SYSTEM_READ_EXEC {
        add_rule_if_dir(&ruleset, Path::new(dir), read_exec)?;
    }
    for dir in SYSTEM_READ {
        add_rule_if_dir(&ruleset, Path::new(dir), read)?;
    }
    // Abrir con O_TRUNC (`> /dev/null`) cuenta como truncar.
    let dev = (READ_FILE | WRITE_FILE | TRUNCATE | IOCTL_DEV) & handled;
    for path in DEVICES {
        add_rule_if_exists(&ruleset, Path::new(path), dev)?;
    }
    for file in SYSTEM_FILES {
        if let Ok(target) = std::fs::canonicalize(file) {
            if target.is_file() {
                add_rule(&ruleset, &target, READ_FILE)?;
            }
        }
    }
    Ok(ruleset)
}

fn add_rule_if_dir(ruleset: &OwnedFd, path: &Path, access: u64) -> io::Result<()> {
    if path.is_dir() {
        add_rule(ruleset, path, access)?;
    }
    Ok(())
}

fn add_rule_if_exists(ruleset: &OwnedFd, path: &Path, access: u64) -> io::Result<()> {
    if path.exists() {
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

fn seccomp_available() -> bool {
    // SAFETY: solo consulta el modo de seccomp del proceso.
    unsafe { libc::prctl(libc::PR_GET_SECCOMP, 0, 0, 0, 0) >= 0 }
}

// Instrucciones de BPF clásico (linux/filter.h).
const LD_W_ABS: u16 = 0x20;
const JEQ: u16 = 0x15;
const JGE: u16 = 0x35;
const JSET: u16 = 0x45;
const RET: u16 = 0x06;

// Campos de `struct seccomp_data`.
const NR: u32 = 0;
const ARCH: u32 = 4;
/// Mitad baja (little-endian) del argumento `i`.
const fn arg(i: u32) -> u32 {
    16 + 8 * i
}

const ALLOW: u32 = libc::SECCOMP_RET_ALLOW;
const fn errno(e: i32) -> u32 {
    libc::SECCOMP_RET_ERRNO | (e as u32 & libc::SECCOMP_RET_DATA)
}

/// Llamadas que un servidor de Minecraft nunca necesita y que abren puertas
/// fuera del aislamiento o al núcleo del sistema.
const DENIED: &[libc::c_long] = &[
    // Espacios de nombres y montajes.
    libc::SYS_unshare,
    libc::SYS_setns,
    libc::SYS_mount,
    libc::SYS_umount2,
    libc::SYS_pivot_root,
    libc::SYS_chroot,
    libc::SYS_open_tree,
    libc::SYS_move_mount,
    libc::SYS_fsopen,
    libc::SYS_fsconfig,
    libc::SYS_fsmount,
    libc::SYS_fspick,
    libc::SYS_mount_setattr,
    // Llavero del kernel (claves de la sesión).
    libc::SYS_add_key,
    libc::SYS_request_key,
    libc::SYS_keyctl,
    // Leer o manipular otros procesos.
    libc::SYS_ptrace,
    libc::SYS_process_vm_readv,
    libc::SYS_process_vm_writev,
    libc::SYS_process_madvise,
    libc::SYS_pidfd_getfd,
    libc::SYS_kcmp,
    // Superficie del kernel que usan los exploits.
    libc::SYS_bpf,
    libc::SYS_perf_event_open,
    libc::SYS_userfaultfd,
    libc::SYS_kexec_load,
    libc::SYS_kexec_file_load,
    libc::SYS_init_module,
    libc::SYS_finit_module,
    libc::SYS_delete_module,
    libc::SYS_open_by_handle_at,
    libc::SYS_quotactl,
    libc::SYS_swapon,
    libc::SYS_swapoff,
    libc::SYS_reboot,
    libc::SYS_acct,
    libc::SYS_syslog,
];

/// "No implementadas": quien las usa vuelve a la llamada clásica (glibc pasa
/// de clone3 a clone, que sí se puede revisar; Netty, de io_uring a epoll).
/// io_uring además haría las operaciones sin pasar por este filtro.
const UNSUPPORTED: &[libc::c_long] = &[
    libc::SYS_clone3,
    libc::SYS_io_uring_setup,
    libc::SYS_io_uring_enter,
    libc::SYS_io_uring_register,
];

/// Banderas de `clone` que crean espacios de nombres.
const CLONE_NAMESPACES: u32 = (libc::CLONE_NEWNS
    | libc::CLONE_NEWCGROUP
    | libc::CLONE_NEWUTS
    | libc::CLONE_NEWIPC
    | libc::CLONE_NEWUSER
    | libc::CLONE_NEWPID
    | libc::CLONE_NEWNET) as u32;

/// Programa de seccomp, o `None` en arquitecturas que no conocemos.
fn seccomp_filter() -> Option<Vec<libc::sock_filter>> {
    #[cfg(target_arch = "x86_64")]
    const AUDIT_ARCH: u32 = 0xC000_003E;
    #[cfg(target_arch = "aarch64")]
    const AUDIT_ARCH: u32 = 0xC000_00B7;
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    return None;

    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    {
        let mut p = Program::default();
        // Llamadas de otra arquitectura (int 0x80 en x86_64): los números no
        // significan lo mismo, así que se rechazan todas.
        p.load(ARCH);
        p.jump(JEQ, AUDIT_ARCH, 1, 0);
        p.ret(errno(libc::EPERM));
        p.load(NR);
        #[cfg(target_arch = "x86_64")]
        {
            // ABI x32: los mismos números con un bit extra.
            p.jump(JGE, 0x4000_0000, 0, 1);
            p.ret(errno(libc::EPERM));
        }
        for &nr in DENIED {
            p.deny(nr, errno(libc::EPERM));
        }
        for &nr in UNSUPPORTED {
            p.deny(nr, errno(libc::ENOSYS));
        }

        // socket(): solo internet y la lista de interfaces de red
        // (NETLINK_ROUTE). Nada de sockets Unix: por ellos se llega a D-Bus,
        // al escritorio, al agente SSH y a otros programas de la sesión.
        p.jump(JEQ, libc::SYS_socket as u32, 0, 8);
        p.load(arg(0));
        p.jump(JEQ, libc::AF_INET as u32, 5, 0);
        p.jump(JEQ, libc::AF_INET6 as u32, 4, 0);
        p.jump(JEQ, libc::AF_NETLINK as u32, 0, 2);
        p.load(arg(2));
        p.jump(JEQ, libc::NETLINK_ROUTE as u32, 1, 0);
        p.ret(errno(libc::EACCES));
        p.ret(ALLOW);

        // clone(): hilos y procesos sí; espacios de nombres no.
        p.jump(JEQ, libc::SYS_clone as u32, 0, 4);
        p.load(arg(0));
        p.jump(JSET, CLONE_NAMESPACES, 0, 1);
        p.ret(errno(libc::EPERM));
        p.ret(ALLOW);

        // ioctl(): ni meter teclas en la terminal (TIOCSTI) ni TIOCLINUX.
        // El kernel solo mira 32 bits de la orden, y eso se compara.
        p.jump(JEQ, libc::SYS_ioctl as u32, 0, 5);
        p.load(arg(1));
        p.jump(JEQ, libc::TIOCSTI as u32, 2, 0);
        p.jump(JEQ, libc::TIOCLINUX as u32, 1, 0);
        p.ret(ALLOW);
        p.ret(errno(libc::EPERM));

        p.ret(ALLOW);
        Some(p.0)
    }
}

#[derive(Default)]
struct Program(Vec<libc::sock_filter>);

impl Program {
    fn load(&mut self, offset: u32) {
        self.push(LD_W_ABS, 0, 0, offset);
    }

    fn jump(&mut self, op: u16, k: u32, jt: u8, jf: u8) {
        self.push(op, jt, jf, k);
    }

    fn ret(&mut self, action: u32) {
        self.push(RET, 0, 0, action);
    }

    /// Con el número de la llamada cargado: si es `nr`, devuelve `action`.
    fn deny(&mut self, nr: libc::c_long, action: u32) {
        self.jump(JEQ, nr as u32, 0, 1);
        self.ret(action);
    }

    fn push(&mut self, code: u16, jt: u8, jf: u8, k: u32) {
        self.0.push(libc::sock_filter { code, jt, jf, k });
    }
}

/// Registra el aislamiento para que se aplique en el hijo antes del `exec`.
/// `prepared` debe seguir vivo hasta que `spawn` termine.
pub fn apply_pre_exec(cmd: &mut tokio::process::Command, prepared: &Prepared) {
    let ruleset: Option<RawFd> = prepared.ruleset.as_ref().map(|r| r.as_raw_fd());
    // Dirección y longitud: un puntero no puede viajar en el cierre.
    let filter = prepared
        .filter
        .as_ref()
        .map(|f| (f.as_ptr() as usize, f.len() as libc::c_ushort));
    // SAFETY: el cierre se ejecuta en el hijo tras `fork` y solo hace llamadas
    // al sistema seguras en ese contexto, sin reservar memoria. El programa
    // de seccomp vive en `prepared`, que sigue vivo durante `spawn`.
    unsafe {
        cmd.pre_exec(move || {
            // Sesión propia: sin terminal de control no puede escribir en
            // la de la persona ni recibir sus Ctrl+C.
            libc::setsid();
            close_inherited_fds();
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            if let Some(fd) = ruleset {
                if libc::syscall(libc::SYS_landlock_restrict_self, fd, 0u32) != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            if let Some((ptr, len)) = filter {
                let program = libc::sock_fprog {
                    len,
                    filter: ptr as *mut libc::sock_filter,
                };
                if libc::syscall(
                    libc::SYS_seccomp,
                    libc::SECCOMP_SET_MODE_FILTER,
                    0u32,
                    &program as *const libc::sock_fprog,
                ) != 0
                {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
}

/// Ningún descriptor de Micra (salvo la entrada y las salidas) pasa al
/// servidor: se marcan para cerrarse en el `exec`.
///
/// # Safety
/// Solo en el hijo, entre `fork` y `exec`.
unsafe fn close_inherited_fds() {
    if libc::syscall(
        libc::SYS_close_range,
        3u32,
        u32::MAX,
        libc::CLOSE_RANGE_CLOEXEC,
    ) == 0
    {
        return;
    }
    // Kernel anterior a 5.11.
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    let max = if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) == 0 {
        limit.rlim_cur.min(65_536) as RawFd
    } else {
        1024
    };
    for fd in 3..max {
        libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn run_isolated(server: &Path, script: &str) -> String {
        // "Java" falso: /usr como JAVA_HOME para poder ejecutar sh y python.
        let prepared = prepare(server, Path::new("/usr")).unwrap().unwrap();
        let mut cmd = tokio::process::Command::new("/bin/sh");
        cmd.arg("-c").arg(script).current_dir(server);
        apply_pre_exec(&mut cmd, &prepared);
        let out = cmd.output().await.unwrap();
        drop(prepared);
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
    }

    /// Lanza `sh` aislado y comprueba que puede escribir en su carpeta pero
    /// no leer ni escribir fuera de ella (tampoco en /tmp ni en /dev/shm).
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
        let tmp_name = format!("micra-test-{}", std::process::id());

        let script = format!(
            "echo ok > {s}/world.txt && cat {s}/world.txt; \
             cat {p}/passwords.txt && echo LEIDO; \
             echo x > {p}/autostart.sh && echo ESCRITO; \
             echo x > /tmp/{t} && echo TMP; \
             echo x > /dev/shm/{t} && echo SHM; \
             echo x > /dev/null && echo NULL; true",
            s = server.display(),
            p = secret_dir.display(),
            t = tmp_name,
        );
        let out = run_isolated(&server, &script).await;
        assert!(out.contains("ok"), "debería escribir en su carpeta: {out}");
        assert!(out.contains("NULL"), "debería poder usar /dev/null: {out}");
        assert!(!out.contains("LEIDO"), "no debería leer fuera: {out}");
        assert!(!out.contains("ESCRITO"), "no debería escribir fuera: {out}");
        assert!(!out.contains("TMP"), "no debería escribir en /tmp: {out}");
        assert!(
            !out.contains("SHM"),
            "no debería escribir en /dev/shm: {out}"
        );
        assert!(!secret_dir.join("autostart.sh").exists());
        let _ = std::fs::remove_file(format!("/tmp/{tmp_name}"));
        let _ = std::fs::remove_file(format!("/dev/shm/{tmp_name}"));
    }

    /// Un Java de verdad arranca y funciona aislado: red, hilos, DNS,
    /// selectores y su carpeta temporal; sin sockets Unix ni archivos de fuera.
    #[tokio::test]
    async fn real_java_works_isolated() {
        let java = std::env::var_os("JAVA_HOME")
            .map(|h| Path::new(&h).join("bin/java"))
            .filter(|j| j.exists())
            .or_else(|| Some(Path::new("/usr/bin/java").to_path_buf()).filter(|j| j.exists()));
        let Some(java) = java.and_then(|j| std::fs::canonicalize(j).ok()) else {
            eprintln!("sin Java: se omite la prueba");
            return;
        };
        if abi_version().is_none() {
            eprintln!("Landlock no disponible: se omite la prueba");
            return;
        }
        let java_home = java.parent().and_then(Path::parent).unwrap().to_path_buf();
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let server = root.path().join("server");
        let tmp = server.join(".micra/tmp");
        let home = server.join(".micra/home");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let secret = root.path().join("secreto.txt");
        std::fs::write(&secret, "x").unwrap();
        std::fs::write(
            server.join("Probe.java"),
            r#"import java.net.*; import java.nio.channels.*; import java.nio.file.*; import java.util.concurrent.*;
public class Probe {
  public static void main(String[] a) throws Exception {
    try (ServerSocket s = new ServerSocket(0)) { System.out.println("BIND OK"); }
    System.out.println("DNS " + InetAddress.getByName("localhost").isLoopbackAddress());
    ExecutorService ex = Executors.newFixedThreadPool(4);
    ex.submit(() -> System.out.println("THREAD OK")).get();
    ex.shutdown();
    System.out.println("TMP " + Files.createTempFile("probe", ".tmp").startsWith(System.getProperty("java.io.tmpdir")));
    try { Files.readAllBytes(Path.of(a[0])); System.out.println("SECRET READ"); } catch (Exception e) { System.out.println("SECRET DENIED"); }
    ProtocolFamily unix = null;
    try { unix = StandardProtocolFamily.valueOf("UNIX"); } catch (IllegalArgumentException e) { System.out.println("UNIX N/A"); }
    if (unix != null) {
      try (SocketChannel c = (SocketChannel) SocketChannel.class.getMethod("open", ProtocolFamily.class).invoke(null, unix)) { System.out.println("UNIX OPEN"); }
      catch (java.lang.reflect.InvocationTargetException e) { System.out.println("UNIX DENIED"); }
    }
    try (Selector sel = Selector.open()) { System.out.println("SELECTOR OK"); }
  }
}
"#,
        )
        .unwrap();
        let prepared = prepare(&server, &java_home).unwrap().unwrap();
        let mut cmd = tokio::process::Command::new(&java);
        cmd.env_clear()
            .env("PATH", "/usr/bin:/bin")
            .arg("-XX:+PerfDisableSharedMem")
            .arg(format!("-Djava.io.tmpdir={}", tmp.display()))
            .arg(format!("-Duser.home={}", home.display()))
            .arg("-Djava.awt.headless=true")
            .arg("Probe.java")
            .arg(&secret)
            .current_dir(&server);
        apply_pre_exec(&mut cmd, &prepared);
        let out = cmd.output().await.unwrap();
        drop(prepared);
        let text = String::from_utf8_lossy(&out.stdout).into_owned()
            + &String::from_utf8_lossy(&out.stderr);
        for expected in [
            "BIND OK",
            "DNS true",
            "THREAD OK",
            "TMP true",
            "SECRET DENIED",
            "SELECTOR OK",
        ] {
            assert!(text.contains(expected), "falta «{expected}»: {text}");
        }
        // Java abre sockets Unix desde la 16; la 11 no tiene con qué.
        assert!(
            text.contains("UNIX DENIED") || text.contains("UNIX N/A"),
            "falta «UNIX DENIED»: {text}"
        );
    }

    /// seccomp: sin sockets Unix ni espacios de nombres, con internet; y
    /// (Landlock 6) sin señales a procesos de fuera.
    #[tokio::test]
    async fn blocks_unix_sockets_namespaces_and_signals() {
        if !seccomp_available() || seccomp_filter().is_none() {
            eprintln!("seccomp no disponible: se omite la prueba");
            return;
        }
        if !Path::new("/usr/bin/python3").exists() {
            eprintln!("sin python3: se omite la prueba");
            return;
        }
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let server = root.path().join("server");
        std::fs::create_dir_all(&server).unwrap();
        // os.unshare llegó en Python 3.12 (Ubuntu 22.04 trae la 3.10), así
        // que unshare se llama directamente.
        let py = format!(
            "import ctypes, socket\n\
            libc = ctypes.CDLL(None, use_errno=True)\n\
            def tryit(name, f):\n\
            \x20   try:\n\
            \x20       f()\n\
            \x20       print(name + ' OK')\n\
            \x20   except OSError as e:\n\
            \x20       print(name + ' BLOQUEADO ' + str(e.errno))\n\
            def unshare_user():\n\
            \x20   if libc.unshare({newuser}) != 0:\n\
            \x20       raise OSError(ctypes.get_errno(), 'unshare')\n\
            tryit('unix', lambda: socket.socket(socket.AF_UNIX, socket.SOCK_STREAM))\n\
            tryit('inet', lambda: socket.socket(socket.AF_INET, socket.SOCK_STREAM))\n\
            tryit('inet6', lambda: socket.socket(socket.AF_INET6, socket.SOCK_DGRAM))\n\
            tryit('par', lambda: socket.socketpair())\n\
            tryit('userns', unshare_user)\n\
            tryit('hilo', lambda: __import__('threading').Thread(target=lambda: None).start())\n",
            newuser = libc::CLONE_NEWUSER,
        );
        std::fs::write(server.join("t.py"), py).unwrap();
        let out = run_isolated(&server, "/usr/bin/python3 -B t.py").await;
        assert!(out.contains("unix BLOQUEADO 13"), "{out}");
        assert!(out.contains("inet OK"), "{out}");
        // Sin IPv6 en el equipo, el error es otro (EAFNOSUPPORT), no el nuestro.
        assert!(!out.contains("inet6 BLOQUEADO 13"), "{out}");
        assert!(out.contains("par OK"), "{out}");
        assert!(out.contains("userns BLOQUEADO 1"), "{out}");
        assert!(out.contains("hilo OK"), "{out}");
        if abi_version().is_some_and(|abi| abi >= 6) {
            // El padre de sh es esta prueba, fuera del aislamiento.
            let out = run_isolated(&server, "kill -0 $PPID && echo SENAL").await;
            assert!(!out.contains("SENAL"), "{out}");
        }
    }
}
