//! Escáner de malware conocido en los mods (`.jar`).
//!
//! No es un antivirus: busca indicadores concretos de malware que ya ha
//! circulado en mods de Minecraft (sobre todo fractureiser, 2023) y patrones
//! típicos de los ladrones de cuentas. Mira:
//!
//! - las cadenas de cada `.class` (constant pool),
//! - las cadenas que el código monta byte a byte para esconderlas
//!   (`new String(new byte[]{56, 53, ...})`, el truco de fractureiser),
//! - los nombres de las entradas del jar,
//! - los jars anidados (jar-in-jar de Forge y Fabric), con límites para que
//!   un ZIP malicioso no agote la memoria.
//!
//! Se analizan todos los jars del servidor: los de `mods/` y `plugins/` con
//! todas las reglas, y el resto (el jar de arranque, `libraries/`...) solo
//! buscando malware conocido, porque un pack también puede esconderlo ahí.
//!
//! Los resultados se guardan por SHA-1 fuera de la carpeta del servidor
//! (`servers/.configs/<id>.scan.json`), donde un mod no puede marcarse como
//! limpio; si cambian las reglas (`RULES_VERSION`), todo se vuelve a analizar.

use std::collections::HashMap;
use std::io::{Cursor, Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::fsutil::{walk_files, write_atomic_sync};
use crate::{tr, Error, Result};

/// Súbelo al cambiar las reglas: invalida la caché.
const RULES_VERSION: u32 = 2;
/// Extensión de la caché en `servers/.configs`.
pub const CACHE_EXT: &str = "scan.json";
/// Donde estaba la caché hasta la 0.3.0 (dentro del servidor).
const OLD_CACHE_FILE: &str = ".micra/scan-cache.json";
pub const QUARANTINE_DIR: &str = ".micra/quarantine";

/// Límites contra ZIPs maliciosos.
const MAX_DEPTH: u32 = 3;
const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    /// Patrón propio de los ladrones de cuentas: la persona decide.
    Suspicious,
    /// Malware conocido: el servidor no arranca con él.
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub severity: Severity,
    /// Identificador de la regla (`fractureiser`, `discord-webhook`...).
    pub rule: String,
    /// Dónde: entrada del jar (o ruta del equipo en los hallazgos del sistema).
    pub location: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileReport {
    /// Ruta relativa a la carpeta del servidor, con `/`.
    pub path: String,
    pub sha1: String,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub scanned: usize,
    /// Solo los archivos con algún hallazgo.
    pub files: Vec<FileReport>,
    /// Restos de malware en el propio equipo (fuera del servidor).
    pub host: Vec<Finding>,
}

impl ScanReport {
    pub fn critical_files(&self) -> Vec<String> {
        self.files
            .iter()
            .filter(|f| f.findings.iter().any(|x| x.severity == Severity::Critical))
            .map(|f| f.path.clone())
            .collect()
    }

    pub fn has_critical(&self) -> bool {
        !self.critical_files().is_empty()
            || self.host.iter().any(|f| f.severity == Severity::Critical)
    }
}

struct Rule {
    id: &'static str,
    severity: Severity,
    /// Se comparan en minúsculas.
    needles: &'static [&'static str],
}

