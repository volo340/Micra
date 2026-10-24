//! Por qué no arrancó (o se cayó) un servidor con mods. Lee el final de la
//! consola y el informe de error que deja Minecraft en `crash-reports/`, y
//! señala qué mods hay que quitar: los que solo funcionan en el juego
//! (cliente), los que necesitan otro mod que falta o son de otra versión, los
//! repetidos y el que dio el error.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

use crate::providers::modrinth::ModrinthClient;
use crate::providers::search::dedupe_key;
use crate::server::mods::{ModInfo, MODS_DIR};

const CRASH_DIR: &str = "crash-reports";
/// Lo que se lee como mucho de un informe de error.
const MAX_REPORT: u64 = 1024 * 1024;
/// Mods que se señalan como mucho por las líneas de un error.
const MAX_SUSPECTS: usize = 3;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CrashReport {
    /// Mods que conviene quitar: primero los que señala el error.
    pub culprits: Vec<Culprit>,
    /// Mods que piden otros y no están instalados.
    pub missing: Vec<String>,
    /// El error en una línea, para quien quiera buscarlo.
    pub headline: Option<String>,
    /// Informe de Minecraft (en `crash-reports/`), si escribió uno.
    pub report_file: Option<String>,
    /// Falló al arrancar (no llegó a abrirse).
    pub during_start: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Culprit {
    /// Archivo en `mods/` (lo que se quita).
    pub file_name: String,
    pub mod_id: Option<String>,
    pub name: Option<String>,
    pub reason: CulpritReason,
    /// Lo señala el error; si no, es un mod que no hace falta en el servidor.
    pub in_log: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CulpritReason {
    /// Solo funciona en el juego (cliente), no en un servidor.
    ClientOnly,
    /// Necesita otro mod que no está instalado.
    MissingDependency { dependency: String },
    /// Es para otra versión (de Minecraft, del loader o de otro mod).
    WrongVersion { needs: String },
    /// Es de otro loader (un mod de Fabric en NeoForge...).
    WrongLoader { loader: String },
    /// No es compatible con otro mod instalado.
    Incompatible { other: String },
    /// Está instalado dos veces.
    Duplicate,
    /// Dio el error al cargar.
    Failed,
    /// Aparece en el error, pero puede no ser la causa.
    Suspected,
}

impl CulpritReason {
    /// Cuanto más bajo, más seguro es que el mod sea el problema.
    fn rank(&self) -> u8 {
        match self {
            Self::ClientOnly => 0,
            Self::MissingDependency { .. } => 1,
            Self::WrongLoader { .. } => 2,
            Self::WrongVersion { .. } => 3,
            Self::Incompatible { .. } => 4,
            Self::Duplicate => 5,
            Self::Failed => 6,
            Self::Suspected => 7,
        }
    }
}

/// Cómo se encuentra el archivo de un mod que nombra el error.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Locator {
    ModId(String),
    /// Nombre visible ("Sophisticated Backpacks") o id.
    Name(String),
    File(String),
    /// Clase de Java ("net/coderbot/iris/Iris"): se busca en los jars.
    Class(String),
    /// Configuración de mixins ("lithium.mixins.json"): se busca en los jars.
    Mixin(String),
}

/// Lo que se sabe del arranque que falló.
pub struct CrashInput<'a> {
    /// Las últimas líneas de la consola.
    pub console: &'a [String],
    /// El informe de `crash-reports/` de este arranque y su nombre.
    pub report: Option<(&'a str, &'a str)>,
    pub mods: &'a [ModInfo],
    /// Carpeta del servidor (para buscar clases dentro de los jars).
    pub server_dir: &'a Path,
    pub during_start: bool,
}

/// Lee el error y señala los mods. Bloquea (abre los jars): llamar desde
/// `spawn_blocking`.
pub fn diagnose(input: &CrashInput) -> CrashReport {
    // Los mods desactivados no se cargan: no pueden ser la causa.
    let mods: Vec<ModInfo> = input.mods.iter().filter(|m| m.enabled).cloned().collect();
    let mut lines: Vec<&str> = input.console.iter().map(String::as_str).collect();
    let report_lines: Vec<&str> = input
        .report
        .map(|(text, _)| text.lines().collect())
        .unwrap_or_default();
    lines.extend(&report_lines);

    let mut found = Findings::default();
    scan_lines(&lines, &mut found);
    // Las líneas del error principal: las del informe y, si no hay, las de la consola.
    let main = if report_lines.is_empty() {
        main_error(&lines)
    } else {
        main_error(&report_lines)
    };
    for locator in suspects(main) {
        found.add(locator, CulpritReason::Suspected);
    }

    let headline = headline(&report_lines).or_else(|| headline(&lines));
    let culprits = resolve(found.items, &mods, &input.server_dir.join(MODS_DIR));
    let installed: HashSet<String> = mods
        .iter()
        .filter_map(|m| m.mod_id.as_deref())
        .map(str::to_lowercase)
        .collect();
    let mut missing: Vec<String> = found
        .missing
        .into_iter()
        .filter(|d| !installed.contains(&d.to_lowercase()))
        .collect();
    missing.sort();
    missing.dedup();
    CrashReport {
        culprits,
        missing,
        headline,
        report_file: input.report.map(|(_, name)| name.to_string()),
        during_start: input.during_start,
    }
}

/// Añade los mods solo de cliente que no señaló el error: por sus metadatos
/// o porque Modrinth lo dice (`sha1s`).
pub fn add_client_only(report: &mut CrashReport, mods: &[ModInfo], sha1s: &HashSet<String>) {
    for m in mods {
        if !m.enabled || !(m.client_only || sha1s.contains(&m.sha1)) {
            continue;
        }
        match report
            .culprits
            .iter_mut()
            .find(|c| c.file_name == m.file_name)
        {
            // Si el error ya lo señala, ahora se sabe además por qué.
            Some(c) if c.reason.rank() > CulpritReason::ClientOnly.rank() => {
                c.reason = CulpritReason::ClientOnly
            }
            Some(_) => {}
            None => report.culprits.push(Culprit {
                file_name: m.file_name.clone(),
                mod_id: m.mod_id.clone(),
                name: m.name.clone(),
                reason: CulpritReason::ClientOnly,
                in_log: false,
            }),
        }
    }
    sort_culprits(&mut report.culprits);
}

/// SHA-1 de los mods que Modrinth dice que no funcionan en un servidor.
pub async fn client_only_on_modrinth(
    modrinth: &ModrinthClient,
    mods: &[ModInfo],
) -> crate::Result<HashSet<String>> {
    let hashes: Vec<String> = mods.iter().map(|m| m.sha1.clone()).collect();
    let versions = modrinth.versions_by_hashes(&hashes).await?;
    let mut ids: Vec<String> = versions.values().map(|v| v.project_id.clone()).collect();
    ids.sort();
    ids.dedup();
    let client_only: HashSet<String> = modrinth
        .projects(&ids)
        .await?
        .into_iter()
        .filter(|p| p.server_side == "unsupported")
        .map(|p| p.id)
        .collect();
    Ok(versions
        .into_iter()
        .filter(|(_, v)| client_only.contains(&v.project_id))
        .map(|(sha1, _)| sha1)
        .collect())
}

/// El informe de `crash-reports/` más reciente escrito desde `since`, con su
/// nombre.
pub async fn latest_report(server_dir: &Path, since: SystemTime) -> Option<(String, String)> {
    let dir = server_dir.join(CRASH_DIR);
    tokio::task::spawn_blocking(move || {
        let newest: PathBuf = std::fs::read_dir(&dir)
            .ok()?
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".txt"))
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .filter(|(modified, _)| *modified >= since)
            .max_by_key(|(modified, _)| *modified)?
            .1;
        let mut text = String::new();
        std::fs::File::open(&newest)
            .ok()?
            .take(MAX_REPORT)
            .read_to_string(&mut text)
            .ok()?;
        let name = format!("{CRASH_DIR}/{}", newest.file_name()?.to_string_lossy());
        Some((text, name))
    })
    .await
    .ok()
    .flatten()
}