const RULES: &[Rule] = &[
    Rule {
        id: "fractureiser",
        severity: Severity::Critical,
        needles: &[
            "85.217.144.130",
            "107.189.3.101",
            "files-8ie.pages.dev",
            "dev/neko/nekoclient",
            "dev/neko/nekoinjector",
            "libwebgl64.jar",
            "systemd-utility.service",
        ],
    },
    Rule {
        id: "discord-webhook",
        severity: Severity::Suspicious,
        needles: &["discord.com/api/webhooks/", "discordapp.com/api/webhooks/"],
    },
    // Exfiltración por un bot de Telegram, típica de los ladrones de 2024-2025.
    Rule {
        id: "telegram-bot",
        severity: Severity::Suspicious,
        needles: &["api.telegram.org/bot"],
    },
    Rule {
        id: "account-stealer",
        severity: Severity::Suspicious,
        needles: &[
            "local storage\\leveldb",
            "local storage/leveldb",
            "launcher_accounts.json",
            "microsoft_accounts.json",
            "\\default\\login data",
            "/default/login data",
            "\\google\\chrome\\user data",
            "\\network\\cookies",
            "/network/cookies",
            // Prefijo de los tokens cifrados de Discord.
            "dqw4w9wgxcq:",
            // Cuentas de otros launchers (Lunar, TLauncher, Feather).
            ".lunarclient/settings/game/accounts.json",
            ".lunarclient\\settings\\game\\accounts.json",
            "tlauncherprofiles.json",
            "feather/accounts.json",
            "feather\\accounts.json",
            // Monederos de criptomonedas.
            "exodus.wallet",
            "nkbihfbeogaeaoehlefnkodbefgpgknn",
        ],
    },
    Rule {
        id: "persistence",
        severity: Severity::Suspicious,
        needles: &[
            "currentversion\\run",
            "start menu\\programs\\startup",
            "schtasks /create",
            ".config/autostart/",
            "systemctl --user enable",
            // Descargas y ejecución de PowerShell típicas de un dropper.
            "-encodedcommand",
            "invoke-webrequest",
            "downloadstring(",
            "certutil -urlcache",
            "bitsadmin /transfer",
        ],
    },
];

/// Prefijos de entradas que delatan malware conocido.
const BAD_ENTRY_PREFIXES: &[(&str, &str)] = &[("dev/neko/", "fractureiser")];

fn match_rules(text: &str, location: &str, out: &mut Vec<Finding>) {
    let lower = text.to_ascii_lowercase();
    for rule in RULES {
        if rule.needles.iter().any(|n| lower.contains(n)) && !out.iter().any(|f| f.rule == rule.id)
        {
            out.push(Finding {
                severity: rule.severity,
                rule: rule.id.to_string(),
                location: location.to_string(),
            });
        }
    }
}

/// Analiza un jar ya abierto. `prefix` es la ruta del jar contenedor (para
/// los anidados).
fn scan_zip<R: Read + Seek>(
    reader: R,
    prefix: &str,
    depth: u32,
    budget: &mut u64,
    out: &mut Vec<Finding>,
) -> Result<()> {
    let mut zip = zip::ZipArchive::new(reader)?;
    if zip.len() > MAX_ENTRIES {
        out.push(Finding {
            severity: Severity::Suspicious,
            rule: "zip-bomb".into(),
            location: prefix.to_string(),
        });
        return Ok(());
    }
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().replace('\\', "/");
        let location = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}!/{name}")
        };
        let lower = name.to_ascii_lowercase();
        for (bad, rule) in BAD_ENTRY_PREFIXES {
            if lower.starts_with(bad) && !out.iter().any(|f| f.rule == *rule) {
                out.push(Finding {
                    severity: Severity::Critical,
                    rule: rule.to_string(),
                    location: location.clone(),
                });
            }
        }
        let is_class = lower.ends_with(".class");
        let is_jar = lower.ends_with(".jar") && depth < MAX_DEPTH;
        if !is_class && !is_jar {
            continue;
        }
        let size = entry.size();
        if size > MAX_ENTRY_BYTES || size > *budget {
            out.push(Finding {
                severity: Severity::Suspicious,
                rule: "zip-bomb".into(),
                location,
            });
            continue;
        }
        *budget -= size;
        let mut data = Vec::with_capacity(size as usize);
        // `take` por si el tamaño declarado miente.
        (&mut entry).take(size + 1).read_to_end(&mut data)?;
        if data.len() as u64 > size {
            continue;
        }
        if is_class {
            for s in class_strings(&data) {
                match_rules(&s, &location, out);
            }
        } else if let Err(e) = scan_zip(Cursor::new(data), &location, depth + 1, budget, out) {
            tracing::debug!("jar anidado ilegible {location}: {e}");
        }
    }
    Ok(())
}

/// Analiza un jar del disco.
pub fn scan_jar(path: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    let mut budget = 1024 * 1024 * 1024;
    scan_zip(
        std::fs::File::open(path)?,
        "",
        0,
        &mut budget,
        &mut findings,
    )?;
    findings.sort_by_key(|f| std::cmp::Reverse(f.severity));
    Ok(findings)
}

/// Cadenas de un `.class`: las del constant pool y las montadas con
/// arrays de bytes en el código.
pub fn class_strings(class: &[u8]) -> Vec<String> {
    let mut out = constant_pool_strings(class).unwrap_or_default();
    out.extend(byte_array_strings(class));
    out
}

fn constant_pool_strings(class: &[u8]) -> Option<Vec<String>> {
    if class.len() < 10 || class[..4] != [0xCA, 0xFE, 0xBA, 0xBE] {
        return None;
    }
    let count = u16::from_be_bytes([class[8], class[9]]) as usize;
    let mut pos = 10;
    let mut out = Vec::new();
    let mut i = 1;
    while i < count {
        let tag = *class.get(pos)?;
        pos += 1;
        let len = match tag {
            1 => {
                let n = u16::from_be_bytes([*class.get(pos)?, *class.get(pos + 1)?]) as usize;
                let bytes = class.get(pos + 2..pos + 2 + n)?;
                out.push(String::from_utf8_lossy(bytes).into_owned());
                2 + n
            }
            3 | 4 => 4,
            5 | 6 => {
                // Ocupan dos entradas.
                i += 1;
                8
            }
            7 | 8 | 16 | 19 | 20 => 2,
            15 => 3,
            9 | 10 | 11 | 12 | 17 | 18 => 4,
            _ => return Some(out),
        };
        pos += len;
        i += 1;
    }
    Some(out)
}

/// Busca en el bytecode la secuencia `dup, <índice>, <valor>, bastore`
/// repetida con índices 0, 1, 2... y reconstruye la cadena resultante.
fn byte_array_strings(class: &[u8]) -> Vec<String> {
    // Lee una constante entera empujada a la pila en `pos`.
    fn push_value(class: &[u8], pos: usize) -> Option<(i32, usize)> {
        match *class.get(pos)? {
            op @ 0x02..=0x08 => Some((op as i32 - 3, 1)), // iconst_m1..iconst_5
            0x10 => Some((*class.get(pos + 1)? as i8 as i32, 2)), // bipush
            0x11 => Some((
                i16::from_be_bytes([*class.get(pos + 1)?, *class.get(pos + 2)?]) as i32,
                3,
            )), // sipush
            _ => None,
        }
    }

    let mut out = Vec::new();
    let mut pos = 0;
    while pos < class.len() {
        let mut bytes: Vec<u8> = Vec::new();
        let mut p = pos;
        loop {
            if class.get(p) != Some(&0x59) {
                break; // dup
            }
            let Some((index, a)) = push_value(class, p + 1) else {
                break;
            };
            let Some((value, b)) = push_value(class, p + 1 + a) else {
                break;
            };
            if index != bytes.len() as i32 || class.get(p + 1 + a + b) != Some(&0x54) {
                break; // bastore
            }
            bytes.push(value as u8);
            p += 1 + a + b + 1;
        }
        if bytes.len() >= 4 {
            out.push(String::from_utf8_lossy(&bytes).into_owned());
            pos = p;
        } else {
            pos += 1;
        }
    }
    out
}

/// Rastros de fractureiser en el propio equipo (fuera de los servidores).
pub fn check_host() -> Vec<Finding> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            candidates.push(
                PathBuf::from(local)
                    .join("Microsoft Edge")
                    .join("libWebGL64.jar"),
            );
        }
    } else if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        candidates.push(home.join(".config/.data/lib.jar"));
        candidates.push(home.join(".config/systemd/user/systemd-utility.service"));
        candidates.push(PathBuf::from("/etc/systemd/system/systemd-utility.service"));
    }
    candidates
        .into_iter()
        .filter(|p| p.exists())
        .map(|p| Finding {
            severity: Severity::Critical,
            rule: "fractureiser-host".into(),
            location: p.display().to_string(),
        })
        .collect()
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    version: u32,
    /// SHA-1 → hallazgos.
    entries: HashMap<String, Vec<Finding>>,
}

const MOD_DIRS: [&str; 2] = ["mods", "plugins"];

fn is_mod_path(rel: &str) -> bool {
    MOD_DIRS
        .iter()
        .any(|d| rel.strip_prefix(d).is_some_and(|r| r.starts_with('/')))
}

fn is_jar(path: &Path) -> bool {
    let n = path.to_string_lossy().to_ascii_lowercase();
    n.ends_with(".jar") || n.ends_with(".jar.disabled")
}