/// Las últimas líneas de `logs/latest.log` (por si la consola perdió
/// alguna).
pub async fn log_tail(server_dir: &Path, max_lines: usize) -> Vec<String> {
    let path = server_dir.join("logs").join("latest.log");
    tokio::task::spawn_blocking(move || {
        use std::io::{Seek, SeekFrom};
        let mut file = std::fs::File::open(path).ok()?;
        let len = file.metadata().ok()?.len();
        file.seek(SeekFrom::Start(len.saturating_sub(MAX_REPORT)))
            .ok()?;
        let mut raw = Vec::new();
        file.read_to_end(&mut raw).ok()?;
        let text = String::from_utf8_lossy(&raw);
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        let skip = lines.len().saturating_sub(max_lines);
        Some(lines.into_iter().skip(skip).collect())
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_default()
}

#[derive(Default)]
struct Findings {
    items: Vec<(Locator, CulpritReason)>,
    missing: Vec<String>,
}

impl Findings {
    fn add(&mut self, locator: Locator, reason: CulpritReason) {
        if let CulpritReason::MissingDependency { dependency } = &reason {
            self.missing.push(dependency.clone());
        }
        self.items.push((locator, reason));
    }
}

/// Señales de error del log y del informe que nombran a un mod.
fn scan_lines(lines: &[&str], found: &mut Findings) {
    // Sección del informe de Forge/NeoForge para un mod que falló.
    let mut section: Option<(Option<String>, Option<String>)> = None;
    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim();

        if let Some(id) = mod_section(line) {
            section = Some((Some(id.to_string()), None));
            continue;
        }
        if line.starts_with("-- ") {
            section = None;
        }
        if let Some((id, file)) = section.as_mut() {
            if let Some(path) = strip_prefix_ci(line, "Mod File:") {
                *file = Some(file_of_path(path).to_string());
            }
            let locator = file
                .clone()
                .map(Locator::File)
                .or_else(|| id.clone().map(Locator::ModId));
            if let Some(locator) = locator {
                if is_client_marker(line) {
                    found.add(locator, CulpritReason::ClientOnly);
                } else if strip_prefix_ci(line, "Failure message:").is_some() {
                    found.add(locator, CulpritReason::Failed);
                }
            }
        }

        // Forge y NeoForge: «Mod ID: 'x', Requested by: 'y', ...».
        if let (Some(dep), Some(by)) = (
            between(line, "Mod ID: '", "'"),
            between(line, "Requested by: '", "'"),
        ) {
            let actual = between(line, "Actual version: '", "'").unwrap_or("");
            let reason = if actual.eq_ignore_ascii_case("[MISSING]") {
                CulpritReason::MissingDependency {
                    dependency: dep.to_string(),
                }
            } else {
                let range = between(line, "Expected range: '", "'").unwrap_or("");
                CulpritReason::WrongVersion {
                    needs: format!("{dep} {range}").trim().to_string(),
                }
            };
            found.add(Locator::ModId(by.to_string()), reason);
            continue;
        }

        // Fabric: «Mod 'Iris' (iris) 1.6.11 requires ... of fabric-api, which is missing!».
        if let Some(id) = fabric_mod_id(line) {
            if let Some(what) = between(line, " requires ", ", which is missing") {
                found.add(
                    Locator::ModId(id.to_string()),
                    CulpritReason::MissingDependency {
                        dependency: dependency_name(what),
                    },
                );
                continue;
            }
            if let Some(what) = between(line, " requires ", ", but only the wrong version") {
                found.add(
                    Locator::ModId(id.to_string()),
                    CulpritReason::WrongVersion {
                        needs: what.trim().to_string(),
                    },
                );
                continue;
            }
            if let Some(rest) = line.split_once(" is incompatible with ").map(|(_, r)| r) {
                let other = rest.split(", but").next().unwrap_or(rest);
                found.add(
                    Locator::ModId(id.to_string()),
                    CulpritReason::Incompatible {
                        other: dependency_name(other),
                    },
                );
                continue;
            }
        }

        // NeoForge: «Mod X requires y 1.0 or above» y «Currently, y is not installed».
        if let Some(rest) = line
            .trim_start_matches("- ")
            .strip_prefix("Mod ")
            .filter(|r| !r.starts_with('\''))
        {
            if let Some((who, what)) = rest.split_once(" requires ") {
                let dep = what.split_whitespace().next().unwrap_or("").to_string();
                let next = lines.get(i + 1).map(|l| l.trim()).unwrap_or("");
                if !dep.is_empty() && next.starts_with("Currently,") {
                    let reason = if next.contains("not installed") {
                        CulpritReason::MissingDependency { dependency: dep }
                    } else {
                        CulpritReason::WrongVersion {
                            needs: what.trim().to_string(),
                        }
                    };
                    found.add(Locator::Name(who.trim().to_string()), reason);
                    continue;
                }
            }
        }

        // «Create (create) has failed to load correctly».
        if let Some((before, _)) = line.split_once(" has failed to load correctly") {
            if let Some(id) = last_parenthesized(before) {
                found.add(Locator::ModId(id.to_string()), failure(lines, i, 4));
                continue;
            }
        }

        // Forge: «Failed to create mod instance. ModID: oculus, class ...».
        if line.contains("mod instance") {
            if let Some(id) = between(line, "ModID: ", ",") {
                found.add(Locator::ModId(id.trim().to_string()), failure(lines, i, 8));
                continue;
            }
        }

        // Fabric: «Could not execute entrypoint stage 'main' due to errors,
        // provided by 'betterhud' at 'com.example.Hud'!».
        if line.contains("Could not execute entrypoint") {
            if let Some(id) = between(line, "provided by '", "'") {
                found.add(Locator::ModId(id.to_string()), failure(lines, i, 6));
            }
        }

        // Una clase del juego (cliente) cargada en el servidor: el primer mod
        // de su traza es el que la pidió.
        if is_client_marker(line) && section.is_none() {
            if let Some(locator) = suspects(stack_after(lines, i)).into_iter().next() {
                found.add(locator, CulpritReason::ClientOnly);
            }
            continue;
        }

        // Mod de otro loader: «File x.jar is a Fabric mod and cannot be loaded».
        for loader in ["Fabric", "Forge", "NeoForge", "Quilt"] {
            if line.contains(&format!("is a {loader} mod")) {
                if let Some(jar) = jar_tokens(line).into_iter().next() {
                    found.add(
                        Locator::File(jar),
                        CulpritReason::WrongLoader {
                            loader: loader.to_string(),
                        },
                    );
                }
            }
        }

        // Mods repetidos: «Found duplicate mods: ... from mod files: a.jar, b.jar».
        if line.to_ascii_lowercase().contains("duplicate mod") {
            for l in lines.iter().skip(i).take(8) {
                for jar in jar_tokens(l).into_iter().skip(1) {
                    found.add(Locator::File(jar), CulpritReason::Duplicate);
                }
            }
            continue;
        }

        // Java: «com/x/Foo has been compiled by a more recent version of the Java Runtime
        // (class file version 65.0)».
        if let Some(rest) = line.split("UnsupportedClassVersionError: ").nth(1) {
            if let Some(class) = rest.split_whitespace().next() {
                let java = between(rest, "class file version ", ")")
                    .and_then(|v| v.split('.').next()?.parse::<u32>().ok())
                    .filter(|v| *v > 44)
                    .map(|v| format!("Java {}", v - 44))
                    .unwrap_or_else(|| "Java".into());
                found.add(
                    Locator::Class(class.replace('.', "/")),
                    CulpritReason::WrongVersion { needs: java },
                );
            }
            continue;
        }

        // Mixins que fallan: «Mixin apply for mod x failed», «... from mod x»,
        // «Mixin [x.mixins.json:Foo] ... FAILED during APPLY».
        if let Some(id) = between(line, "Mixin apply for mod ", " failed") {
            found.add(Locator::ModId(id.to_string()), CulpritReason::Failed);
            continue;
        }
        let mixin_error = line.contains("FAILED during")
            || line.contains("Critical injection failure")
            || line.contains("MixinApplyError")
            || line.contains("InvalidInjectionException")
            || line.contains("MixinTransformerError");
        if mixin_error {
            if let Some(id) = between(line, "from mod ", " ")
                .or_else(|| line.split("from mod ").nth(1))
                .map(|s| s.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_'))
                .filter(|s| !s.is_empty())
            {
                found.add(Locator::ModId(id.to_string()), CulpritReason::Failed);
            } else if let Some(config) = mixin_config(line) {
                found.add(Locator::Mixin(config), CulpritReason::Failed);
            }
            continue;
        }

        // Informe de Forge/NeoForge: «Suspected Mods: Create (create), Version: 0.5.1.f».
        if let Some(rest) = strip_prefix_ci(line, "Suspected Mod")
            .map(|r| r.trim_start_matches('s').trim_start_matches(':').trim())
        {
            if rest.eq_ignore_ascii_case("NONE") {
                continue;
            }
            let candidates = std::iter::once(rest).chain(
                lines
                    .iter()
                    .skip(i + 1)
                    .take(12)
                    .map(|l| l.trim())
                    .take_while(|l| !l.is_empty()),
            );
            for l in candidates {
                if let Some(id) = l.split(", Version").next().and_then(last_parenthesized) {
                    found.add(Locator::ModId(id.to_string()), CulpritReason::Suspected);
                }
            }
        }
    }
}

/// El mod falló al cargar; si las líneas siguientes hablan de clases del
/// juego, es que solo funciona en el cliente.
fn failure(lines: &[&str], i: usize, window: usize) -> CulpritReason {
    let client = lines
        .iter()
        .skip(i + 1)
        .take(window)
        .any(|l| is_client_marker(l));
    if client {
        CulpritReason::ClientOnly
    } else {
        CulpritReason::Failed
    }
}

/// «-- MOD create --» (Forge) o «-- Mod loading issue for: create --» (NeoForge).
fn mod_section(line: &str) -> Option<&str> {
    let inner = line.strip_prefix("-- ")?.strip_suffix(" --")?;
    inner
        .strip_prefix("MOD ")
        .or_else(|| inner.strip_prefix("Mod loading issue for: "))
        .map(str::trim)
        .filter(|id| !id.is_empty() && !id.contains(' '))
}

/// Un mod de cliente: el servidor intentó cargar una clase del juego.
fn is_client_marker(line: &str) -> bool {
    line.contains("invalid dist DEDICATED_SERVER")
        || line.contains("in environment type SERVER")
        || line.contains("net/minecraft/client/")
        || (line.contains("NoClassDefFoundError") && line.contains("net.minecraft.client."))
}

/// «Mod 'Name' (id) ...» de Fabric → `id`.
fn fabric_mod_id(line: &str) -> Option<&str> {
    let rest = &line[line.find("Mod '")?..];
    between(rest, "' (", ")").filter(|id| !id.is_empty() && !id.contains(' '))
}

/// El id de un mod en un texto de Fabric: lo último entre paréntesis o, si
/// no hay, la última palabra («any version of fabric-api»).
fn dependency_name(text: &str) -> String {
    let text = text.trim().trim_end_matches(['!', '.']);
    if let Some(id) = last_parenthesized(text) {
        return id.to_string();
    }
    text.split_whitespace()
        .last()
        .unwrap_or(text)
        .trim_matches(|c| c == '\'' || c == '"')
        .to_string()
}

/// Lo que va entre los últimos paréntesis: «Create (create)» → `create`.
fn last_parenthesized(text: &str) -> Option<&str> {
    let text = text.trim_end();
    let open = text.rfind('(')?;
    let close = open + text[open..].find(')')?;
    let inner = text[open + 1..close].trim();
    (!inner.is_empty() && !inner.contains(' ')).then_some(inner)
}

/// Nombres de archivo `.jar` de una línea.
fn jar_tokens(line: &str) -> Vec<String> {
    line.split(|c: char| c.is_whitespace() || matches!(c, ',' | '[' | ']' | '\'' | '"' | '(' | ')'))
        .filter(|t| t.to_ascii_lowercase().ends_with(".jar"))
        .map(|t| file_of_path(t).to_string())
        .filter(|t| t.len() > 4)
        .collect()
}

/// «[lithium.mixins.json:ai.FooMixin]» → `lithium.mixins.json`.
fn mixin_config(line: &str) -> Option<String> {
    line.split(|c: char| c.is_whitespace() || matches!(c, '[' | ']' | ':' | '\'' | '"' | ','))
        .find(|t| t.ends_with(".json") && t.contains("mixin"))
        .map(str::to_string)
}

fn file_of_path(path: &str) -> &str {
    path.trim()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .trim()
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = s.find(start)? + start.len();
    let len = s[from..].find(end)?;
    Some(&s[from..from + len])
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

/// Si una línea de la consola (sin hora ni hilo) anuncia que el servidor no
/// va a poder arrancar.
pub fn stops_the_start(msg: &str) -> bool {
    msg.contains("Failed to start the minecraft server") || msg.contains("Failed to load mods")
}

/// Mensajes con los que Minecraft y los loaders anuncian el error que para
/// el servidor.
const FATAL_MARKERS: &[&str] = &[
    "Failed to start the minecraft server",
    "Encountered an unexpected exception",
    "Exception in server tick loop",
    "Failed to load mods",
    "Mod resolution encountered",
    "Uncaught exception in thread",
    "Description:",
];

/// Las líneas del error principal: desde el primer aviso de error grave (o
/// la última excepción si no hay aviso).
fn main_error<'a>(lines: &'a [&'a str]) -> &'a [&'a str] {
    let start = lines
        .iter()
        .position(|l| FATAL_MARKERS.iter().any(|m| l.contains(m)))
        .or_else(|| lines.iter().rposition(|l| is_exception_line(l)));
    match start {
        Some(i) => &lines[i..lines.len().min(i + 400)],
        None => &[],
    }
}

/// La traza que sigue a la línea `i`.
fn stack_after<'a>(lines: &'a [&'a str], i: usize) -> &'a [&'a str] {
    &lines[i..lines.len().min(i + 80)]
}

/// El error en una línea: la causa más profunda de la primera excepción
/// del error principal.
fn headline(lines: &[&str]) -> Option<String> {
    let main = main_error(lines);
    let first = main.iter().position(|l| is_exception_line(l))?;
    let mut best = main[first].trim();
    for l in &main[first + 1..] {
        let t = l.trim();
        if let Some(cause) = t.strip_prefix("Caused by: ") {
            best = cause;
        } else if !(t.starts_with("at ") || t.starts_with("...") || t.starts_with("Suppressed")) {
            // Las líneas de un mensaje de varias líneas (Fabric) cuentan; una
            // línea del log con hora corta la traza.
            if t.starts_with('[') || t.is_empty() {
                break;
            }
        }
    }
    let mut text: String = best.chars().take(300).collect();
    if text.len() < best.len() {
        text.push('…');
    }
    Some(text)
}

/// «java.lang.RuntimeException: ...», «Caused by: x.y.FooError».
fn is_exception_line(line: &str) -> bool {
    let t = line.trim();
    let t = t.strip_prefix("Caused by: ").unwrap_or(t);
    let t = t
        .split_once("Exception in thread ")
        .map_or(t, |(_, r)| r.split_once("\" ").map_or(r, |(_, r)| r));
    let name = t.split([':', ' ']).next().unwrap_or("");
    name.contains('.')
        && !name.contains('/')
        && (name.ends_with("Exception") || name.ends_with("Error") || name.ends_with("Throwable"))
        && name.split('.').all(|p| {
            p.chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        })
}

/// Paquetes que no son de ningún mod (el juego, Java, los loaders y sus
/// bibliotecas): sus líneas en una traza no señalan a nadie.
const FRAMEWORK: &[&str] = &[
    "java.",
    "javax.",
    "jdk.",
    "sun.",
    "com.sun.",
    "net.minecraft.",
    "com.mojang.",
    "net.fabricmc.",
    "org.quiltmc.",
    "org.spongepowered.",
    "com.llamalad7.mixinextras.",
    "cpw.mods.",
    "net.minecraftforge.",
    "net.neoforged.",
    "org.apache.",
    "com.google.",
    "io.netty.",
    "it.unimi.",
    "org.slf4j.",
    "org.objectweb.",
    "com.electronwill.",
    "joptsimple.",
    "org.lwjgl.",
    "kotlin.",
    "kotlinx.",
    "scala.",
    "org.jetbrains.",
    "oshi.",
    "org.openjdk.",
];

/// Mods que aparecen en las primeras líneas de una traza.
fn suspects(lines: &[&str]) -> Vec<Locator> {
    let mut out: Vec<Locator> = Vec::new();
    for line in lines {
        let Some(frame) = parse_frame(line) else {
            continue;
        };
        let candidate = if let Some(id) = frame.mixin_mod {
            Some(Locator::ModId(id.to_string()))
        } else if FRAMEWORK.iter().any(|p| frame.class.starts_with(p)) {
            None
        } else if let Some(jar) = frame.jar {
            Some(Locator::File(jar.to_string()))
        } else if let Some(module) = frame.module {
            Some(Locator::ModId(module.to_string()))
        } else {
            Some(Locator::Class(frame.class.replace('.', "/")))
        };
        if let Some(c) = candidate {
            if !out.contains(&c) {
                out.push(c);
            }
        }
        if out.len() >= MAX_SUSPECTS {
            break;
        }
    }
    out
}

struct Frame<'a> {
    class: &'a str,
    /// Módulo con que Forge y NeoForge cargan cada mod ("create@0.5.1.f" → `create`).
    module: Option<&'a str>,
    /// Jar del mod en las trazas de Forge («~[create-1.20.1.jar%23200!/:?]»).
    jar: Option<&'a str>,
    /// Id del mod en los métodos que inyecta un mixin («handler$zfe000$lithium$tick»).
    mixin_mod: Option<&'a str>,
}

fn parse_frame(line: &str) -> Option<Frame<'_>> {
    let rest = line.trim_start().strip_prefix("at ")?;
    let (call, tail) = rest.split_once('(')?;
    let (prefix, qualified) = match call.rfind('/') {
        Some(i) => (Some(&call[..i]), &call[i + 1..]),
        None => (None, call),
    };
    let (class, method) = qualified.rsplit_once('.')?;
    let module = prefix
        .and_then(|p| p.rsplit('/').next())
        .filter(|m| m.contains('@'))
        .and_then(|m| m.split('@').next())
        .filter(|m| !m.is_empty() && !m.contains('.'));
    let jar = tail.split_once('[').and_then(|(_, t)| {
        let end = t.to_ascii_lowercase().find(".jar")? + 4;
        Some(&t[..end]).filter(|j| !j.starts_with('?'))
    });
    let parts: Vec<&str> = method.split('$').collect();
    let mixin_mod = (parts.len() >= 4
        && parts[1].len() >= 6
        && parts[1].chars().all(|c| c.is_ascii_alphanumeric()))
    .then(|| parts[2])
    .filter(|id| !id.is_empty());
    Some(Frame {
        class,
        module,
        jar,
        mixin_mod,
    })
}

/// Busca el archivo de cada mod señalado y junta lo de cada archivo.
fn resolve(
    items: Vec<(Locator, CulpritReason)>,
    mods: &[ModInfo],
    mods_dir: &Path,
) -> Vec<Culprit> {
    let by_id: HashMap<String, &ModInfo> = mods
        .iter()
        .filter_map(|m| Some((m.mod_id.as_deref()?.to_lowercase(), m)))
        .collect();
    let by_name: HashMap<String, &ModInfo> = mods
        .iter()
        .filter_map(|m| Some((dedupe_key(m.name.as_deref()?), m)))
        .collect();
    let by_file = |name: &str| {
        let name = name.replace("%20", " ").replace("%2B", "+");
        mods.iter().find(|m| {
            let base = m
                .file_name
                .strip_suffix(".disabled")
                .unwrap_or(&m.file_name);
            base.eq_ignore_ascii_case(&name)
        })
    };
    let wanted: Vec<String> = items
        .iter()
        .filter_map(|(l, _)| match l {
            Locator::Class(c) => Some(format!("{c}.class")),
            Locator::Mixin(m) => Some(m.clone()),
            _ => None,
        })
        .collect();
    let in_jars = if wanted.is_empty() {
        HashMap::new()
    } else {
        find_in_jars(mods_dir, mods, &wanted)
    };

    let mut out: Vec<Culprit> = Vec::new();
    for (locator, reason) in items {
        let found = match &locator {
            Locator::ModId(id) => by_id.get(&id.to_lowercase()).copied(),
            Locator::Name(name) => by_id
                .get(&name.to_lowercase())
                .or_else(|| by_name.get(&dedupe_key(name)))
                .copied(),
            Locator::File(file) => by_file(file),
            Locator::Class(c) => in_jars.get(&format!("{c}.class")).and_then(|f| by_file(f)),
            Locator::Mixin(m) => in_jars.get(m).and_then(|f| by_file(f)),
        };
        let Some(m) = found else {
            continue;
        };
        match out.iter_mut().find(|c| c.file_name == m.file_name) {
            Some(existing) => {
                if reason.rank() < existing.reason.rank() {
                    existing.reason = reason;
                }
            }
            None => out.push(Culprit {
                file_name: m.file_name.clone(),
                mod_id: m.mod_id.clone(),
                name: m.name.clone(),
                reason,
                in_log: true,
            }),
        }
    }
    sort_culprits(&mut out);
    out
}

fn sort_culprits(list: &mut [Culprit]) {
    list.sort_by(|a, b| {
        b.in_log
            .cmp(&a.in_log)
            .then(a.reason.rank().cmp(&b.reason.rank()))
            .then_with(|| {
                let name = |c: &Culprit| c.name.clone().unwrap_or_else(|| c.file_name.clone());
                name(a).to_lowercase().cmp(&name(b).to_lowercase())
            })
    });
}

/// Qué jar de `mods/` contiene cada una de estas entradas (clases o
/// configuraciones de mixins).
fn find_in_jars(mods_dir: &Path, mods: &[ModInfo], wanted: &[String]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for m in mods {
        let Ok(file) = std::fs::File::open(mods_dir.join(&m.file_name)) else {
            continue;
        };
        let Ok(zip) = zip::ZipArchive::new(file) else {
            continue;
        };
        let names: HashSet<&str> = zip.file_names().collect();
        for entry in wanted {
            if !out.contains_key(entry) && names.contains(entry.as_str()) {
                out.insert(entry.clone(), m.file_name.clone());
            }
        }
        if out.len() == wanted.len() {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn info(file: &str, id: &str, name: &str) -> ModInfo {
        ModInfo {
            file_name: file.into(),
            enabled: true,
            size: 0,
            sha1: format!("sha1-{id}"),
            mod_id: Some(id.into()),
            name: Some(name.into()),
            version: None,
            from_pack: false,
            client_only: false,
        }
    }

    fn run(console: &str, report: Option<&str>, mods: &[ModInfo], dir: &Path) -> CrashReport {
        let console: Vec<String> = console.lines().map(str::to_string).collect();
        diagnose(&CrashInput {
            console: &console,
            report: report.map(|r| (r, "crash-reports/crash.txt")),
            mods,
            server_dir: dir,
            during_start: true,
        })
    }

    fn files(report: &CrashReport) -> Vec<(&str, &CulpritReason)> {
        report
            .culprits
            .iter()
            .map(|c| (c.file_name.as_str(), &c.reason))
            .collect()
    }

    #[test]
    fn forge_missing_and_wrong_versions() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [
            info("mowzies.jar", "mowziesmobs", "Mowzie's Mobs"),
            info("oldmod.jar", "oldmod", "Old Mod"),
            info("create.jar", "create", "Create"),
        ];
        let log = "\
[12:00:01] [main/ERROR] [net.minecraftforge.fml.loading.ModSorter/LOADING]: Missing or unsupported mandatory dependencies:
\tMod ID: 'geckolib', Requested by: 'mowziesmobs', Expected range: '[4.2,)', Actual version: '[MISSING]'
\tMod ID: 'minecraft', Requested by: 'oldmod', Expected range: '[1.19.2,1.19.3)', Actual version: '1.20.1'
[12:00:02] [main/FATAL] [net.minecraftforge.server.loading.ServerModLoader/]: Failed to load mods
[12:00:02] [main/ERROR] [net.minecraft.server.Main/FATAL]: Failed to start the minecraft server
net.minecraftforge.fml.LoadingFailedException: Loading errors encountered: [
\tMowzie's Mobs (mowziesmobs) is missing a dependency
]
\tat net.minecraftforge.fml.ModLoader.waitForTransition(ModLoader.java:243) ~[fmlcore-1.20.1-47.2.0.jar%23185!/:?] {}
";
        let report = run(log, None, &mods, dir.path());
        assert_eq!(
            files(&report),
            vec![
                (
                    "mowzies.jar",
                    &CulpritReason::MissingDependency {
                        dependency: "geckolib".into()
                    }
                ),
                (
                    "oldmod.jar",
                    &CulpritReason::WrongVersion {
                        needs: "minecraft [1.19.2,1.19.3)".into()
                    }
                ),
            ]
        );
        assert_eq!(report.missing, vec!["geckolib".to_string()]);
        assert!(report
            .headline
            .as_deref()
            .unwrap()
            .starts_with("net.minecraftforge.fml.LoadingFailedException"));
    }

    #[test]
    fn forge_client_only_mod_in_crash_report() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [
            info("oculus-mc1.20.1-1.6.9.jar", "oculus", "Oculus"),
            info("create.jar", "create", "Create"),
        ];
        let crash = "\
---- Minecraft Crash Report ----
Description: Mod loading error has occurred

java.lang.Exception: Mod Loading has failed
\tat net.minecraftforge.logging.CrashReportExtender.dumpModLoadingCrashReport(CrashReportExtender.java:60) ~[forge-1.20.1-47.2.0-universal.jar%23189!/:?] {re:classloading}

-- MOD oculus --
Details:
\tCaused by 0: java.lang.RuntimeException: Attempted to load class net/minecraft/client/gui/screens/Screen for invalid dist DEDICATED_SERVER
\t\tat TRANSFORMER/oculus@1.6.9/net.coderbot.iris.Iris.<init>(Iris.java:107) ~[oculus-mc1.20.1-1.6.9.jar%23187!/:?] {re:classloading}
\tMod File: /home/volo/.local/share/micra/servers/pack/mods/oculus-mc1.20.1-1.6.9.jar
\tFailure message: Oculus (oculus) has failed to load correctly
\tMod Version: 1.6.9
";
        let report = run("", Some(crash), &mods, dir.path());
        assert_eq!(
            files(&report),
            vec![("oculus-mc1.20.1-1.6.9.jar", &CulpritReason::ClientOnly)]
        );
        assert_eq!(
            report.report_file.as_deref(),
            Some("crash-reports/crash.txt")
        );
    }

    #[test]
    fn neoforge_client_class_from_stack() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [info("zoomify.jar", "zoomify", "Zoomify")];
        let log = "\
[main/ERROR] [net.neoforged.fml.javafmlmod.FMLModContainer/LOADING]: Exception caught during firing event: Attempted to load class net/minecraft/client/KeyMapping for invalid dist DEDICATED_SERVER
java.lang.RuntimeException: Attempted to load class net/minecraft/client/KeyMapping for invalid dist DEDICATED_SERVER
\tat MC-BOOTSTRAP/cpw.mods.modlauncher@11.0.4/cpw.mods.modlauncher.TransformingClassLoader.maybeTransformClassBytes(TransformingClassLoader.java:50)
\tat TRANSFORMER/zoomify@2.14.2/dev.isxander.zoomify.Zoomify.<clinit>(Zoomify.kt:30)
";
        let report = run(log, None, &mods, dir.path());
        assert_eq!(
            files(&report),
            vec![("zoomify.jar", &CulpritReason::ClientOnly)]
        );
    }

    #[test]
    fn neoforge_loading_issues() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [
            info(
                "backpacks.jar",
                "sophisticatedbackpacks",
                "Sophisticated Backpacks",
            ),
            info(
                "storage.jar",
                "sophisticatedstorage",
                "Sophisticated Storage",
            ),
        ];
        let log = "\
[main/ERROR] [net.neoforged.neoforge.server.loading.ServerModLoader/]: Failed to load mods
net.neoforged.fml.ModLoadingException: Loading errors encountered:
\t- Mod Sophisticated Backpacks requires sophisticatedcore 1.2.0 or above
Currently, sophisticatedcore is not installed
\t- Mod sophisticatedstorage requires neoforge 21.1.80 or above
Currently, neoforge is 21.1.1
";
        let report = run(log, None, &mods, dir.path());
        assert_eq!(
            files(&report),
            vec![
                (
                    "backpacks.jar",
                    &CulpritReason::MissingDependency {
                        dependency: "sophisticatedcore".into()
                    }
                ),
                (
                    "storage.jar",
                    &CulpritReason::WrongVersion {
                        needs: "neoforge 21.1.80 or above".into()
                    }
                ),
            ]
        );
        assert_eq!(report.missing, vec!["sophisticatedcore".to_string()]);
    }

    #[test]
    fn fabric_resolution_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [
            info("iris.jar", "iris", "Iris"),
            info("sodium.jar", "sodium", "Sodium"),
            info("indium.jar", "indium", "Indium"),
        ];
        let log = "\
[main/ERROR]: Incompatible mods found!
net.fabricmc.loader.impl.FormattedException: Mod resolution encountered an incompatible mod set!
A potential solution has been determined, this may resolve your problem:
\t - Install fabric-api, any version.
More details:
\t - Mod 'Iris' (iris) 1.6.11 requires any version of fabric-api, which is missing!
\t - Mod 'Sodium' (sodium) 0.5.3 requires version 1.20.1 of 'Minecraft' (minecraft), but only the wrong version is present: 1.20.4!
\t - Mod 'Indium' (indium) 1.0.27 is incompatible with any version of 'Sodium' (sodium), but only the wrong version is present: 0.5.3!
\tat net.fabricmc.loader.impl.FormattedException.ofLocalized(FormattedException.java:51)
";
        let report = run(log, None, &mods, dir.path());
        assert_eq!(
            files(&report),
            vec![
                (
                    "iris.jar",
                    &CulpritReason::MissingDependency {
                        dependency: "fabric-api".into()
                    }
                ),
                (
                    "sodium.jar",
                    &CulpritReason::WrongVersion {
                        needs: "version 1.20.1 of 'Minecraft' (minecraft)".into()
                    }
                ),
                (
                    "indium.jar",
                    &CulpritReason::Incompatible {
                        other: "sodium".into()
                    }
                ),
            ]
        );
        assert_eq!(report.missing, vec!["fabric-api".to_string()]);
        assert_eq!(
            report.headline.as_deref(),
            Some("net.fabricmc.loader.impl.FormattedException: Mod resolution encountered an incompatible mod set!")
        );
    }

    #[test]
    fn fabric_client_class_found_in_jar() {
        let dir = tempfile::tempdir().unwrap();
        let mods_dir = dir.path().join(MODS_DIR);
        std::fs::create_dir_all(&mods_dir).unwrap();
        let file = std::fs::File::create(mods_dir.join("betterhud.jar")).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(
            "com/example/hud/Hud.class",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"\xca\xfe\xba\xbe").unwrap();
        zip.finish().unwrap();
        let mods = [
            info("betterhud.jar", "betterhud", "Better HUD"),
            info("lithium.jar", "lithium", "Lithium"),
        ];
        let log = "\
[main/ERROR]: Failed to start the minecraft server
java.lang.RuntimeException: Could not execute entrypoint stage 'main' due to errors, provided by 'betterhud'!
Caused by: java.lang.RuntimeException: Cannot load class net.minecraft.class_310 in environment type SERVER
\tat net.fabricmc.loader.impl.launch.knot.KnotClassDelegate.loadClass(KnotClassDelegate.java:235)
\tat com.example.hud.Hud.onInitialize(Hud.java:12)
\tat net.minecraft.class_3218.handler$zfe000$lithium$tick(class_3218.java:5000)
";
        let report = run(log, None, &mods, dir.path());
        assert_eq!(
            files(&report),
            vec![
                ("betterhud.jar", &CulpritReason::ClientOnly),
                ("lithium.jar", &CulpritReason::Suspected),
            ]
        );
        assert_eq!(
            report.headline.as_deref(),
            Some("java.lang.RuntimeException: Cannot load class net.minecraft.class_310 in environment type SERVER")
        );
    }

    #[test]
    fn duplicates_other_loaders_and_suspects() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [
            info("jei-15.2.jar", "jei", "JEI"),
            info("jei-15.3.jar", "jei", "JEI"),
            info("sodium-fabric.jar", "sodium", "Sodium"),
            info("create.jar", "create", "Create"),
        ];
        let log = "\
[main/ERROR]: Found duplicate mods:
\tMod ID: 'jei' from mod files: jei-15.2.jar, jei-15.3.jar
[main/ERROR]: File sodium-fabric.jar is a Fabric mod and cannot be loaded
";
        let crash = "\
Description: Exception in server tick loop

java.lang.NullPointerException: Cannot invoke \"Object.hashCode()\" because \"key\" is null
\tat TRANSFORMER/create@0.5.1.f/com.simibubi.create.content.Foo.tick(Foo.java:10) ~[create.jar%23200!/:0.5.1.f] {re:classloading}

Suspected Mods: NONE
";
        let report = run(log, Some(crash), &mods, dir.path());
        assert_eq!(
            files(&report),
            vec![
                (
                    "sodium-fabric.jar",
                    &CulpritReason::WrongLoader {
                        loader: "Fabric".into()
                    }
                ),
                ("jei-15.3.jar", &CulpritReason::Duplicate),
                ("create.jar", &CulpritReason::Suspected),
            ]
        );
    }

    #[test]
    fn client_only_hints_after_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let mut zoom = info("zoom.jar", "zoom", "Zoom");
        zoom.client_only = true;
        let mods = [
            info("create.jar", "create", "Create"),
            zoom,
            info("menus.jar", "menus", "Fancy Menus"),
        ];
        let mut report = run(
            "Suspected Mods:\n\tCreate (create), Version: 0.5.1.f\n",
            None,
            &mods,
            dir.path(),
        );
        add_client_only(
            &mut report,
            &mods,
            &HashSet::from(["sha1-menus".to_string()]),
        );
        let list: Vec<(&str, bool)> = report
            .culprits
            .iter()
            .map(|c| (c.file_name.as_str(), c.in_log))
            .collect();
        assert_eq!(
            list,
            vec![
                ("create.jar", true),
                ("menus.jar", false),
                ("zoom.jar", false)
            ]
        );
    }

    #[test]
    fn disabled_mods_are_not_blamed() {
        let dir = tempfile::tempdir().unwrap();
        let mut old = info("jei-old.jar.disabled", "jei", "JEI");
        old.enabled = false;
        let mods = [old, info("jei-new.jar", "jei", "JEI")];
        let report = run("\tMod ID: 'x', Requested by: 'jei', Expected range: '[1,)', Actual version: '[MISSING]'\n", None, &mods, dir.path());
        assert_eq!(report.culprits[0].file_name, "jei-new.jar");
        assert_eq!(report.culprits.len(), 1);
    }

    #[test]
    fn nothing_to_blame() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [info("create.jar", "create", "Create")];
        let report = run(
            "[main/INFO]: Starting minecraft server version 1.20.1\n[main/WARN]: Mixin config create.mixins.json does not specify \"minVersion\" property\n",
            None,
            &mods,
            dir.path(),
        );
        assert!(report.culprits.is_empty());
        assert!(report.headline.is_none());
    }

    #[test]
    fn frames() {
        let f = parse_frame("\tat TRANSFORMER/create@0.5.1.f/com.simibubi.create.Create.<init>(Create.java:10) ~[create-1.20.1-0.5.1.f.jar%23200!/:0.5.1.f] {re:classloading}").unwrap();
        assert_eq!(f.class, "com.simibubi.create.Create");
        assert_eq!(f.module, Some("create"));
        assert_eq!(f.jar, Some("create-1.20.1-0.5.1.f.jar"));
        let f = parse_frame("at java.base/java.lang.Thread.run(Thread.java:840) [?:?]").unwrap();
        assert_eq!(f.module, None);
        assert_eq!(f.jar, None);
        let f = parse_frame("at net.minecraft.class_1.handler$zfe000$lithium$tick(class_1.java:5)")
            .unwrap();
        assert_eq!(f.mixin_mod, Some("lithium"));
        assert!(parse_frame("[12:00:00] [main/INFO]: at noon").is_none());
    }
}