/// Jars que se analizan: todo `mods/` y `plugins/` (activos y desactivados,
/// también si la carpeta es un enlace que puso la persona) y cualquier otro
/// jar de la carpeta del servidor, salvo lo que guarda Micra (`.micra`: la
/// cuarentena y las copias de los mods quitados).
fn jars_to_scan(server_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in MOD_DIRS {
        if let Ok(files) = walk_files(&server_dir.join(dir)) {
            out.extend(files.into_iter().map(|(p, _)| p).filter(|p| is_jar(p)));
        }
    }
    if let Ok(entries) = std::fs::read_dir(server_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if name == ".micra" || MOD_DIRS.contains(&name.as_str()) || kind.is_symlink() {
                continue;
            }
            if kind.is_file() && is_jar(&entry.path()) {
                out.push(entry.path());
            } else if kind.is_dir() {
                if let Ok(files) = walk_files(&entry.path()) {
                    out.extend(files.into_iter().map(|(p, _)| p).filter(|p| is_jar(p)));
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

pub fn sha1_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Analiza todos los jars del servidor (síncrono: desde `spawn_blocking`).
/// `progress(hechos, total)`.
pub fn scan_server_sync(
    server_dir: &Path,
    progress: Option<&(dyn Fn(usize, usize) + Sync)>,
) -> Result<ScanReport> {
    let cache_path = crate::server::config::trusted_path(server_dir, CACHE_EXT);
    let _ = std::fs::remove_file(server_dir.join(OLD_CACHE_FILE));
    let mut cache: Cache = std::fs::read(&cache_path)
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .filter(|c: &Cache| c.version == RULES_VERSION)
        .unwrap_or_default();
    cache.version = RULES_VERSION;

    let jars = jars_to_scan(server_dir);
    let total = jars.len();
    let done = AtomicUsize::new(0);
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(PathBuf, String, Vec<Finding>)>> = Mutex::new(Vec::new());
    let cached = &cache.entries;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, 8);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(jar) = jars.get(i) else { break };
                let outcome = sha1_file(jar).map(|sha1| {
                    let findings = match cached.get(&sha1) {
                        Some(f) => f.clone(),
                        None => scan_jar(jar).unwrap_or_else(|e| {
                            tracing::debug!("jar ilegible {}: {e}", jar.display());
                            Vec::new()
                        }),
                    };
                    (sha1, findings)
                });
                if let Ok((sha1, findings)) = outcome {
                    results.lock().unwrap().push((jar.clone(), sha1, findings));
                }
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if let Some(cb) = progress {
                    cb(n, total);
                }
            });
        }
    });

    let mut report = ScanReport {
        scanned: total,
        files: Vec::new(),
        host: check_host(),
    };
    let mut entries = HashMap::new();
    for (path, sha1, findings) in results.into_inner().unwrap() {
        let rel = relative(server_dir, &path);
        // Fuera de mods/ y plugins/ solo cuenta el malware conocido: las
        // librerías normales también hablan de cookies o del registro.
        let shown: Vec<Finding> = if is_mod_path(&rel) {
            findings.clone()
        } else {
            findings
                .iter()
                .filter(|f| f.severity == Severity::Critical)
                .cloned()
                .collect()
        };
        if !shown.is_empty() {
            report.files.push(FileReport {
                path: rel,
                sha1: sha1.clone(),
                findings: shown,
            });
        }
        entries.insert(sha1, findings);
    }
    report.files.sort_by(|a, b| a.path.cmp(&b.path));
    // Solo se guardan los jars que siguen ahí.
    cache.entries = entries;
    if let Err(e) = write_atomic_sync(&cache_path, &serde_json::to_vec(&cache)?) {
        tracing::warn!("no se pudo guardar la caché del escáner: {e}");
    }
    Ok(report)
}

pub async fn scan_server(
    server_dir: &Path,
    progress: Option<std::sync::Arc<dyn Fn(usize, usize) + Send + Sync>>,
) -> Result<ScanReport> {
    let dir = server_dir.to_owned();
    tokio::task::spawn_blocking(move || {
        let cb = progress.as_ref().map(|p| {
            let p = p.clone();
            move |done: usize, total: usize| p(done, total)
        });
        scan_server_sync(
            &dir,
            cb.as_ref().map(|c| c as &(dyn Fn(usize, usize) + Sync)),
        )
    })
    .await
    .expect("el análisis no debería entrar en pánico")
}

fn relative(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Mueve un archivo del servidor a la cuarentena (`.micra/quarantine`),
/// donde ningún loader lo carga. La ruta llega de la interfaz y se valida.
pub async fn quarantine(server_dir: &Path, rel: &str) -> Result<()> {
    let rel_norm = rel.replace('\\', "/");
    let lower = rel_norm.to_ascii_lowercase();
    let allowed = (lower.ends_with(".jar") || lower.ends_with(".jar.disabled"))
        && !lower.starts_with(".micra/")
        && !rel_norm.split('/').any(|p| p == ".." || p.is_empty());
    if !allowed {
        return Err(Error::UnsafePath(rel.to_string()));
    }
    let source = crate::archive::safe_join_no_links(server_dir, &rel_norm)?;
    let name = source
        .file_name()
        .ok_or_else(|| Error::UnsafePath(rel.to_string()))?
        .to_string_lossy()
        .into_owned();
    let dir = crate::fsutil::internal_dir(server_dir, QUARANTINE_DIR)?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    // `.quarantined` para que nada lo trate como un jar.
    let target = dir.join(format!("{stamp}_{name}.quarantined"));
    tokio::fs::rename(&source, &target).await?;
    Ok(())
}

/// Mensaje para el error que impide arrancar con malware.
pub fn malware_error(report: &ScanReport) -> Error {
    let mut names = report.critical_files();
    names.extend(report.host.iter().map(|f| f.location.clone()));
    let list = names.join(", ");
    if report.critical_files().is_empty() {
        Error::Malware(tr!(
            "tu equipo ({list}). Sigue la guía de limpieza de fractureiser antes de continuar",
            "your computer ({list}). Follow the fractureiser clean-up guide before continuing"
        ))
    } else {
        Error::Malware(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Un `.class` mínimo con un constant pool dado y un trozo de "código".
    fn class_with(utf8: &[&str], code: &[u8]) -> Vec<u8> {
        let mut c = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 61];
        c.extend(((utf8.len() + 2) as u16).to_be_bytes());
        for s in utf8 {
            c.push(1);
            c.extend((s.len() as u16).to_be_bytes());
            c.extend(s.as_bytes());
        }
        c.extend([5, 0, 0, 0, 0, 0, 0, 0, 1]); // un Long (dos entradas)
        c.extend(code);
        c
    }

    /// Bytecode de `new byte[]{...}` con dup/índice/valor/bastore.
    fn byte_array_code(text: &str) -> Vec<u8> {
        let mut code = vec![0x10, text.len() as u8, 0xBC, 8]; // bipush n; newarray byte
        for (i, b) in text.bytes().enumerate() {
            code.push(0x59);
            if i <= 5 {
                code.push(0x03 + i as u8);
            } else {
                code.extend([0x10, i as u8]);
            }
            code.extend([0x10, b]);
            code.push(0x54);
        }
        code
    }

    fn jar(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut buf);
        for (name, data) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn reads_constant_pool_and_hidden_strings() {
        let class = class_with(
            &["java/lang/Object", "hola"],
            &byte_array_code("85.217.144.130"),
        );
        let strings = class_strings(&class);
        assert!(strings.contains(&"java/lang/Object".to_string()));
        assert!(strings.contains(&"hola".to_string()));
        assert!(strings.contains(&"85.217.144.130".to_string()));
    }

    #[test]
    fn detects_fractureiser_even_nested() {
        let dir = tempfile::tempdir().unwrap();
        let evil = class_with(
            &["java/net/URLClassLoader"],
            &byte_array_code("85.217.144.130"),
        );
        let inner = jar(&[("a/Evil.class", evil)]);
        let outer = jar(&[
            ("fabric.mod.json", b"{}".to_vec()),
            ("META-INF/jars/inner.jar", inner),
        ]);
        let path = dir.path().join("mod.jar");
        std::fs::write(&path, outer).unwrap();
        let findings = scan_jar(&path).unwrap();
        assert_eq!(findings[0].severity, Severity::Critical);
        assert_eq!(findings[0].rule, "fractureiser");
        assert_eq!(
            findings[0].location,
            "META-INF/jars/inner.jar!/a/Evil.class"
        );
    }

    #[test]
    fn flags_entries_and_suspicious_strings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.jar");
        let class = class_with(&["https://discord.com/api/webhooks/123/abc"], &[]);
        std::fs::write(
            &path,
            jar(&[
                ("dev/neko/nekoclient/Client.class", vec![0, 1]),
                ("b/Hook.class", class),
            ]),
        )
        .unwrap();
        let findings = scan_jar(&path).unwrap();
        assert!(findings
            .iter()
            .any(|f| f.rule == "fractureiser" && f.severity == Severity::Critical));
        assert!(findings
            .iter()
            .any(|f| f.rule == "discord-webhook" && f.severity == Severity::Suspicious));
    }

    #[test]
    fn clean_jar_and_cache() {
        let root = tempfile::tempdir().unwrap();
        let server = root.path().join("servers").join("s");
        let mods = server.join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        let clean = class_with(&["net/minecraft/server/MinecraftServer", "Hello"], &[]);
        std::fs::write(mods.join("ok.jar"), jar(&[("a/A.class", clean)])).unwrap();
        let evil = class_with(&["http://107.189.3.101/x"], &[]);
        std::fs::write(mods.join("bad.jar.disabled"), jar(&[("b/B.class", evil)])).unwrap();
        // Una caché antigua dentro del servidor (un mod pudo escribirla).
        std::fs::create_dir_all(server.join(".micra")).unwrap();
        std::fs::write(server.join(OLD_CACHE_FILE), "{}").unwrap();

        let report = scan_server_sync(&server, None).unwrap();
        assert_eq!(report.scanned, 2);
        assert_eq!(
            report.critical_files(),
            vec!["mods/bad.jar.disabled".to_string()]
        );
        assert!(!server.join(OLD_CACHE_FILE).exists());
        let cache = root.path().join("servers/.configs/s.scan.json");
        assert!(cache.exists());
        // Segunda pasada desde la caché, mismo resultado.
        let again = scan_server_sync(&server, None).unwrap();
        assert_eq!(again.critical_files(), report.critical_files());

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(quarantine(&server, "mods/bad.jar.disabled"))
            .unwrap();
        assert!(!mods.join("bad.jar.disabled").exists());
        assert!(rt.block_on(quarantine(&server, "../etc/passwd")).is_err());
        assert!(rt.block_on(quarantine(&server, "world/level.dat")).is_err());
        let clean_report = scan_server_sync(&server, None).unwrap();
        assert!(!clean_report.has_critical() || !clean_report.host.is_empty());
    }

    /// Malware escondido fuera de `mods/` (el jar de arranque, `libraries/`)
    /// también se encuentra; las sospechas de una librería normal, no.
    #[test]
    fn scans_jars_outside_mods() {
        let root = tempfile::tempdir().unwrap();
        let server = root.path().join("servers").join("s");
        let libs = server.join("libraries/org/x");
        std::fs::create_dir_all(&libs).unwrap();
        let evil = class_with(&["files-8ie.pages.dev"], &[]);
        std::fs::write(
            server.join("fabric-server-launch.jar"),
            jar(&[("E.class", evil)]),
        )
        .unwrap();
        let chatty = class_with(&["https://discord.com/api/webhooks/1/x"], &[]);
        std::fs::write(libs.join("lib.jar"), jar(&[("L.class", chatty)])).unwrap();
        let report = scan_server_sync(&server, None).unwrap();
        assert_eq!(report.scanned, 2);
        assert_eq!(
            report.critical_files(),
            vec!["fabric-server-launch.jar".to_string()]
        );
        assert!(!report
            .files
            .iter()
            .any(|f| f.path.starts_with("libraries/")));
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(quarantine(&server, "fabric-server-launch.jar"))
            .unwrap();
        assert!(!server.join("fabric-server-launch.jar").exists());
    }

    #[test]
    fn truncated_classes_do_not_panic() {
        assert!(class_strings(&[0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 61, 0, 9, 1, 0, 200]).is_empty());
        assert!(class_strings(&[]).is_empty());
        let _ = byte_array_strings(&[0x59, 0x03, 0x10]);
    }
}
