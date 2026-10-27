//! Por qué no arrancó (o se cayó) un servidor con mods, y qué hacer.
//!
//! Solo se lee el error que paró el servidor. Los avisos y errores que no lo
//! pararon (mixins opcionales, compatibilidades con mods que no están, mods
//! de otro loader que se ignoran...) no cuentan: si contaran, se pedirían
//! quitar mods que no tienen nada que ver. De ese error se saca:
//!
//! 1. Lo que dice el loader de cada mod (Fabric, Forge o NeoForge): le falta
//!    otro, es para otra versión, está repetido, no cargó. Es seguro.
//! 2. La causa real de la excepción, la última «Caused by» de la cadena (no
//!    la primera línea), y el mod en cuyo código ocurrió: la primera línea de
//!    la traza que no es del juego, de Java ni de un loader. Según el tipo de
//!    error es seguro (una clase del cliente, una que falta) o probable.
//! 3. Lo demás que aparece (los «Suspected Mods» de Forge, un mod más abajo
//!    en la traza) solo es posible: la interfaz no lo marca para quitar.
//!
//! Si falta una dependencia, lo que se propone es instalarla, no quitar el
//! mod que la pide.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

use crate::providers::modrinth::ModrinthClient;
use crate::providers::search::{dedupe_key, Source};
use crate::server::config::Loader;
use crate::server::mods::{ModInfo, MODS_DIR};

const CRASH_DIR: &str = "crash-reports";
/// Lo que se lee como mucho de un informe de error.
const MAX_REPORT: u64 = 1024 * 1024;
/// Líneas de cada traza en las que se busca el mod.
const MAX_FRAMES: usize = 24;
/// Si el mod está entre las primeras líneas de la traza, el error ocurrió en
/// su código; más abajo, solo pasaba por ahí.
const LIKELY_DEPTH: usize = 4;
/// Como mucho, cuántos mods «probables» y «posibles» se muestran.
const MAX_LIKELY: usize = 2;
const MAX_POSSIBLE: usize = 3;
/// Largo máximo de cada línea que se muestra como prueba.
const MAX_EVIDENCE: usize = 240;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CrashReport {
    /// Mods que hay que quitar o que pueden ser la causa, del más seguro al
    /// menos.
    pub culprits: Vec<Culprit>,
    /// Mods que faltan: se arregla instalándolos (o activándolos).
    pub missing: Vec<MissingMod>,
    /// Lo que pasó cuando no es culpa de un mod concreto.
    pub cause: Option<CrashCause>,
    /// La causa del error en una línea, para quien quiera buscarla.
    pub headline: Option<String>,
    /// Informe de Minecraft (en `crash-reports/`), si escribió uno.
    pub report_file: Option<String>,
    /// Falló al arrancar (no llegó a abrirse).
    pub during_start: bool,
    /// El error es por cargar clases del juego (cliente) en el servidor.
    pub client_classes: bool,
    /// Ya se buscó dónde descargar los mods que faltan (los que siguen sin
    /// `found` no se encontraron para este servidor).
    pub missing_checked: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Culprit {
    /// Archivo en `mods/` (lo que se quita).
    pub file_name: String,
    pub mod_id: Option<String>,
    pub name: Option<String>,
    pub reason: CulpritReason,
    pub confidence: Confidence,
    /// Las líneas del error que lo señalan.
    pub evidence: Option<String>,
}

/// Lo seguro que es que un mod sea el problema (de más a menos).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum Confidence {
    /// Lo dice el loader o el error lo nombra: hay que quitarlo.
    Certain,
    /// El error ocurrió en su código.
    Likely,
    /// Aparece en el error, pero puede no ser la causa.
    Possible,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CulpritReason {
    /// Solo funciona en el juego (cliente), no en un servidor.
    ClientOnly,
    /// Es de otro loader (un mod de Fabric en NeoForge...).
    WrongLoader { loader: String },
    /// Es para otra versión de Minecraft.
    GameVersion { needs: Option<String> },
    /// Necesita otra versión del loader (Forge, NeoForge, Fabric Loader).
    LoaderVersion {
        loader: String,
        needs: Option<String>,
    },
    /// Necesita una versión de Java más nueva.
    JavaVersion { needs: String },
    /// Necesita otra versión de un mod que sí está instalado.
    DependencyVersion {
        dependency: String,
        needs: Option<String>,
    },
    /// Necesita algo que no está instalado y que Micra no sabe buscar (los
    /// mods que sí, van en `missing`).
    MissingDependency { dependency: String },
    /// No es compatible con otro mod instalado.
    Incompatible { other: String },
    /// Está instalado dos veces (se queda la versión más nueva).
    Duplicate,
    /// Falló al modificar el código del juego (mixin).
    MixinFailed,
    /// Dio un error al cargar.
    Failed,
    /// El error ocurrió en su código.
    ErrorInCode,
    /// Aparece en el error, pero puede no ser la causa.
    Suspected,
}

impl CulpritReason {
    /// Cuanto más bajo, más concreto es el motivo.
    fn rank(&self) -> u8 {
        match self {
            Self::ClientOnly => 0,
            Self::WrongLoader { .. } => 1,
            Self::GameVersion { .. } => 2,
            Self::LoaderVersion { .. } => 3,
            Self::JavaVersion { .. } => 4,
            Self::DependencyVersion { .. } => 5,
            Self::MissingDependency { .. } => 6,
            Self::Incompatible { .. } => 7,
            Self::Duplicate => 8,
            Self::MixinFailed => 9,
            Self::Failed => 10,
            Self::ErrorInCode => 11,
            Self::Suspected => 12,
        }
    }

    /// Motivos que solo dicen que algo falló, sin decir qué.
    fn is_generic(&self) -> bool {
        matches!(self, Self::Failed | Self::ErrorInCode | Self::Suspected)
    }
}

/// Un mod que falta.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MissingMod {
    /// Id que pide el error ("geckolib").
    pub id: String,
    /// Versión que piden, si el error la dice («≥ 4.2»).
    pub version: Option<String>,
    /// Mods instalados que lo necesitan (por si se prefiere quitarlos).
    pub required_by: Vec<ModRef>,
    /// Está en `mods/` pero desactivado: basta con activarlo.
    pub disabled_file: Option<String>,
    /// Dónde descargarlo para este servidor, si se encontró (lo rellena
    /// `missing::find` después del diagnóstico).
    pub found: Option<FoundMod>,
}

/// Un proyecto de Modrinth o CurseForge con versión para este servidor.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FoundMod {
    pub source: Source,
    pub project_id: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModRef {
    pub file_name: String,
    pub name: Option<String>,
}

/// Lo que pasó cuando no es culpa de un mod concreto.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CrashCause {
    /// Se quedó sin memoria.
    OutOfMemory,
    /// El puerto ya lo usa otro programa.
    PortInUse,
    /// Una vuelta del servidor tardó tanto que Minecraft lo dio por colgado.
    Watchdog,
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
    /// El loader del servidor: cambia qué mod es una biblioteca que falta.
    pub loader: &'a Loader,
}

/// Lee el error y dice qué mods quitar y cuáles faltan. Bloquea (abre los
/// jars): llamar desde `spawn_blocking`.
pub fn diagnose(input: &CrashInput) -> CrashReport {
    let console: Vec<String> = input.console.iter().map(|l| clean(l)).collect();
    let console: Vec<&str> = console.iter().map(String::as_str).collect();
    let report: Vec<String> = input
        .report
        .map(|(text, _)| text.lines().map(clean).collect())
        .unwrap_or_default();
    let report: Vec<&str> = report.iter().map(String::as_str).collect();
    let ctx = Ctx {
        loader: LoaderKind::of(input.loader),
    };

    let mut found = Findings::default();
    // El informe de Minecraft es solo del error: va primero.
    if !report.is_empty() {
        read_report(&report, &mut found, &ctx);
    }
    for range in fatal_events(&console) {
        read_block(&console[range], &mut found, &ctx);
    }
    finish(found, input)
}

/// Añade lo que se sabe de qué mods solo funcionan en el juego (por sus
/// metadatos o porque Modrinth lo dice, `sha1s`). Solo importa si el error
/// es de clases del juego: si ya se sabe qué mod lo causó, ahora se sabe
/// además por qué; si no, los mods de cliente pasan a ser los posibles
/// culpables. Si el error es de otra cosa no se añade nada: un mod de
/// cliente que el servidor ignora no hace daño.
pub fn add_client_only(report: &mut CrashReport, mods: &[ModInfo], sha1s: &HashSet<String>) {
    if !report.client_classes {
        return;
    }
    let attributed = report
        .culprits
        .iter()
        .any(|c| c.reason == CulpritReason::ClientOnly && c.confidence == Confidence::Certain);
    for m in mods {
        if !m.enabled || !(m.client_only || sha1s.contains(&m.sha1)) {
            continue;
        }
        match report
            .culprits
            .iter_mut()
            .find(|c| c.file_name == m.file_name)
        {
            // Lo señala el error y además solo funciona en el juego.
            Some(c) if c.evidence.is_some() => {
                c.reason = CulpritReason::ClientOnly;
                c.confidence = c.confidence.min(Confidence::Likely);
            }
            Some(_) => {}
            None if !attributed => report.culprits.push(Culprit {
                file_name: m.file_name.clone(),
                mod_id: m.mod_id.clone(),
                name: m.name.clone(),
                reason: CulpritReason::ClientOnly,
                confidence: Confidence::Possible,
                evidence: None,
            }),
            None => {}
        }
    }
    sort_culprits(&mut report.culprits);
}

/// Deja de pedir los mods que faltaban y ya están instalados y activos.
pub fn forget_installed(report: &mut CrashReport, mods: &[ModInfo]) {
    let ids: HashSet<String> = mods
        .iter()
        .filter(|m| m.enabled)
        .filter_map(|m| m.mod_id.as_deref())
        .map(|id| norm_id(alias(id)))
        .collect();
    report
        .missing
        .retain(|m| !ids.contains(&norm_id(alias(&m.id))));
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
        let mut raw = Vec::new();
        std::fs::File::open(&newest)
            .ok()?
            .take(MAX_REPORT)
            .read_to_end(&mut raw)
            .ok()?;
        let name = format!("{CRASH_DIR}/{}", newest.file_name()?.to_string_lossy());
        Some((String::from_utf8_lossy(&raw).into_owned(), name))
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

/// Si una línea de la consola (sin hora ni hilo) anuncia que el servidor no
/// va a poder arrancar.
pub fn stops_the_start(msg: &str) -> bool {
    msg.contains("Failed to start the minecraft server") || msg.contains("Failed to load mods")
}

// --- Lo que se va encontrando ------------------------------------------------

/// Qué tipo de loader es, para saber de quién es una clase que falta.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LoaderKind {
    Fabric,
    Forge,
    NeoForge,
    Other,
}

impl LoaderKind {
    fn of(loader: &Loader) -> Self {
        match loader {
            Loader::Fabric { .. } | Loader::Quilt { .. } => Self::Fabric,
            Loader::Forge { .. } => Self::Forge,
            Loader::NeoForge { .. } => Self::NeoForge,
            Loader::Vanilla => Self::Other,
        }
    }
}

struct Ctx {
    loader: LoaderKind,
}

/// Cómo se encuentra el archivo de un mod que nombra el error.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Locator {
    ModId(String),
    /// Nombre visible ("Sophisticated Backpacks") o id.
    Name(String),
    File(String),
    /// Clase de Java ("net/coderbot/iris/Iris"): se busca en los jars.
    Class(String),
    /// Configuración de mixins ("lithium.mixins.json"): se busca en los jars.
    Mixin(String),
    /// Una línea de una traza: su jar, su módulo (Forge) y su clase.
    Frame {
        jar: Option<String>,
        module: Option<String>,
        class: String,
    },
}

struct Finding {
    locator: Locator,
    reason: CulpritReason,
    confidence: Confidence,
    evidence: String,
}

/// Un mod que pide otro que no está.
struct Need {
    dependency: String,
    version: Option<String>,
    requester: Option<Locator>,
    evidence: String,
}

#[derive(Default)]
struct Findings {
    items: Vec<Finding>,
    needs: Vec<Need>,
    /// Grupos de archivos del mismo mod, con la línea que lo dice.
    duplicates: Vec<(Vec<String>, String)>,
    /// Bibliotecas (paquetes de Java) que faltan y no se sabe de qué mod son.
    packages: HashSet<String>,
    cause: Option<CrashCause>,
    client_classes: bool,
    /// La causa del error que señaló algo, y si no, la primera que se leyó.
    headline: Option<String>,
    fallback_headline: Option<String>,
}

impl Findings {
    fn add(
        &mut self,
        locator: Locator,
        reason: CulpritReason,
        confidence: Confidence,
        evidence: &str,
    ) {
        self.items.push(Finding {
            locator,
            reason,
            confidence,
            evidence: evidence.to_string(),
        });
    }

    fn count(&self) -> usize {
        self.items.len()
            + self.needs.len()
            + self.duplicates.len()
            + usize::from(self.cause.is_some())
    }
}

// --- Las partes del registro que importan -------------------------------------

/// Sin códigos de color de Minecraft («§e») ni de terminal.
fn clean(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '§' => {
                chars.next();
            }
            '\u{1b}' => {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    for c in chars.by_ref() {
                        if c.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
            }
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// «[12:00:00] [main/ERROR] [net.minecraftforge...]: mensaje».
fn is_header(line: &str) -> bool {
    line.starts_with('[')
        && [
            "/FATAL]", "/ERROR]", "/WARN]", "/INFO]", "/DEBUG]", "/TRACE]",
        ]
        .iter()
        .any(|level| head_of(line, 200).contains(level))
}

/// Mensajes con los que Minecraft, los loaders y Java anuncian el error que
/// para el servidor. Cualquier otro error del registro no lo paró.
const FATAL_MARKERS: &[&str] = &[
    // Minecraft
    "Failed to start the minecraft server",
    "Encountered an unexpected exception",
    "Exception in server tick loop",
    "Considering it to be crashed",
    "FAILED TO BIND TO PORT",
    // Forge y NeoForge
    "Failed to load mods",
    "Missing or unsupported mandatory dependencies",
    "Unsupported installed optional dependencies",
    "Loading errors encountered",
    "Exception caught during firing event",
    "Failed to create mod instance",
    "Found duplicate mods",
    "Duplicate mods found",
    // Fabric y Quilt
    "Incompatible mods found",
    "Mod resolution failed",
    // Java
    "Exception in thread \"main\"",
    "Uncaught exception in thread \"main\"",
    "java.lang.OutOfMemoryError",
];

/// Las entradas del registro que anuncian el error que paró el servidor,
/// cada una con las líneas que la siguen (traza, mensaje de varias líneas).
fn fatal_events(lines: &[&str]) -> Vec<std::ops::Range<usize>> {
    let mut starts: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| is_header(l) || l.starts_with("Exception in thread "))
        .map(|(i, _)| i)
        .collect();
    // Lo que va antes de la primera cabecera (salida de error de Java) es
    // otra entrada.
    if starts.first() != Some(&0) {
        starts.insert(0, 0);
    }
    let mut out = Vec::new();
    for (k, &start) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(lines.len());
        let block = &lines[start..end];
        let head = block.first().copied().unwrap_or("");
        let fatal = if is_header(head) {
            FATAL_MARKERS.iter().any(|m| head.contains(m))
        } else {
            block
                .iter()
                .take(3)
                .any(|l| FATAL_MARKERS.iter().any(|m| l.contains(m)))
        };
        if fatal {
            out.push(start..end);
        }
    }
    out
}

/// Una entrada del registro que anuncia el error.
fn read_block(lines: &[&str], found: &mut Findings, ctx: &Ctx) {
    if lines.iter().any(|l| l.contains("FAILED TO BIND TO PORT")) {
        found.cause.get_or_insert(CrashCause::PortInUse);
    }
    if lines
        .iter()
        .any(|l| l.contains("Considering it to be crashed"))
    {
        found.cause.get_or_insert(CrashCause::Watchdog);
    }
    loader_statements(lines, found, ctx, None);
    let chain = chain(lines);
    if !chain.is_empty() {
        analyse(&chain, found, ctx, None);
    }
}

/// El informe de `crash-reports/`: la excepción principal, las secciones de
/// cada mod que falló (Forge y NeoForge) y sus «Suspected Mods».
fn read_report(lines: &[&str], found: &mut Findings, ctx: &Ctx) {
    // La lista de mods y los datos del sistema no dicen nada del error.
    let end = lines
        .iter()
        .position(|l| l.trim_start().starts_with("-- System Details --"))
        .unwrap_or(lines.len());
    let lines = &lines[..end];
    let walkthrough = lines
        .iter()
        .position(|l| l.starts_with("A detailed walkthrough"))
        .unwrap_or(lines.len());
    if let Some(d) = lines.iter().position(|l| l.starts_with("Description:")) {
        if lines[d].contains("Watching Server") {
            found.cause.get_or_insert(CrashCause::Watchdog);
        }
        let main = &lines[d + 1..walkthrough.max(d + 1)];
        loader_statements(main, found, ctx, None);
        let chain = chain(main);
        if !chain.is_empty() {
            analyse(&chain, found, ctx, None);
        }
    }

    let mut i = 0;
    while i < lines.len() {
        let Some(id) = mod_section(lines[i].trim()) else {
            i += 1;
            continue;
        };
        let end = lines[i + 1..]
            .iter()
            .position(|l| l.trim_start().starts_with("-- "))
            .map_or(lines.len(), |p| i + 1 + p);
        read_mod_section(id, &lines[i + 1..end], found, ctx);
        i = end;
    }

    // Forge: «Suspected Mods: Create (create), Version: 0.5.1.f». Forge los
    // saca de toda la traza, así que solo son posibles.
    if let Some(s) = lines
        .iter()
        .position(|l| strip_prefix_ci(l.trim(), "Suspected Mod").is_some())
    {
        let first = lines[s]
            .trim()
            .split_once(':')
            .map_or("", |(_, rest)| rest.trim());
        let following = lines[s + 1..]
            .iter()
            .take(24)
            .take_while(|l| l.starts_with(['\t', ' ']))
            .map(|l| l.trim());
        let mut count = 0;
        for l in std::iter::once(first).chain(following) {
            if count == MAX_POSSIBLE {
                break;
            }
            if let Some(id) = l.split(", Version").next().and_then(last_parenthesized) {
                if l.contains(", Version") || l == first {
                    found.add(
                        Locator::ModId(id.to_string()),
                        CulpritReason::Suspected,
                        Confidence::Possible,
                        l,
                    );
                    count += 1;
                }
            }
        }
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

/// La sección del informe de un mod que no cargó: su archivo, el mensaje
/// del loader y las causas («Caused by 0: ...»).
fn read_mod_section(id: &str, lines: &[&str], found: &mut Findings, ctx: &Ctx) {
    let file = lines
        .iter()
        .find_map(|l| strip_prefix_ci(l.trim(), "Mod File:"))
        .map(file_of_path)
        .filter(|f| f.to_ascii_lowercase().ends_with(".jar"));
    let owner = file.map_or_else(
        || Locator::ModId(id.to_string()),
        |f| Locator::File(f.to_string()),
    );

    // «Failure message: ...» y las líneas que la siguen, más sangradas.
    let mut failure: Vec<&str> = Vec::new();
    if let Some(f) = lines
        .iter()
        .position(|l| strip_prefix_ci(l.trim(), "Failure message:").is_some())
    {
        let indent = indent_of(lines[f]);
        failure.push(
            strip_prefix_ci(lines[f].trim(), "Failure message:")
                .unwrap_or("")
                .trim(),
        );
        failure.extend(
            lines[f + 1..]
                .iter()
                .take_while(|l| indent_of(l) > indent && !l.trim().is_empty())
                .map(|l| l.trim()),
        );
    }
    // Si el loader dice qué le falta, no hace falta mirar más.
    let before = found.count();
    requirements(&failure, found, Some(&owner));
    if found.count() > before {
        return;
    }
    let evidence = failure.first().copied().unwrap_or("").to_string();
    let mut chain = section_chain(lines);
    if chain.is_empty() {
        chain = self::chain(&failure);
    }
    if chain.is_empty() {
        if !failure.is_empty() {
            found.add(owner, CulpritReason::Failed, Confidence::Certain, &evidence);
        }
        return;
    }
    analyse(&chain, found, ctx, Some((owner, evidence)));
}

fn indent_of(line: &str) -> usize {
    line.chars().take_while(|c| *c == '\t' || *c == ' ').count()
}

// --- Lo que dicen los loaders -------------------------------------------------

/// Lo que Forge, NeoForge y Fabric dicen de cada mod: dependencias,
/// versiones, repetidos, mods de otro loader... `owner` es el mod del que
/// habla la sección del informe, si se sabe.
fn loader_statements(lines: &[&str], found: &mut Findings, ctx: &Ctx, owner: Option<&Locator>) {
    requirements(lines, found, owner);
    // Las soluciones que propone Fabric van entre estas dos líneas.
    let mut in_solution = false;
    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        let item = line.trim_start_matches(['-', '*', ' ', '\t']);
        if line.contains("A potential solution has been determined") {
            in_solution = true;
            continue;
        }
        if line.starts_with("More details") {
            in_solution = false;
            continue;
        }
        if in_solution {
            // «Install fabric-api, any version.» / «Install fabric-api, version 0.92.0 or later.»
            if let Some(rest) = item.strip_prefix("Install ") {
                let rest = rest.trim_end_matches('.');
                let (dep, version) = rest.split_once(", ").unwrap_or((rest, ""));
                found.needs.push(Need {
                    dependency: dependency_name(dep),
                    version: version.strip_prefix("version ").map(str::to_string),
                    requester: None,
                    evidence: line.to_string(),
                });
            }
            // «Remove mod 'Indium' (indium) 1.0.27.»
            if item.starts_with("Remove mod '") {
                if let Some(id) = between(item, "' (", ")") {
                    found.add(
                        Locator::ModId(id.to_string()),
                        CulpritReason::Failed,
                        Confidence::Certain,
                        line,
                    );
                }
            }
            continue;
        }

        // Forge: «Mod ID: 'jei' from mod files: a.jar, b.jar».
        // NeoForge: «Mod jei is present in multiple files: a.jar, b.jar».
        if let Some((_, files)) = line
            .split_once(" from mod files: ")
            .or_else(|| line.split_once(" is present in multiple files: "))
        {
            let files: Vec<String> = files
                .split(',')
                .map(|f| file_of_path(f.trim_end_matches('.')).to_string())
                .filter(|f| f.to_ascii_lowercase().ends_with(".jar"))
                .collect();
            if files.len() > 1 {
                found.duplicates.push((files, line.to_string()));
            }
            continue;
        }

        // «Create (create) has failed to load correctly», con el error debajo.
        if let Some((before, _)) = line.split_once(" has failed to load correctly") {
            if let Some(id) = last_parenthesized(before) {
                let who = owner
                    .cloned()
                    .unwrap_or_else(|| Locator::ModId(id.to_string()));
                let detail: Vec<&str> = lines[i + 1..]
                    .iter()
                    .take(3)
                    .map(|l| l.trim())
                    .take_while(|l| !l.contains(" has failed to load correctly"))
                    .collect();
                let chain = chain(&detail);
                if chain.is_empty() {
                    found.add(who, CulpritReason::Failed, Confidence::Certain, line);
                } else {
                    analyse(&chain, found, ctx, Some((who, line.to_string())));
                }
            }
            continue;
        }

        // Forge: «Failed to create mod instance. ModID: oculus, class ...».
        if line.contains("Failed to create mod instance") {
            if let Some(id) = between(line, "ModID: ", ",") {
                found.add(
                    Locator::ModId(id.trim().to_string()),
                    CulpritReason::Failed,
                    Confidence::Certain,
                    line,
                );
            }
            continue;
        }

        // Mod de otro loader: «File x.jar is a Fabric mod and cannot be loaded».
        for (needle, loader) in [
            ("is a Fabric mod", "Fabric"),
            ("is a Quilt mod", "Quilt"),
            ("is a NeoForge mod", "NeoForge"),
            ("is a Minecraft Forge mod", "Forge"),
            ("is a Forge mod", "Forge"),
            ("is a LiteLoader mod", "LiteLoader"),
        ] {
            if line.contains(needle) {
                if let Some(jar) = jar_tokens(line).into_iter().next() {
                    found.add(
                        Locator::File(jar),
                        CulpritReason::WrongLoader {
                            loader: loader.to_string(),
                        },
                        Confidence::Certain,
                        line,
                    );
                }
                break;
            }
        }

        // Archivos rotos: «The Mod File .../x.jar has mods that were not found»
        // o «File x.jar is not a valid mod file».
        let broken = between(line, "The Mod File ", " has mods that were not found")
            .or_else(|| between(line, "File ", " is not a valid mod file"));
        if let Some(path) = broken {
            let file = file_of_path(path);
            if file.to_ascii_lowercase().ends_with(".jar") {
                found.add(
                    Locator::File(file.to_string()),
                    CulpritReason::Failed,
                    Confidence::Certain,
                    line,
                );
            }
        }

        // Forge: «Mod File x.jar needs language provider javafml:47 or above to load».
        if let Some((before, after)) = line.split_once(" needs language provider ") {
            if let Some(jar) = jar_tokens(before).into_iter().next() {
                found.add(
                    Locator::File(jar),
                    CulpritReason::LoaderVersion {
                        loader: "Forge".into(),
                        needs: Some(after.trim_end_matches(" to load").trim().to_string()),
                    },
                    Confidence::Certain,
                    line,
                );
            }
        }
    }
}

/// Dependencias que faltan o son de otra versión, con el formato de cada
/// loader. `owner`, si se sabe, es el mod que las pide.
fn requirements(lines: &[&str], found: &mut Findings, owner: Option<&Locator>) {
    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        let item = line.trim_start_matches(['-', '*', ' ', '\t']);

        // Forge: «Mod ID: 'geckolib', Requested by: 'mowziesmobs',
        // Expected range: '[4.2,)', Actual version: '[MISSING]'».
        if let (Some(dep), Some(by)) = (
            between(line, "Mod ID: '", "'"),
            between(line, "Requested by: '", "'"),
        ) {
            let range = between(line, "Expected range: '", "'").and_then(pretty_range);
            let actual = between(line, "Actual version: '", "'").unwrap_or("");
            let who = owner
                .cloned()
                .unwrap_or_else(|| Locator::ModId(by.to_string()));
            requirement(
                found,
                who,
                dep,
                range,
                actual.eq_ignore_ascii_case("[MISSING]"),
                line,
            );
            continue;
        }

        // Fabric: «Mod 'Iris' (iris) 1.6.11 requires any version of fabric-api,
        // which is missing!» o «..., but only the wrong version is present: 1.20.4!».
        if item.starts_with("Mod '") {
            let Some(id) = between(item, "' (", ")") else {
                continue;
            };
            let who = owner
                .cloned()
                .unwrap_or_else(|| Locator::ModId(id.to_string()));
            if let Some((_, what)) = item.split_once(" requires ") {
                if let Some((what, _)) = what.split_once(", which is missing") {
                    let (dep, needs) = fabric_dependency(what);
                    requirement(found, who, &dep, needs, true, line);
                } else if let Some((what, _)) = what.split_once(", but only the wrong version") {
                    let (dep, needs) = fabric_dependency(what);
                    requirement(found, who, &dep, needs, false, line);
                }
            } else if let Some((_, what)) = item.split_once(" is incompatible with ") {
                let what = what.split(", but").next().unwrap_or(what);
                let (other, _) = fabric_dependency(what);
                found.add(
                    who,
                    CulpritReason::Incompatible { other },
                    Confidence::Certain,
                    line,
                );
            }
            continue;
        }

        // NeoForge y Forge: «Mod Sophisticated Backpacks requires
        // sophisticatedcore 1.2.0 or above» y debajo «Currently,
        // sophisticatedcore is not installed» (o «... is 1.0.0»).
        if let Some(rest) = item.strip_prefix("Mod ") {
            let (rest, next) = match rest.split_once(" Currently,") {
                Some((rest, current)) => (rest.trim(), format!("Currently,{current}")),
                None => (
                    rest,
                    lines
                        .get(i + 1)
                        .map(|l| l.trim().trim_start_matches(['-', ' ']).to_string())
                        .unwrap_or_default(),
                ),
            };
            if !next.starts_with("Currently,") {
                continue;
            }
            if let Some((who, what)) = rest.split_once(" requires ") {
                let what = what.trim();
                let (dep, range) = what.split_once(' ').unwrap_or((what, ""));
                let needs = Some(range.trim())
                    .filter(|r| !r.is_empty() && *r != "any")
                    .map(str::to_string);
                let who = owner
                    .cloned()
                    .unwrap_or_else(|| Locator::Name(who.trim().to_string()));
                let evidence = format!("{line}\n{next}");
                requirement(
                    found,
                    who,
                    dep,
                    needs,
                    next.contains("not installed"),
                    &evidence,
                );
            } else if let Some((who, what)) = rest.split_once(" is incompatible with ") {
                let other = what.split_whitespace().next().unwrap_or("").to_string();
                let who = owner
                    .cloned()
                    .unwrap_or_else(|| Locator::Name(who.trim().to_string()));
                found.add(
                    who,
                    CulpritReason::Incompatible { other },
                    Confidence::Certain,
                    &format!("{line}\n{next}"),
                );
            }
        }
    }
}

/// Un mod que necesita `dep`: si es el juego, el loader o Java, el mod es de
/// otra versión; si es otro mod, falta (se instala) o es de otra versión.
fn requirement(
    found: &mut Findings,
    who: Locator,
    dep: &str,
    needs: Option<String>,
    missing: bool,
    evidence: &str,
) {
    let needs = needs.map(|n| version_text(&n));
    let loader = |name: &str| CulpritReason::LoaderVersion {
        loader: name.to_string(),
        needs: needs.clone(),
    };
    let reason = match norm_id(dep).as_str() {
        "minecraft" => CulpritReason::GameVersion {
            needs: needs.clone(),
        },
        "forge" | "fml" | "javafml" => loader("Forge"),
        "neoforge" => loader("NeoForge"),
        "fabricloader" => loader("Fabric Loader"),
        "quiltloader" => loader("Quilt Loader"),
        "java" => CulpritReason::JavaVersion {
            needs: needs.clone().unwrap_or_default(),
        },
        _ if missing => {
            found.needs.push(Need {
                dependency: dep.to_string(),
                version: needs,
                requester: Some(who),
                evidence: evidence.to_string(),
            });
            if found.headline.is_none() {
                found.headline = Some(shorten(evidence.lines().next().unwrap_or(evidence)));
            }
            return;
        }
        _ => CulpritReason::DependencyVersion {
            dependency: dep.to_string(),
            needs,
        },
    };
    if found.headline.is_none() {
        found.headline = Some(shorten(evidence.lines().next().unwrap_or(evidence)));
    }
    found.add(who, reason, Confidence::Certain, evidence);
}

/// «any version of fabric-api» → (`fabric-api`, nada);
/// «version 4.2 or later of mod 'GeckoLib' (geckolib)» → (`geckolib`, «4.2 or later»).
fn fabric_dependency(what: &str) -> (String, Option<String>) {
    let what = what.trim().trim_end_matches(['!', '.']);
    let (version, target) = what.split_once(" of ").unwrap_or(("", what));
    let needs = version
        .trim()
        .strip_prefix("version ")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    (dependency_name(target), needs)
}

/// Las versiones que pide un mod, tal como las escribe el loader en inglés,
/// con símbolos para que se entiendan en cualquier idioma: «4.2 or above» →
/// «≥ 4.2», «0.5.8 or later» → «≥ 0.5.8», «below 2.0» → «< 2.0».
fn version_text(text: &str) -> String {
    let text = text.trim();
    let bound = |t: &str| -> Option<String> {
        let t = t.trim();
        for (suffix, symbol) in [
            (" or above", "≥"),
            (" or later", "≥"),
            (" or newer", "≥"),
            (" or below", "≤"),
            (" or earlier", "≤"),
            (" or older", "≤"),
        ] {
            if let Some(v) = t.strip_suffix(suffix) {
                return Some(format!("{symbol} {}", v.trim()));
            }
        }
        for (prefix, symbol) in [
            ("above ", ">"),
            ("later than ", ">"),
            ("newer than ", ">"),
            ("below ", "<"),
            ("earlier than ", "<"),
            ("older than ", "<"),
            ("before ", "<"),
        ] {
            if let Some(v) = t.strip_prefix(prefix) {
                return Some(format!("{symbol} {}", v.trim()));
            }
        }
        None
    };
    if let Some(range) = text.strip_prefix("between ") {
        let range = range.trim_end_matches(" (inclusive)");
        if let Some((low, high)) = range.split_once(" and ") {
            return format!("{} – {}", low.trim(), high.trim());
        }
    }
    if let Some((low, high)) = text.split_once(" and ") {
        if let (Some(low), Some(high)) = (bound(low), bound(high)) {
            return format!("{low}, {high}");
        }
    }
    bound(text).unwrap_or_else(|| text.to_string())
}

/// Rango de versiones de Forge sin corchetes: «[4.2,)» → «≥ 4.2»,
/// «[1.19.2,1.19.3)» → «≥ 1.19.2, < 1.19.3», «[1.20.1]» → «1.20.1».
fn pretty_range(range: &str) -> Option<String> {
    let range = range.trim();
    if range.is_empty() || range == "*" {
        return None;
    }
    let (open, close) = (range.chars().next()?, range.chars().last()?);
    if !matches!(open, '[' | '(') || !matches!(close, ']' | ')') || range.len() < 2 {
        return Some(range.to_string());
    }
    let inner = &range[1..range.len() - 1];
    let Some((low, high)) = inner.split_once(',') else {
        return Some(inner.trim().to_string());
    };
    let (low, high) = (low.trim(), high.trim());
    let mut parts = Vec::new();
    if !low.is_empty() {
        parts.push(format!("{} {low}", if open == '[' { "≥" } else { ">" }));
    }
    if !high.is_empty() {
        parts.push(format!("{} {high}", if close == ']' { "≤" } else { "<" }));
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

// --- La excepción -------------------------------------------------------------

/// Una excepción de la cadena, con su mensaje y su traza.
struct Throwable<'a> {
    /// «java.lang.RuntimeException: mensaje» (sin «Caused by: »).
    head: &'a str,
    /// Líneas del mensaje después de la primera.
    message: Vec<&'a str>,
    /// Líneas «at ...».
    frames: Vec<&'a str>,
}

impl<'a> Throwable<'a> {
    fn class(&self) -> &'a str {
        self.head.split([':', ' ']).next().unwrap_or("")
    }

    /// Lo que va tras «Clase: ».
    fn detail(&self) -> &'a str {
        self.head.split_once(": ").map_or("", |(_, d)| d.trim())
    }

    fn text(&self) -> impl Iterator<Item = &'a str> + '_ {
        std::iter::once(self.head).chain(self.message.iter().copied())
    }
}

/// «java.lang.RuntimeException: ...», «Caused by: x.y.FooError», «Exception in
/// thread "main" x.y.Z» → la excepción sin lo de delante.
fn exception_head(line: &str) -> Option<&str> {
    let mut t = line.trim();
    t = t.strip_prefix("Caused by: ").unwrap_or(t);
    if let Some((_, rest)) = t.split_once("Exception in thread \"") {
        t = rest.split_once("\" ").map_or(rest, |(_, r)| r);
    }
    let name = t.split([':', ' ']).next().unwrap_or("");
    let java_name = name.contains('.')
        && !name.contains('/')
        && name.split('.').all(|p| {
            p.chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        });
    // Con Fabric, las excepciones del propio juego tienen nombres ofuscados
    // («net.minecraft.class_148: Exception ticking world»).
    let obfuscated = name
        .strip_prefix("net.minecraft.class_")
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    let looks_like = java_name
        && (name.ends_with("Exception")
            || name.ends_with("Error")
            || name.ends_with("Throwable")
            || obfuscated);
    looks_like.then_some(t)
}

/// La primera excepción de estas líneas con toda su cadena de causas. Las
/// suprimidas («Suppressed: ») no cuentan.
fn chain<'a>(lines: &[&'a str]) -> Vec<Throwable<'a>> {
    let Some(first) = lines.iter().position(|l| exception_head(l).is_some()) else {
        return Vec::new();
    };
    let mut out = vec![Throwable {
        head: exception_head(lines[first]).unwrap_or(lines[first]),
        message: Vec::new(),
        frames: Vec::new(),
    }];
    let mut suppressed = false;
    for raw in &lines[first + 1..] {
        let line = raw.trim();
        // Las causas de la excepción principal van sin sangría.
        if let Some(cause) = raw.strip_prefix("Caused by: ") {
            suppressed = false;
            out.push(Throwable {
                head: cause.trim(),
                message: Vec::new(),
                frames: Vec::new(),
            });
            continue;
        }
        if suppressed {
            continue;
        }
        if line.starts_with("Suppressed: ") {
            suppressed = true;
            continue;
        }
        let Some(current) = out.last_mut() else {
            break;
        };
        if line.starts_with("at ") {
            current.frames.push(line);
        } else if line.starts_with("...") {
            // «... 12 more»: las mismas líneas que la excepción de fuera.
        } else if is_header(raw) {
            break;
        } else if current.frames.is_empty() {
            if !line.is_empty() && current.message.len() < 80 {
                current.message.push(line);
            }
        } else {
            break;
        }
    }
    out
}

/// Las causas de una sección del informe de Forge: «Caused by 0: ...» con
/// su traza debajo.
fn section_chain<'a>(lines: &[&'a str]) -> Vec<Throwable<'a>> {
    let mut out: Vec<Throwable<'a>> = Vec::new();
    let mut open = false;
    for raw in lines {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("Caused by ") {
            if let Some((index, head)) = rest.split_once(": ") {
                if index.chars().all(|c| c.is_ascii_digit()) {
                    out.push(Throwable {
                        head: head.trim(),
                        message: Vec::new(),
                        frames: Vec::new(),
                    });
                    open = true;
                    continue;
                }
            }
        }
        match out.last_mut() {
            Some(current) if open && line.starts_with("at ") => current.frames.push(line),
            Some(_) if open && line.starts_with("...") => {}
            _ => open = false,
        }
    }
    out
}

/// Qué tipo de error es, mirando toda la cadena.
enum Problem {
    OutOfMemory,
    PortInUse,
    Watchdog,
    /// Una clase compilada para un Java más nuevo.
    JavaTooOld {
        class: String,
        java: String,
    },
    Mixin {
        mod_id: Option<String>,
        config: Option<String>,
        conflict: Option<String>,
    },
    /// Se cargó una clase del juego (cliente) en el servidor.
    ClientClass,
    /// Falta una clase que no es del juego: falta una biblioteca.
    MissingClass {
        class: String,
    },
    /// Falta una clase o un método del juego: el mod es de otra versión.
    GameMember,
    /// Falta un método o campo de otro mod: ese mod es de otra versión.
    ModMember {
        owner: String,
    },
    /// Falló el código que prepara una clase (`<clinit>`).
    BrokenClass {
        class: String,
    },
    Other,
}

fn classify(chain: &[Throwable]) -> Problem {
    let texts: Vec<&str> = chain.iter().flat_map(|t| t.text()).collect();
    if chain
        .iter()
        .any(|t| t.class() == "java.lang.OutOfMemoryError")
    {
        return Problem::OutOfMemory;
    }
    if texts
        .iter()
        .any(|l| l.contains("java.net.BindException") || l.contains("Address already in use"))
    {
        return Problem::PortInUse;
    }
    if texts.iter().any(|l| {
        l.contains("ServerHangWatchdog")
            || l.contains("A single server tick took")
            || *l == "java.lang.Error: Watchdog"
    }) {
        return Problem::Watchdog;
    }
    for t in chain.iter().rev() {
        if t.class().ends_with("UnsupportedClassVersionError") {
            if let Some(problem) = java_too_old(t.detail()) {
                return problem;
            }
        }
    }
    if let Some(problem) = mixin_problem(&texts) {
        return problem;
    }
    if texts.iter().any(|l| is_client_marker(l)) {
        return Problem::ClientClass;
    }
    for t in chain.iter().rev() {
        let class = t.class();
        if class.ends_with("NoClassDefFoundError") || class.ends_with("ClassNotFoundException") {
            let detail = t.detail();
            if let Some(broken) = detail.strip_prefix("Could not initialize class ") {
                return Problem::BrokenClass {
                    class: broken.trim().replace('.', "/"),
                };
            }
            let missing = detail
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_end_matches([',', ';', ')'])
                .replace('/', ".");
            if missing.is_empty() {
                continue;
            }
            return if is_game_class(&missing) {
                Problem::GameMember
            } else {
                Problem::MissingClass { class: missing }
            };
        }
        let member_error = [
            "NoSuchMethodError",
            "NoSuchFieldError",
            "AbstractMethodError",
            "IncompatibleClassChangeError",
        ]
        .iter()
        .any(|e| class.ends_with(e));
        if member_error {
            return match member_owner(t.detail()) {
                Some(owner) if is_game_class(&owner) => Problem::GameMember,
                Some(owner) => Problem::ModMember { owner },
                None if is_obfuscated(t.detail()) => Problem::GameMember,
                None => Problem::Other,
            };
        }
    }
    Problem::Other
}

/// «com/x/Foo has been compiled by a more recent version of the Java Runtime
/// (class file version 65.0), ...» → la clase y Java 21.
fn java_too_old(detail: &str) -> Option<Problem> {
    let class = detail.split_whitespace().next()?.replace('.', "/");
    if class.starts_with("net/minecraft/") {
        return None;
    }
    let java = between(detail, "class file version ", ")")
        .and_then(|v| v.split('.').next()?.parse::<u32>().ok())
        .filter(|v| *v > 44)
        .map(|v| (v - 44).to_string())?;
    Some(Problem::JavaTooOld { class, java })
}

/// Errores de mixins: el mod y la configuración que fallaron y, si chocó
/// con otro, de quién es el otro.
fn mixin_problem(texts: &[&str]) -> Option<Problem> {
    const MARKERS: &[&str] = &[
        "MixinApplyError",
        "MixinPrepareError",
        "MixinTransformerError",
        "InvalidMixinException",
        "InvalidInjectionException",
        "InjectionError",
        "Critical injection failure",
        "FAILED during",
        "@Redirect conflict",
    ];
    if !texts.iter().any(|l| MARKERS.iter().any(|m| l.contains(m))) {
        return None;
    }
    // La causa más profunda es la más concreta.
    let mod_id = texts.iter().rev().find_map(|l| from_mod(l));
    let config = texts.iter().rev().find_map(|l| mixin_config(l));
    let conflict = texts.iter().find_map(|l| {
        l.split_once("already redirected by ")
            .and_then(|(_, rest)| from_mod(rest))
    });
    Some(Problem::Mixin {
        mod_id,
        config,
        conflict,
    })
}

/// «... from mod lithium -> ...» → `lithium`.
fn from_mod(text: &str) -> Option<String> {
    let rest = text.split("from mod ").nth(1)?;
    let id: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-'))
        .collect();
    let id = id.trim_end_matches('-');
    (!id.is_empty()).then(|| id.to_string())
}

/// «[lithium.mixins.json:ai.FooMixin]» → `lithium.mixins.json`.
fn mixin_config(line: &str) -> Option<String> {
    line.split(|c: char| c.is_whitespace() || matches!(c, '[' | ']' | ':' | '\'' | '"' | ','))
        .find(|t| t.ends_with(".json") && t.contains("mixin"))
        .map(str::to_string)
}

/// Un mod de cliente: el servidor intentó cargar una clase del juego.
fn is_client_marker(line: &str) -> bool {
    line.contains("invalid dist DEDICATED_SERVER")
        || line.contains("in environment type SERVER")
        || line.contains("net/minecraft/client/")
        || line.contains("com/mojang/blaze3d/")
        || ((line.contains("NoClassDefFoundError") || line.contains("ClassNotFoundException"))
            && (line.contains("net.minecraft.client.") || line.contains("com.mojang.blaze3d.")))
}

fn is_game_class(class: &str) -> bool {
    class.starts_with("net.minecraft.") || class.starts_with("com.mojang.")
}

/// Nombres ofuscados del juego: «field_1234» (Fabric), «f_12345_» (Forge).
fn is_obfuscated(detail: &str) -> bool {
    let name = detail.trim().trim_matches('\'');
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    ["field_", "method_", "class_"]
        .iter()
        .any(|p| name.strip_prefix(p).is_some_and(digits))
        || ["f_", "m_"].iter().any(|p| {
            name.strip_prefix(p)
                .and_then(|r| r.strip_suffix('_'))
                .is_some_and(digits)
        })
}

/// De quién es el método o campo que falta:
/// «'void net.minecraft.world.level.Level.foo(int)'» → `net.minecraft.world.level.Level`.
fn member_owner(detail: &str) -> Option<String> {
    for marker in [
        "of interface ",
        "of abstract class ",
        "of class ",
        "requested interface ",
        "Found interface ",
        "Found class ",
        "Class ",
    ] {
        if let Some(rest) = detail.split(marker).nth(1) {
            let owner = rest
                .split([' ', ',', '\''])
                .next()
                .unwrap_or("")
                .trim_end_matches('.');
            if owner.contains('.') {
                return Some(owner.replace('/', "."));
            }
        }
    }
    let token = detail
        .trim_matches('\'')
        .split_whitespace()
        .find(|t| t.contains('('))?;
    let (path, _) = token.split_once('(')?;
    let (owner, _) = path.rsplit_once('.')?;
    owner.contains(['.', '/']).then(|| owner.replace('/', "."))
}

/// El mod que nombra el propio error: Fabric («provided by 'x'»), Forge
/// («ModID: x»), «Create (create) has failed to load correctly».
fn named_mod(chain: &[Throwable]) -> Option<(Locator, String)> {
    chain.iter().flat_map(|t| t.text()).find_map(|l| {
        let id = between(l, "provided by '", "'")
            .or_else(|| between(l, "ModID: ", ","))
            .or_else(|| {
                l.split_once(" has failed to load correctly")
                    .and_then(|(before, _)| last_parenthesized(before))
            })?;
        Some((Locator::ModId(id.trim().to_string()), l.to_string()))
    })
}

/// El primer mod de la traza, empezando por la causa más profunda: el
/// localizador, en qué línea de su traza está y si es la de la causa.
struct FrameHit<'a> {
    locator: Locator,
    depth: usize,
    in_root: bool,
    line: &'a str,
}

fn frame_mod<'a>(chain: &[Throwable<'a>]) -> Option<FrameHit<'a>> {
    for (k, t) in chain.iter().enumerate().rev() {
        for (depth, line) in t.frames.iter().copied().take(MAX_FRAMES).enumerate() {
            let Some(frame) = parse_frame(line) else {
                continue;
            };
            if let Some(locator) = frame_locator(&frame) {
                return Some(FrameHit {
                    locator,
                    depth,
                    in_root: k + 1 == chain.len(),
                    line,
                });
            }
        }
    }
    None
}

/// Lo que se sabe de la cadena de excepciones de un error.
fn analyse(chain: &[Throwable], found: &mut Findings, ctx: &Ctx, owner: Option<(Locator, String)>) {
    let Some(root) = chain.last() else {
        return;
    };
    let before = found.count();
    // Los mensajes que reúnen los errores de varios mods (Forge, NeoForge,
    // Fabric) ya se leyeron línea a línea en `loader_statements`, y su traza
    // es la del loader que los junta, no la de ningún mod culpable.
    let chain: Vec<Throwable> = chain
        .iter()
        .map(|t| {
            let summary = is_summary(t.head);
            Throwable {
                head: t.head,
                message: if summary {
                    Vec::new()
                } else {
                    t.message.clone()
                },
                frames: if summary {
                    Vec::new()
                } else {
                    t.frames.clone()
                },
            }
        })
        .collect();
    let chain = chain.as_slice();
    let headline = shorten(root.head);
    if found.fallback_headline.is_none() {
        found.fallback_headline = Some(headline.clone());
    }
    let problem = classify(chain);
    let named = owner.or_else(|| named_mod(chain));
    let framed = frame_mod(chain);
    let cause_line = root.head;
    let proof = |extra: Option<&str>| match extra {
        Some(line) if line != cause_line => format!("{cause_line}\n{line}"),
        _ => cause_line.to_string(),
    };

    match problem {
        Problem::OutOfMemory => {
            found.cause = Some(CrashCause::OutOfMemory);
        }
        Problem::PortInUse => {
            found.cause = Some(CrashCause::PortInUse);
        }
        Problem::Watchdog => {
            found.cause = Some(CrashCause::Watchdog);
            if let Some(hit) = framed {
                found.add(
                    hit.locator,
                    CulpritReason::Suspected,
                    Confidence::Possible,
                    &proof(Some(hit.line)),
                );
            }
        }
        Problem::JavaTooOld { class, java } => {
            found.add(
                Locator::Class(class),
                CulpritReason::JavaVersion { needs: java },
                Confidence::Certain,
                cause_line,
            );
        }
        Problem::Mixin {
            mod_id,
            config,
            conflict,
        } => {
            let who = mod_id
                .map(Locator::ModId)
                .or(config.map(Locator::Mixin))
                .or(named.map(|(l, _)| l))
                .or(framed.map(|h| h.locator));
            if let Some(who) = who {
                let reason = match conflict {
                    Some(other) => CulpritReason::Incompatible { other },
                    None => CulpritReason::MixinFailed,
                };
                found.add(who, reason, Confidence::Certain, cause_line);
            }
        }
        Problem::ClientClass => {
            found.client_classes = true;
            // La línea de la traza que pidió la clase es la del mod culpable.
            match (framed, named) {
                (Some(hit), named) => {
                    let evidence = proof(Some(hit.line));
                    if let Some((other, line)) = named.filter(|(l, _)| *l != hit.locator) {
                        found.add(other, CulpritReason::Failed, Confidence::Possible, &line);
                    }
                    found.add(
                        hit.locator,
                        CulpritReason::ClientOnly,
                        Confidence::Certain,
                        &evidence,
                    );
                }
                (None, Some((who, line))) => {
                    found.add(
                        who,
                        CulpritReason::ClientOnly,
                        Confidence::Certain,
                        &proof(Some(&line)),
                    );
                }
                (None, None) => {}
            }
        }
        Problem::MissingClass { class } => {
            let (requester, line) = match (framed, named) {
                (Some(hit), _) => (Some(hit.locator), Some(hit.line.to_string())),
                (None, Some((who, line))) => (Some(who), Some(line)),
                (None, None) => (None, None),
            };
            let evidence = proof(line.as_deref());
            match missing_class(&class, ctx) {
                MissingKind::Mod(dep) => found.needs.push(Need {
                    dependency: dep.to_string(),
                    version: None,
                    requester,
                    evidence,
                }),
                MissingKind::Loader(loader, wrong) => {
                    if let Some(who) = requester {
                        let reason = if wrong {
                            CulpritReason::WrongLoader {
                                loader: loader.to_string(),
                            }
                        } else {
                            CulpritReason::LoaderVersion {
                                loader: loader.to_string(),
                                needs: None,
                            }
                        };
                        found.add(who, reason, Confidence::Certain, &evidence);
                    }
                }
                MissingKind::Unknown => {
                    if let Some(who) = requester {
                        let library = library_of(&class);
                        found.packages.insert(library.clone());
                        found.add(
                            who,
                            CulpritReason::MissingDependency {
                                dependency: library,
                            },
                            Confidence::Certain,
                            &evidence,
                        );
                    }
                }
            }
        }
        Problem::GameMember => {
            if let Some(hit) = &framed {
                let confidence = if hit.depth <= LIKELY_DEPTH {
                    Confidence::Certain
                } else {
                    Confidence::Likely
                };
                found.add(
                    hit.locator.clone(),
                    CulpritReason::GameVersion { needs: None },
                    confidence,
                    &proof(Some(hit.line)),
                );
            } else if let Some((who, line)) = named {
                found.add(
                    who,
                    CulpritReason::GameVersion { needs: None },
                    Confidence::Certain,
                    &proof(Some(&line)),
                );
            }
        }
        Problem::ModMember { owner } => {
            let requester = framed.map(|h| (h.locator, h.line.to_string())).or(named);
            if let Some((who, line)) = requester {
                let dependency = match missing_class(&owner, ctx) {
                    MissingKind::Mod(dep) => dep.to_string(),
                    _ => {
                        let library = library_of(&owner);
                        found.packages.insert(library.clone());
                        library
                    }
                };
                found.add(
                    who,
                    CulpritReason::DependencyVersion {
                        dependency,
                        needs: None,
                    },
                    Confidence::Certain,
                    &proof(Some(&line)),
                );
            }
        }
        Problem::BrokenClass { class } => {
            if let Some((who, line)) = named.clone() {
                found.add(
                    who,
                    CulpritReason::Failed,
                    Confidence::Certain,
                    &proof(Some(&line)),
                );
            }
            found.add(
                Locator::Class(class),
                CulpritReason::ErrorInCode,
                Confidence::Likely,
                cause_line,
            );
        }
        Problem::Other => {
            let named_locator = named.as_ref().map(|(l, _)| l.clone());
            if let Some((who, line)) = named {
                found.add(
                    who,
                    CulpritReason::Failed,
                    Confidence::Certain,
                    &proof(Some(&line)),
                );
            }
            if let Some(hit) = framed.filter(|h| Some(&h.locator) != named_locator.as_ref()) {
                let (reason, confidence) = if hit.in_root && hit.depth <= LIKELY_DEPTH {
                    (CulpritReason::ErrorInCode, Confidence::Likely)
                } else {
                    (CulpritReason::Suspected, Confidence::Possible)
                };
                found.add(hit.locator, reason, confidence, &proof(Some(hit.line)));
            }
        }
    }
    if found.count() > before && found.headline.is_none() {
        found.headline = Some(headline);
    }
}

/// Excepciones con las que el loader junta los errores de todos los mods.
fn is_summary(head: &str) -> bool {
    head.contains("Loading errors encountered")
        || head.contains("LoadingFailedException")
        || head.contains("ModLoadingException")
        || head.contains("FormattedException")
}

/// De quién es una clase que falta.
enum MissingKind {
    /// De este mod (se puede instalar).
    Mod(&'static str),
    /// Del loader: el mod es de otro (`true`) o de otra versión (`false`).
    Loader(&'static str, bool),
    Unknown,
}

/// Paquetes de las bibliotecas más usadas y el id de su mod.
const KNOWN_PACKAGES: &[(&str, &str)] = &[
    ("me.shedaniel.clothconfig2.", "cloth-config"),
    ("me.shedaniel.autoconfig.", "cloth-config"),
    ("me.shedaniel.cloth.", "cloth-config"),
    ("software.bernie.geckolib.", "geckolib"),
    ("mod.azure.azurelib.", "azurelib"),
    ("dev.architectury.", "architectury"),
    ("me.shedaniel.architectury.", "architectury"),
    ("dev.isxander.yacl3.", "yet_another_config_lib_v3"),
    ("net.blay09.mods.balm.", "balm"),
    ("vazkii.patchouli.", "patchouli"),
    ("top.theillusivec4.curios.", "curios"),
    ("dev.emi.trinkets.", "trinkets"),
    ("mezz.jei.", "jei"),
    ("dev.emi.emi.", "emi"),
    ("fuzs.puzzleslib.", "puzzleslib"),
    ("fuzs.forgeconfigapiport.", "forgeconfigapiport"),
    ("com.teamresourceful.resourcefullib.", "resourcefullib"),
    ("dev.ftb.mods.ftblibrary.", "ftblibrary"),
    ("team.creative.creativecore.", "creativecore"),
    ("net.darkhax.bookshelf.", "bookshelf"),
    ("com.mrcrayfish.framework.", "framework"),
    ("io.wispforest.owo.", "owo"),
    ("thedarkcolour.kotlinforforge.", "kotlinforforge"),
    ("net.mehvahdjukaar.moonlight.", "moonlight"),
    ("com.github.alexthe666.citadel.", "citadel"),
    ("org.violetmoon.zeta.", "zeta"),
    ("dev.latvian.mods.kubejs.", "kubejs"),
    ("dev.latvian.mods.rhino.", "rhino"),
    ("com.blamejared.crafttweaker.", "crafttweaker"),
    ("snownee.kiwi.", "kiwi"),
    ("terrablender.", "terrablender"),
    ("glitchcore.", "glitchcore"),
    ("dev.toma.configuration.", "configuration"),
    ("com.simibubi.create.", "create"),
    ("com.jozufozu.flywheel.", "flywheel"),
    ("dev.engine_room.flywheel.", "flywheel"),
];

fn missing_class(class: &str, ctx: &Ctx) -> MissingKind {
    let fabric = ctx.loader == LoaderKind::Fabric;
    if class.starts_with("kotlin.") || class.starts_with("kotlinx.") {
        return MissingKind::Mod(if fabric {
            "fabric-language-kotlin"
        } else {
            "kotlinforforge"
        });
    }
    if class.starts_with("net.fabricmc.fabric.") {
        return if fabric {
            MissingKind::Mod("fabric-api")
        } else {
            MissingKind::Loader("Fabric", true)
        };
    }
    let forge_config = [
        "net.minecraftforge.common.ForgeConfigSpec",
        "net.minecraftforge.fml.config.",
        "net.neoforged.neoforge.common.ModConfigSpec",
        "net.neoforged.fml.config.",
    ];
    if fabric && forge_config.iter().any(|p| class.starts_with(p)) {
        return MissingKind::Mod("forgeconfigapiport");
    }
    if class.starts_with("net.minecraftforge.") {
        return MissingKind::Loader("Forge", ctx.loader != LoaderKind::Forge);
    }
    if class.starts_with("net.neoforged.") {
        return MissingKind::Loader("NeoForge", ctx.loader != LoaderKind::NeoForge);
    }
    KNOWN_PACKAGES
        .iter()
        .find(|(prefix, _)| class.starts_with(prefix))
        .map_or(MissingKind::Unknown, |(_, id)| MissingKind::Mod(id))
}

/// «me.shedaniel.clothconfig2.api.ConfigBuilder» → «me.shedaniel.clothconfig2».
fn library_of(class: &str) -> String {
    let parts: Vec<&str> = class.split('.').collect();
    // Sin la clase (la última parte), como mucho tres niveles de paquete.
    let package = &parts[..parts.len().saturating_sub(1).max(1)];
    package[..package.len().min(3)].join(".")
}

// --- Las líneas de una traza ----------------------------------------------------

/// Paquetes que no son de ningún mod (el juego, Java, los loaders y sus
/// bibliotecas) o que solo reparten eventos a otros mods: sus líneas en una
/// traza no señalan a nadie.
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
    "org.joml.",
    "com.ibm.icu.",
    "org.jline.",
    "net.minecrell.",
    "com.typesafe.",
    "org.yaml.",
    "kotlin.",
    "kotlinx.",
    "scala.",
    "org.jetbrains.",
    "oshi.",
    "org.openjdk.",
    "thedarkcolour.kotlinforforge.",
    "dev.architectury.event.",
    "org.sinytra.",
];

struct Frame<'a> {
    class: &'a str,
    /// Módulo con que Forge y NeoForge cargan cada mod ("create@0.5.1.f" → `create`).
    module: Option<&'a str>,
    /// Jar del mod en las trazas («~[create-1.20.1.jar%23200!/:?]»).
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

fn frame_locator(frame: &Frame) -> Option<Locator> {
    if let Some(id) = frame.mixin_mod {
        return Some(Locator::ModId(id.to_string()));
    }
    if FRAMEWORK.iter().any(|p| frame.class.starts_with(p)) {
        return None;
    }
    Some(Locator::Frame {
        jar: frame.jar.map(str::to_string),
        module: frame.module.map(str::to_string),
        class: frame.class.replace('.', "/"),
    })
}

// --- De lo encontrado a los mods instalados -------------------------------------

/// Los mods activos, para encontrarlos por id, nombre o archivo.
struct ModIndex<'a> {
    mods: Vec<&'a ModInfo>,
    by_id: HashMap<String, &'a ModInfo>,
    by_name: HashMap<String, &'a ModInfo>,
}

impl<'a> ModIndex<'a> {
    fn new(mods: Vec<&'a ModInfo>) -> Self {
        let by_id = mods
            .iter()
            .filter_map(|m| Some((norm_id(m.mod_id.as_deref()?), *m)))
            .collect();
        let by_name = mods
            .iter()
            .filter_map(|m| Some((dedupe_key(m.name.as_deref()?), *m)))
            .collect();
        Self {
            mods,
            by_id,
            by_name,
        }
    }

    fn id(&self, id: &str) -> Option<&'a ModInfo> {
        self.by_id.get(&norm_id(alias(id))).copied()
    }

    fn name(&self, name: &str) -> Option<&'a ModInfo> {
        self.id(name)
            .or_else(|| self.by_name.get(&dedupe_key(name)).copied())
    }

    fn file(&self, name: &str) -> Option<&'a ModInfo> {
        let name = name.replace("%20", " ").replace("%2B", "+");
        self.mods.iter().copied().find(|m| {
            let base = m
                .file_name
                .strip_suffix(".disabled")
                .unwrap_or(&m.file_name);
            base.eq_ignore_ascii_case(&name)
        })
    }
}

/// Ids que un mod también responde por otro nombre.
fn alias(id: &str) -> &str {
    match id {
        "fabric" => "fabric-api",
        _ => id,
    }
}

/// Qué jar contiene cada clase, configuración de mixins o jar incluido, y
/// qué jar tiene cada paquete.
#[derive(Default)]
struct JarIndex {
    entries: HashMap<String, String>,
    packages: HashMap<String, String>,
}

impl JarIndex {
    fn build(
        mods_dir: &Path,
        mods: &[&ModInfo],
        entries: &HashSet<String>,
        packages: &HashSet<String>,
    ) -> Self {
        let mut out = Self::default();
        if entries.is_empty() && packages.is_empty() {
            return out;
        }
        let prefixes: Vec<(String, &String)> = packages
            .iter()
            .map(|p| (format!("{}/", p.replace('.', "/")), p))
            .collect();
        for m in mods {
            let Ok(file) = std::fs::File::open(mods_dir.join(&m.file_name)) else {
                continue;
            };
            let Ok(zip) = zip::ZipArchive::new(file) else {
                continue;
            };
            for name in zip.file_names() {
                if entries.contains(name) && !out.entries.contains_key(name) {
                    out.entries.insert(name.to_string(), m.file_name.clone());
                }
                for (prefix, package) in &prefixes {
                    if name.starts_with(prefix.as_str()) && !out.packages.contains_key(*package) {
                        out.packages.insert((*package).clone(), m.file_name.clone());
                    }
                }
            }
            if out.entries.len() == entries.len() && out.packages.len() == packages.len() {
                break;
            }
        }
        out
    }

    fn entry(&self, name: &str) -> Option<&str> {
        self.entries.get(name).map(String::as_str)
    }
}

/// Lo que hay que buscar dentro de los jars para este localizador.
fn wanted_entries(locator: &Locator, index: &ModIndex, out: &mut HashSet<String>) {
    match locator {
        Locator::Class(c) => {
            out.insert(format!("{c}.class"));
        }
        Locator::Mixin(m) => {
            out.insert(m.clone());
        }
        Locator::Frame { jar, module, class } => {
            let known = jar.as_deref().and_then(|j| index.file(j)).is_some()
                || module.as_deref().and_then(|m| index.id(m)).is_some();
            if !known {
                out.insert(format!("{class}.class"));
                if let Some(jar) = jar {
                    out.insert(format!("META-INF/jars/{jar}"));
                    out.insert(format!("META-INF/jarjar/{jar}"));
                }
            }
        }
        _ => {}
    }
}

fn resolve<'a>(locator: &Locator, index: &ModIndex<'a>, jars: &JarIndex) -> Option<&'a ModInfo> {
    let in_jar = |entry: &str| jars.entry(entry).and_then(|f| index.file(f));
    match locator {
        Locator::ModId(id) => index.id(id),
        Locator::Name(name) => index.name(name),
        Locator::File(file) => index.file(file),
        Locator::Class(c) => in_jar(&format!("{c}.class")),
        Locator::Mixin(m) => in_jar(m),
        Locator::Frame { jar, module, class } => jar
            .as_deref()
            .and_then(|j| index.file(j))
            .or_else(|| module.as_deref().and_then(|m| index.id(m)))
            .or_else(|| {
                let jar = jar.as_deref()?;
                in_jar(&format!("META-INF/jars/{jar}"))
                    .or_else(|| in_jar(&format!("META-INF/jarjar/{jar}")))
            })
            .or_else(|| in_jar(&format!("{class}.class"))),
    }
}

/// Junta lo encontrado en mods concretos: los que quitar y los que faltan.
fn finish(found: Findings, input: &CrashInput) -> CrashReport {
    let index = ModIndex::new(input.mods.iter().filter(|m| m.enabled).collect());
    let mut entries = HashSet::new();
    for item in &found.items {
        wanted_entries(&item.locator, &index, &mut entries);
    }
    for need in &found.needs {
        if let Some(locator) = &need.requester {
            wanted_entries(locator, &index, &mut entries);
        }
    }
    let jars = JarIndex::build(
        &input.server_dir.join(MODS_DIR),
        &index.mods,
        &entries,
        &found.packages,
    );

    let mut culprits: Vec<Culprit> = Vec::new();
    for item in found.items {
        let Some(m) = resolve(&item.locator, &index, &jars) else {
            continue;
        };
        // Una biblioteca que falta pero cuyo paquete tiene otro mod instalado:
        // lo que pasa es que ese mod es de otra versión.
        let owner_of = |package: &str| jars.packages.get(package).and_then(|f| index.file(f));
        let reason = match item.reason {
            CulpritReason::MissingDependency { dependency }
                if found.packages.contains(&dependency) =>
            {
                match owner_of(&dependency) {
                    Some(owner) if owner.file_name == m.file_name => CulpritReason::Failed,
                    Some(owner) => CulpritReason::DependencyVersion {
                        dependency: display_name(owner),
                        needs: None,
                    },
                    None => CulpritReason::MissingDependency { dependency },
                }
            }
            CulpritReason::DependencyVersion {
                dependency,
                needs: None,
            } if found.packages.contains(&dependency) => match owner_of(&dependency) {
                Some(owner) if owner.file_name == m.file_name => CulpritReason::Failed,
                Some(owner) => CulpritReason::DependencyVersion {
                    dependency: display_name(owner),
                    needs: None,
                },
                None => CulpritReason::DependencyVersion {
                    dependency,
                    needs: None,
                },
            },
            reason => reason,
        };
        push_culprit(&mut culprits, m, reason, item.confidence, &item.evidence);
    }

    let mut missing: Vec<MissingMod> = Vec::new();
    let mut requesters: HashSet<String> = HashSet::new();
    for need in found.needs {
        let requester = need
            .requester
            .as_ref()
            .and_then(|l| resolve(l, &index, &jars));
        // Si el que lo pide solo funciona en el juego, sobra él.
        if let Some(r) = requester.filter(|r| r.client_only) {
            push_culprit(
                &mut culprits,
                r,
                CulpritReason::ClientOnly,
                Confidence::Certain,
                &need.evidence,
            );
            continue;
        }
        // Ya está instalado: lo que falla es su versión.
        if let Some(installed) = index.id(&need.dependency) {
            if let Some(r) = requester.filter(|r| r.file_name != installed.file_name) {
                push_culprit(
                    &mut culprits,
                    r,
                    CulpritReason::DependencyVersion {
                        dependency: display_name(installed),
                        needs: need.version,
                    },
                    Confidence::Certain,
                    &need.evidence,
                );
            }
            continue;
        }
        let key = norm_id(alias(&need.dependency));
        let entry = match missing
            .iter_mut()
            .position(|m| norm_id(alias(&m.id)) == key)
        {
            Some(i) => &mut missing[i],
            None => {
                let disabled_file = input
                    .mods
                    .iter()
                    .find(|m| {
                        !m.enabled && m.mod_id.as_deref().is_some_and(|id| norm_id(id) == key)
                    })
                    .map(|m| m.file_name.clone());
                missing.push(MissingMod {
                    id: alias(&need.dependency).to_string(),
                    version: None,
                    required_by: Vec::new(),
                    disabled_file,
                    found: None,
                });
                missing.last_mut().expect("se acaba de añadir")
            }
        };
        if entry.version.is_none() {
            entry.version = need.version;
        }
        if let Some(r) = requester {
            requesters.insert(r.file_name.clone());
            if !entry.required_by.iter().any(|x| x.file_name == r.file_name) {
                entry.required_by.push(ModRef {
                    file_name: r.file_name.clone(),
                    name: r.name.clone(),
                });
            }
        }
    }
    // Si a un mod solo le pasa que le falta otro, se arregla instalando ese.
    culprits.retain(|c| !(requesters.contains(&c.file_name) && c.reason.is_generic()));

    // Mods repetidos: se queda el de versión más nueva.
    for (files, evidence) in &found.duplicates {
        let mut group: Vec<&ModInfo> = files.iter().filter_map(|f| index.file(f)).collect();
        group.dedup_by(|a, b| a.file_name == b.file_name);
        if group.len() < 2 {
            continue;
        }
        group.sort_by(|a, b| {
            version_cmp(a.version.as_deref(), b.version.as_deref())
                .then_with(|| b.file_name.cmp(&a.file_name))
        });
        let keep = group.pop().map(|m| m.file_name.clone());
        for m in group {
            if Some(&m.file_name) != keep.as_ref() {
                push_culprit(
                    &mut culprits,
                    m,
                    CulpritReason::Duplicate,
                    Confidence::Certain,
                    evidence,
                );
            }
        }
    }

    sort_culprits(&mut culprits);
    let mut likely = 0;
    let mut possible = 0;
    culprits.retain(|c| match c.confidence {
        Confidence::Certain => true,
        Confidence::Likely => {
            likely += 1;
            likely <= MAX_LIKELY
        }
        Confidence::Possible => {
            possible += 1;
            possible <= MAX_POSSIBLE
        }
    });
    missing.sort_by(|a, b| a.id.cmp(&b.id));

    CrashReport {
        culprits,
        missing,
        cause: found.cause,
        headline: found.headline.or(found.fallback_headline),
        report_file: input.report.map(|(_, name)| name.to_string()),
        during_start: input.during_start,
        client_classes: found.client_classes,
        missing_checked: false,
    }
}

fn display_name(m: &ModInfo) -> String {
    m.name
        .clone()
        .or_else(|| m.mod_id.clone())
        .unwrap_or_else(|| m.file_name.clone())
}

/// Añade un mod a la lista o, si ya está, se queda con lo más seguro y concreto.
fn push_culprit(
    list: &mut Vec<Culprit>,
    m: &ModInfo,
    reason: CulpritReason,
    confidence: Confidence,
    evidence: &str,
) {
    let evidence = Some(evidence_text(evidence)).filter(|e| !e.is_empty());
    match list.iter_mut().find(|c| c.file_name == m.file_name) {
        Some(existing) => {
            let better =
                (confidence, reason.rank()) < (existing.confidence, existing.reason.rank());
            if better {
                existing.confidence = confidence;
                existing.reason = reason;
                existing.evidence = evidence;
            }
        }
        None => list.push(Culprit {
            file_name: m.file_name.clone(),
            mod_id: m.mod_id.clone(),
            name: m.name.clone(),
            reason,
            confidence,
            evidence,
        }),
    }
}

fn sort_culprits(list: &mut [Culprit]) {
    list.sort_by(|a, b| {
        a.confidence
            .cmp(&b.confidence)
            .then(a.reason.rank().cmp(&b.reason.rank()))
            .then_with(|| {
                let name = |c: &Culprit| c.name.clone().unwrap_or_else(|| c.file_name.clone());
                name(a).to_lowercase().cmp(&name(b).to_lowercase())
            })
    });
}

// --- Utilidades ---------------------------------------------------------------

/// Un id sin mayúsculas, guiones ni guiones bajos: «cloth_config» y
/// «cloth-config» son el mismo mod.
fn norm_id(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Compara versiones por sus números: «15.10» es más nueva que «15.9».
fn version_cmp(a: Option<&str>, b: Option<&str>) -> Ordering {
    fn parts(v: &str) -> Vec<(u64, String)> {
        let mut out = Vec::new();
        let mut chars = v.chars().peekable();
        while let Some(&c) = chars.peek() {
            let mut token = String::new();
            if c.is_ascii_digit() {
                while let Some(&d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                    token.push(d);
                    chars.next();
                }
                out.push((token.parse().unwrap_or(u64::MAX), String::new()));
            } else if c.is_alphabetic() {
                while let Some(&d) = chars.peek().filter(|d| d.is_alphabetic()) {
                    token.extend(d.to_lowercase());
                    chars.next();
                }
                out.push((0, token));
            } else {
                chars.next();
            }
        }
        out
    }
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(a), Some(b)) => parts(a).cmp(&parts(b)),
    }
}

/// Las líneas que se muestran como prueba, cada una no muy larga.
fn evidence_text(text: &str) -> String {
    text.lines()
        .map(|l| shorten(l.trim()))
        .filter(|l| !l.is_empty())
        .take(3)
        .collect::<Vec<_>>()
        .join("\n")
}

fn shorten(text: &str) -> String {
    let mut out: String = text.chars().take(MAX_EVIDENCE).collect();
    if out.len() < text.len() {
        out.push('…');
    }
    out
}

/// El id de un mod en un texto de Fabric: lo último entre paréntesis o, si
/// no hay, la última palabra («fabric-api»).
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

/// Los primeros `max` bytes de una línea, sin cortar un carácter.
fn head_of(line: &str, max: usize) -> &str {
    let mut end = line.len().min(max);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    &line[..end]
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

    fn versioned(file: &str, id: &str, version: &str) -> ModInfo {
        ModInfo {
            version: Some(version.into()),
            ..info(file, id, id)
        }
    }

    fn forge() -> Loader {
        Loader::Forge {
            version: "47.3.0".into(),
        }
    }

    fn neoforge() -> Loader {
        Loader::NeoForge {
            version: "21.1.1".into(),
        }
    }

    fn fabric() -> Loader {
        Loader::Fabric {
            loader_version: "0.16.5".into(),
        }
    }

    fn run(
        loader: &Loader,
        console: &str,
        report: Option<&str>,
        mods: &[ModInfo],
        dir: &Path,
    ) -> CrashReport {
        let console: Vec<String> = console.lines().map(str::to_string).collect();
        diagnose(&CrashInput {
            console: &console,
            report: report.map(|r| (r, "crash-reports/crash.txt")),
            mods,
            server_dir: dir,
            during_start: true,
            loader,
        })
    }

    fn culprits(report: &CrashReport) -> Vec<(&str, &CulpritReason, Confidence)> {
        report
            .culprits
            .iter()
            .map(|c| (c.file_name.as_str(), &c.reason, c.confidence))
            .collect()
    }

    fn missing(report: &CrashReport) -> Vec<(&str, Vec<&str>)> {
        report
            .missing
            .iter()
            .map(|m| {
                (
                    m.id.as_str(),
                    m.required_by.iter().map(|r| r.file_name.as_str()).collect(),
                )
            })
            .collect()
    }

    fn jar(dir: &Path, name: &str, entries: &[&str]) {
        let mods_dir = dir.join(MODS_DIR);
        std::fs::create_dir_all(&mods_dir).unwrap();
        let mut zip = zip::ZipWriter::new(std::fs::File::create(mods_dir.join(name)).unwrap());
        for entry in entries {
            zip.start_file(*entry, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"\xca\xfe\xba\xbe").unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn forge_missing_dependency_is_installed_not_removed() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [
            info("mowziesmobs-1.6.3.jar", "mowziesmobs", "Mowzie's Mobs"),
            info("oldmod-1.19.2.jar", "oldmod", "Old Mod"),
            info("create-1.20.1-0.5.1.f.jar", "create", "Create"),
        ];
        let console = "\
[12:00:01] [main/INFO] [net.minecraftforge.fml.loading.moddiscovery.ModDiscoverer/SCAN]: Found mod file mowziesmobs-1.6.3.jar of type MOD
[12:00:01] [main/WARN] [mixin/]: Error loading class: net/minecraft/client/renderer/LevelRenderer (java.lang.ClassNotFoundException: net/minecraft/client/renderer/LevelRenderer)
[12:00:01] [main/ERROR] [net.minecraftforge.fml.loading.ModSorter/LOADING]: Missing or unsupported mandatory dependencies:
\tMod ID: 'geckolib', Requested by: 'mowziesmobs', Expected range: '[4.2,)', Actual version: '[MISSING]'
\tMod ID: 'minecraft', Requested by: 'oldmod', Expected range: '[1.19.2,1.19.3)', Actual version: '1.20.1'
";
        let crash = "\
---- Minecraft Crash Report ----
// Who set us up the TNT?

Time: 2026-10-27 12:00:03
Description: Mod loading error has occurred

java.lang.Exception: Mod Loading has failed
\tat net.minecraftforge.logging.CrashReportExtender.dumpModLoadingCrashReport(CrashReportExtender.java:60) ~[forge-1.20.1-47.3.0-universal.jar%23189!/:?] {re:classloading}

A detailed walkthrough of the error, its code path and all known details is as follows:
---------------------------------------------------------------------------------------

-- Head --
Thread: main
Suspected Mod: NONE
Stacktrace:
\tat net.minecraftforge.logging.CrashReportExtender.dumpModLoadingCrashReport(CrashReportExtender.java:60) ~[forge-1.20.1-47.3.0-universal.jar%23189!/:?] {re:classloading}

-- MOD mowziesmobs --
Details:
\tMod File: /srv/mods/mowziesmobs-1.6.3.jar
\tFailure message: Mod mowziesmobs requires geckolib 4.2 or above
\t\tCurrently, geckolib is not installed
\tMod Version: 1.6.3
\tException message: MISSING EXCEPTION MESSAGE
Stacktrace:
\tat net.minecraftforge.logging.CrashReportExtender.lambda$dumpModLoadingCrashReport$7(CrashReportExtender.java:63) ~[forge-1.20.1-47.3.0-universal.jar%23189!/:?] {re:classloading}

-- MOD oldmod --
Details:
\tMod File: /srv/mods/oldmod-1.19.2.jar
\tFailure message: Mod oldmod requires minecraft 1.19.2
\t\tCurrently, minecraft is 1.20.1
\tMod Version: 2.0

-- System Details --
Details:
\tMod List:
\t\tcreate-1.20.1-0.5.1.f.jar |Create |create |0.5.1.f |DONE |Manifest: NOSIGNATURE
\t\tmowziesmobs-1.6.3.jar |Mowzie's Mobs (mowziesmobs) |mowziesmobs |1.6.3 |COMMON_SET |Manifest: NOSIGNATURE
";
        let report = run(&forge(), console, Some(crash), &mods, dir.path());
        assert_eq!(
            culprits(&report),
            vec![(
                "oldmod-1.19.2.jar",
                &CulpritReason::GameVersion {
                    needs: Some("1.19.2".into())
                },
                Confidence::Certain
            )]
        );
        assert_eq!(
            missing(&report),
            vec![("geckolib", vec!["mowziesmobs-1.6.3.jar"])]
        );
        assert_eq!(report.missing[0].version.as_deref(), Some("≥ 4.2"));
        assert!(!report.client_classes);
        assert_eq!(
            report.headline.as_deref(),
            Some("Mod mowziesmobs requires geckolib 4.2 or above")
        );
        assert_eq!(
            report.report_file.as_deref(),
            Some("crash-reports/crash.txt")
        );
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

A detailed walkthrough of the error, its code path and all known details is as follows:

-- MOD oculus --
Details:
\tCaused by 0: java.lang.RuntimeException: Attempted to load class net/minecraft/client/gui/screens/Screen for invalid dist DEDICATED_SERVER
\t\tat net.minecraftforge.fml.loading.RuntimeDistCleaner.processClassWithFlags(RuntimeDistCleaner.java:57) ~[fmlloader-1.20.1-47.2.0.jar%23181!/:1.0] {}
\t\tat TRANSFORMER/oculus@1.6.9/net.coderbot.iris.Iris.<init>(Iris.java:107) ~[oculus-mc1.20.1-1.6.9.jar%23187!/:?] {re:classloading}
\tMod File: /home/volo/.local/share/micra/servers/pack/mods/oculus-mc1.20.1-1.6.9.jar
\tFailure message: Oculus (oculus) has failed to load correctly
\t\tjava.lang.RuntimeException: Attempted to load class net/minecraft/client/gui/screens/Screen for invalid dist DEDICATED_SERVER
\tMod Version: 1.6.9
";
        let report = run(&forge(), "", Some(crash), &mods, dir.path());
        assert_eq!(
            culprits(&report),
            vec![(
                "oculus-mc1.20.1-1.6.9.jar",
                &CulpritReason::ClientOnly,
                Confidence::Certain
            )]
        );
        assert!(report.client_classes);
        let evidence = report.culprits[0].evidence.as_deref().unwrap();
        assert!(
            evidence.contains("invalid dist DEDICATED_SERVER"),
            "{evidence}"
        );
        assert!(evidence.contains("net.coderbot.iris.Iris"), "{evidence}");
    }

    #[test]
    fn forge_loading_errors_listed_in_the_console() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [
            info("mowziesmobs.jar", "mowziesmobs", "Mowzie's Mobs"),
            info("oculus.jar", "oculus", "Oculus"),
            info("create.jar", "create", "Create"),
        ];
        let console = "\
[12:00:05] [main/FATAL] [net.minecraftforge.server.loading.ServerModLoader/]: Failed to load mods
net.minecraftforge.fml.LoadingFailedException: Loading errors encountered: [
\tMowzie's Mobs (mowziesmobs) has failed to load correctly
§7java.lang.NoClassDefFoundError: software/bernie/geckolib/core/animatable/GeoAnimatable
\tOculus (oculus) has failed to load correctly
§7java.lang.RuntimeException: Attempted to load class net/minecraft/client/gui/screens/Screen for invalid dist DEDICATED_SERVER
]
\tat net.minecraftforge.fml.ModLoader.waitForTransition(ModLoader.java:243) ~[fmlcore-1.20.1-47.3.0.jar%23185!/:?] {}
\tat TRANSFORMER/create@0.5.1.f/com.simibubi.create.Create.<init>(Create.java:10) ~[create.jar%23200!/:0.5.1.f] {re:classloading}
";
        let report = run(&forge(), console, None, &mods, dir.path());
        assert_eq!(
            culprits(&report),
            vec![(
                "oculus.jar",
                &CulpritReason::ClientOnly,
                Confidence::Certain
            )]
        );
        assert_eq!(
            missing(&report),
            vec![("geckolib", vec!["mowziesmobs.jar"])]
        );
    }

    #[test]
    fn neoforge_client_class_from_an_event() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [
            info("zoomify-2.14.2.jar", "zoomify", "Zoomify"),
            info("lithium.jar", "lithium", "Lithium"),
        ];
        let console = "\
[12:00:05] [main/ERROR] [net.neoforged.fml.javafmlmod.FMLModContainer/LOADING]: Exception caught during firing event: Attempted to load class net/minecraft/client/KeyMapping for invalid dist DEDICATED_SERVER
\tIndex: 1
\tListeners:
\t\t0: NORMAL
\t\t1: ASM: class dev.isxander.zoomify.Zoomify onInit
java.lang.RuntimeException: Attempted to load class net/minecraft/client/KeyMapping for invalid dist DEDICATED_SERVER
\tat MC-BOOTSTRAP/net.neoforged.fml.loading@4.0.24/net.neoforged.fml.loading.RuntimeDistCleaner.processClassWithFlags(RuntimeDistCleaner.java:57)
\tat MC-BOOTSTRAP/cpw.mods.modlauncher@11.0.4/cpw.mods.modlauncher.TransformingClassLoader.maybeTransformClassBytes(TransformingClassLoader.java:50)
\tat TRANSFORMER/zoomify@2.14.2/dev.isxander.zoomify.Zoomify.<clinit>(Zoomify.kt:30)
\tat TRANSFORMER/minecraft@1.21.1/net.minecraft.server.Main.handler$abc000$lithium$main(Main.java:10)
";
        let report = run(&neoforge(), console, None, &mods, dir.path());
        assert_eq!(
            culprits(&report),
            vec![(
                "zoomify-2.14.2.jar",
                &CulpritReason::ClientOnly,
                Confidence::Certain
            )]
        );
    }

    #[test]
    fn neoforge_loading_issues_with_colour_codes() {
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
        let console = "\
[12:00:01] [main/FATAL] [net.neoforged.neoforge.server.loading.ServerModLoader/]: Failed to load mods
net.neoforged.fml.ModLoadingException: Loading errors encountered:
\t- Mod §eSophisticated Backpacks§r requires §6sophisticatedcore§r §o1.2.0 or above§r
§7Currently, §6sophisticatedcore§r§7 is §cnot installed
\t- Mod §eSophisticated Storage§r requires §6neoforge§r §o21.1.80 or above§r
§7Currently, §6neoforge§r§7 is §c21.1.1
\tat MC-BOOTSTRAP/net.neoforged.fml.loading@4.0.24/net.neoforged.fml.ModLoader.waitForTransition(ModLoader.java:243)
";
        let report = run(&neoforge(), console, None, &mods, dir.path());
        assert_eq!(
            culprits(&report),
            vec![(
                "storage.jar",
                &CulpritReason::LoaderVersion {
                    loader: "NeoForge".into(),
                    needs: Some("≥ 21.1.80".into())
                },
                Confidence::Certain
            )]
        );
        assert_eq!(
            missing(&report),
            vec![("sophisticatedcore", vec!["backpacks.jar"])]
        );
        assert_eq!(report.missing[0].version.as_deref(), Some("≥ 1.2.0"));
    }

    #[test]
    fn fabric_resolution_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [
            info("iris.jar", "iris", "Iris"),
            info("sodium.jar", "sodium", "Sodium"),
            info("indium.jar", "indium", "Indium"),
            info("lithium.jar", "lithium", "Lithium"),
        ];
        let console = "\
[12:00:00] [main/ERROR]: Incompatible mods found!
net.fabricmc.loader.impl.FormattedException: Some of your mods are incompatible with the game or each other!
A potential solution has been determined, this may resolve your problem:
\t - Install fabric-api, any version.
\t - Remove mod 'Indium' (indium) 1.0.27+mc1.20.1.
More details:
\t - Mod 'Iris' (iris) 1.6.11 requires any version of fabric-api, which is missing!
\t - Mod 'Sodium' (sodium) 0.5.3 requires version 1.20.1 of 'Minecraft' (minecraft), but only the wrong version is present: 1.20.4!
\t - Mod 'Indium' (indium) 1.0.27 is incompatible with any version of mod 'Sodium' (sodium), but a matching version is present: 0.5.3!
\t - Mod 'Lithium' (lithium) 0.12.0 recommends any version of mod 'Sodium Extra' (sodium-extra), which is missing!
\tat net.fabricmc.loader.impl.FormattedException.ofLocalized(FormattedException.java:51) ~[fabric-loader-0.16.5.jar:?]
";
        let report = run(&fabric(), console, None, &mods, dir.path());
        assert_eq!(
            culprits(&report),
            vec![
                (
                    "sodium.jar",
                    &CulpritReason::GameVersion {
                        needs: Some("1.20.1".into())
                    },
                    Confidence::Certain
                ),
                (
                    "indium.jar",
                    &CulpritReason::Incompatible {
                        other: "sodium".into()
                    },
                    Confidence::Certain
                ),
            ]
        );
        assert_eq!(missing(&report), vec![("fabric-api", vec!["iris.jar"])]);
        assert!(report.headline.as_deref().unwrap().contains("fabric-api"));
    }

    #[test]
    fn fabric_entrypoint_missing_library_is_installed() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [
            info("betterhud-1.2.0.jar", "betterhud", "Better HUD"),
            info("lithium.jar", "lithium", "Lithium"),
        ];
        let console = "\
[12:00:00] [main/ERROR]: Failed to start the minecraft server
java.lang.RuntimeException: Could not execute entrypoint stage 'main' due to errors, provided by 'betterhud' at 'dev.betterhud.Hud'!
\tat net.fabricmc.loader.impl.FabricLoaderImpl.lambda$invokeEntrypoints$2(FabricLoaderImpl.java:388) ~[fabric-loader-0.16.5.jar:?]
\tat net.minecraft.class_3218.handler$zfe000$lithium$tick(class_3218.java:5000) ~[server-intermediary.jar:?]
Caused by: java.lang.NoClassDefFoundError: me/shedaniel/clothconfig2/api/ConfigBuilder
\tat dev.betterhud.Hud.onInitialize(Hud.java:20) ~[betterhud-1.2.0.jar:?]
\tat net.fabricmc.loader.impl.FabricLoaderImpl.invokeEntrypoints(FabricLoaderImpl.java:384) ~[fabric-loader-0.16.5.jar:?]
\t... 6 more
Caused by: java.lang.ClassNotFoundException: me.shedaniel.clothconfig2.api.ConfigBuilder
\tat net.fabricmc.loader.impl.launch.knot.KnotClassDelegate.loadClass(KnotClassDelegate.java:226) ~[fabric-loader-0.16.5.jar:?]
\tat java.lang.ClassLoader.loadClass(ClassLoader.java:526) ~[?:?]
\t... 8 more
";
        let report = run(&fabric(), console, None, &mods, dir.path());
        assert!(report.culprits.is_empty(), "{:?}", report.culprits);
        assert_eq!(
            missing(&report),
            vec![("cloth-config", vec!["betterhud-1.2.0.jar"])]
        );
        // La causa real, no la primera línea del error.
        assert_eq!(
            report.headline.as_deref(),
            Some("java.lang.ClassNotFoundException: me.shedaniel.clothconfig2.api.ConfigBuilder")
        );
    }

    #[test]
    fn fabric_client_class_ignores_mixin_frames() {
        let dir = tempfile::tempdir().unwrap();
        jar(dir.path(), "betterhud.jar", &["com/example/hud/Hud.class"]);
        let mods = [
            info("betterhud.jar", "betterhud", "Better HUD"),
            info("lithium.jar", "lithium", "Lithium"),
        ];
        let console = "\
[12:00:00] [main/ERROR]: Failed to start the minecraft server
java.lang.RuntimeException: Could not execute entrypoint stage 'main' due to errors!
Caused by: java.lang.RuntimeException: Cannot load class net.minecraft.class_310 in environment type SERVER
\tat net.fabricmc.loader.impl.launch.knot.KnotClassDelegate.loadClass(KnotClassDelegate.java:235)
\tat com.example.hud.Hud.onInitialize(Hud.java:12)
\tat net.minecraft.class_3218.handler$zfe000$lithium$tick(class_3218.java:5000)
";
        let report = run(&fabric(), console, None, &mods, dir.path());
        assert_eq!(
            culprits(&report),
            vec![(
                "betterhud.jar",
                &CulpritReason::ClientOnly,
                Confidence::Certain
            )]
        );
        assert!(report.client_classes);
    }

    #[test]
    fn tick_crash_blames_the_root_cause_not_the_wrapper() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [
            info("examplemod-1.0.jar", "examplemod", "Example Mod"),
            info("lithium.jar", "lithium", "Lithium"),
        ];
        let console = "\
[12:00:10] [Server thread/INFO]: Done (12.345s)! For help, type \"help\"
[12:30:11] [Server thread/ERROR]: Encountered an unexpected exception
net.minecraft.class_148: Exception ticking world
\tat net.minecraft.server.MinecraftServer.method_3813(MinecraftServer.java:927) ~[server-intermediary.jar:?]
\tat net.minecraft.class_3218.handler$abc000$lithium$tickChunks(class_3218.java:600) ~[server-intermediary.jar:?]
Caused by: java.lang.NullPointerException: Cannot invoke \"net.minecraft.class_2680.method_26204()\" because \"state\" is null
\tat net.minecraft.class_1937.method_8501(class_1937.java:200) ~[server-intermediary.jar:?]
\tat com.example.worldgen.Feature.place(Feature.java:50) ~[examplemod-1.0.jar:?]
\tat net.minecraft.class_3218.method_18765(class_3218.java:300) ~[server-intermediary.jar:?]
\t... 8 more
[12:30:11] [Server thread/ERROR]: This crash report has been saved to: /srv/crash-reports/crash-2026-10-27_12.30.11-server.txt
";
        let report = run(&fabric(), console, None, &mods, dir.path());
        assert_eq!(
            culprits(&report),
            vec![(
                "examplemod-1.0.jar",
                &CulpritReason::ErrorInCode,
                Confidence::Likely
            )]
        );
        assert!(report
            .headline
            .as_deref()
            .unwrap()
            .starts_with("java.lang.NullPointerException"));
    }

    #[test]
    fn errors_that_did_not_stop_the_server_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [
            info("coolmod.jar", "coolmod", "Cool Mod"),
            info("sodium-fabric.jar", "sodium", "Sodium"),
            info("create.jar", "create", "Create"),
        ];
        let console = "\
[11:59:58] [main/WARN] [mixin/]: Error loading class: net/minecraft/client/gui/screens/Screen (java.lang.ClassNotFoundException: net/minecraft/client/gui/screens/Screen)
[11:59:58] [main/ERROR] [mixin/]: Mixin apply for mod coolmod failed coolmod.mixins.json:compat.JeiMixin from mod coolmod -> mezz.jei.Foo: org.spongepowered.asm.mixin.transformer.throwables.IllegalClassLoadError
[11:59:59] [main/WARN] [net.neoforged.fml.loading.moddiscovery.ModDiscoverer/SCAN]: File sodium-fabric.jar is a Fabric mod and cannot be loaded
[12:00:00] [Worker-Main-3/ERROR] [net.minecraft.world.item.crafting.RecipeManager/]: Parsing error loading recipe create:crushing/foo
com.google.gson.JsonSyntaxException: Unknown item 'foo:bar'
\tat TRANSFORMER/create@0.5.1.f/com.simibubi.create.Foo.bar(Foo.java:10) ~[create.jar%23200!/:0.5.1.f]
[12:00:10] [Server thread/INFO] [minecraft/DedicatedServer]: Done (12.345s)! For help, type \"help\"
[12:05:00] [Server thread/ERROR] [minecraft/MinecraftServer]: Encountered an unexpected exception
java.lang.OutOfMemoryError: Java heap space
\tat TRANSFORMER/create@0.5.1.f/com.simibubi.create.Big.alloc(Big.java:1) ~[create.jar%23200!/:0.5.1.f]
";
        let report = run(&neoforge(), console, None, &mods, dir.path());
        assert!(report.culprits.is_empty(), "{:?}", report.culprits);
        assert!(report.missing.is_empty());
        assert_eq!(report.cause, Some(CrashCause::OutOfMemory));
        assert!(!report.client_classes);
    }

    #[test]
    fn duplicates_keep_the_newest() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [
            versioned("jei-1.20.1-forge-15.10.0.1.jar", "jei", "15.10.0.1"),
            versioned("jei-1.20.1-forge-15.9.2.0.jar", "jei", "15.9.2.0"),
            versioned("balm-neoforge-1.21.1-21.0.19.jar", "balm", "21.0.19"),
            versioned("balm-neoforge-1.21.1-21.0.20.jar", "balm", "21.0.20"),
        ];
        let console = "\
[main/ERROR] [net.minecraftforge.fml.loading.UniqueModListBuilder/]: Found duplicate mods:
\tMod ID: 'jei' from mod files: jei-1.20.1-forge-15.10.0.1.jar, jei-1.20.1-forge-15.9.2.0.jar
[main/FATAL] [net.neoforged.neoforge.server.loading.ServerModLoader/]: Failed to load mods
net.neoforged.fml.ModLoadingException: Loading errors encountered:
\t- Mod balm is present in multiple files: balm-neoforge-1.21.1-21.0.19.jar, balm-neoforge-1.21.1-21.0.20.jar
";
        let report = run(&neoforge(), console, None, &mods, dir.path());
        assert_eq!(
            culprits(&report),
            vec![
                (
                    "balm-neoforge-1.21.1-21.0.19.jar",
                    &CulpritReason::Duplicate,
                    Confidence::Certain
                ),
                (
                    "jei-1.20.1-forge-15.9.2.0.jar",
                    &CulpritReason::Duplicate,
                    Confidence::Certain
                ),
            ]
        );
    }

    #[test]
    fn missing_classes_of_installed_mods_are_version_problems() {
        let dir = tempfile::tempdir().unwrap();
        jar(
            dir.path(),
            "examplelib.jar",
            &["com/example/lib/Other.class"],
        );
        let mods = [
            info("mowzies.jar", "mowziesmobs", "Mowzie's Mobs"),
            info("geckolib.jar", "geckolib", "GeckoLib"),
            info("user.jar", "user", "Lib User"),
            info("examplelib.jar", "examplelib", "Example Lib"),
        ];
        let console = "\
[main/ERROR] [net.minecraftforge.fml.javafmlmod.FMLModContainer/LOADING]: Failed to create mod instance. ModID: mowziesmobs, class com.bobmowzie.mowziesmobs.MowziesMobs
java.lang.NoClassDefFoundError: software/bernie/geckolib/core/animatable/GeoAnimatable
\tat TRANSFORMER/mowziesmobs@1.6.3/com.bobmowzie.mowziesmobs.MowziesMobs.<init>(MowziesMobs.java:62) ~[mowzies.jar%23200!/:1.6.3]
[main/ERROR] [net.minecraftforge.fml.javafmlmod.FMLModContainer/LOADING]: Failed to create mod instance. ModID: user, class com.user.User
java.lang.NoClassDefFoundError: com/example/lib/api/Thing
\tat TRANSFORMER/user@1.0/com.user.User.<init>(User.java:5) ~[user.jar%23201!/:1.0]
";
        let report = run(&forge(), console, None, &mods, dir.path());
        assert_eq!(
            culprits(&report),
            vec![
                (
                    "user.jar",
                    &CulpritReason::DependencyVersion {
                        dependency: "Example Lib".into(),
                        needs: None
                    },
                    Confidence::Certain
                ),
                (
                    "mowzies.jar",
                    &CulpritReason::DependencyVersion {
                        dependency: "GeckoLib".into(),
                        needs: None
                    },
                    Confidence::Certain
                ),
            ]
        );
        assert!(report.missing.is_empty());
    }

    #[test]
    fn mods_for_other_versions() {
        let dir = tempfile::tempdir().unwrap();
        jar(dir.path(), "newmod.jar", &["com/newmod/Main.class"]);
        let mods = [
            info("oldmod-1.19.2-1.0.jar", "oldmod", "Old Mod"),
            info("newmod.jar", "newmod", "New Mod"),
            info("forgemod.jar", "forgemod", "Forge Mod"),
        ];
        let console = "\
[12:00:00] [main/ERROR] [net.minecraft.server.Main/FATAL]: Failed to start the minecraft server
java.lang.NoSuchMethodError: 'net.minecraft.world.item.Item$Properties net.minecraft.world.item.Item$Properties.tab(net.minecraft.world.item.CreativeModeTab)'
\tat TRANSFORMER/oldmod@1.0/com.old.OldItems.<clinit>(OldItems.java:20) ~[oldmod-1.19.2-1.0.jar%23150!/:1.0]
Exception in thread \"main\" java.lang.UnsupportedClassVersionError: com/newmod/Main has been compiled by a more recent version of the Java Runtime (class file version 65.0), this version of the Java Runtime only recognizes class file versions up to 61.0
\tat java.base/java.lang.ClassLoader.defineClass1(Native Method)
[12:00:01] [main/ERROR] [net.neoforged.fml.javafmlmod.FMLModContainer/LOADING]: Failed to create mod instance. ModID: forgemod, class com.forgemod.ForgeMod
java.lang.NoClassDefFoundError: net/minecraftforge/event/TickEvent$ServerTickEvent
\tat TRANSFORMER/forgemod@1.0/com.forgemod.ForgeMod.<init>(ForgeMod.java:5) ~[forgemod.jar%23202!/:1.0]
";
        let report = run(&neoforge(), console, None, &mods, dir.path());
        assert_eq!(
            culprits(&report),
            vec![
                (
                    "forgemod.jar",
                    &CulpritReason::WrongLoader {
                        loader: "Forge".into()
                    },
                    Confidence::Certain
                ),
                (
                    "oldmod-1.19.2-1.0.jar",
                    &CulpritReason::GameVersion { needs: None },
                    Confidence::Certain
                ),
                (
                    "newmod.jar",
                    &CulpritReason::JavaVersion { needs: "21".into() },
                    Confidence::Certain
                ),
            ]
        );
    }

    #[test]
    fn mixin_failures_and_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [
            info("coolmod.jar", "coolmod", "Cool Mod"),
            info("moda.jar", "moda", "Mod A"),
            info("modb.jar", "modb", "Mod B"),
        ];
        let console = "\
[12:00:00] [main/ERROR]: Failed to start the minecraft server
java.lang.RuntimeException: Mixin transformation of net.minecraft.class_1234 failed
\tat net.fabricmc.loader.impl.launch.knot.KnotClassDelegate.getPostMixinClassByteArray(KnotClassDelegate.java:427)
Caused by: org.spongepowered.asm.mixin.transformer.throwables.MixinTransformerError: An unexpected critical error was encountered
\tat org.spongepowered.asm.mixin.transformer.MixinProcessor.applyMixins(MixinProcessor.java:392)
Caused by: org.spongepowered.asm.mixin.throwables.MixinApplyError: Mixin [coolmod.mixins.json:ServerMixin] from phase [DEFAULT] in config [coolmod.mixins.json] FAILED during APPLY
\tat org.spongepowered.asm.mixin.transformer.MixinProcessor.handleMixinError(MixinProcessor.java:638)
Caused by: org.spongepowered.asm.mixin.injection.throwables.InvalidInjectionException: Critical injection failure: @Inject annotation on onTick could not find any targets matching 'tick' in net.minecraft.class_1234. Using refmap coolmod-refmap.json [PREINJECT Applicator Phase -> coolmod.mixins.json:ServerMixin from mod coolmod -> Prepare Injections -> handler$zza000$coolmod$onTick -> Parse]
\tat org.spongepowered.asm.mixin.injection.struct.InjectionInfo.validateTargets(InjectionInfo.java:656)
[12:01:00] [main/ERROR]: Failed to start the minecraft server
org.spongepowered.asm.mixin.injection.throwables.InvalidInjectionException: @Redirect conflict. Skipping a.mixins.json:FooMixin from mod moda->@Redirect::bar(I)V with priority 1000, already redirected by b.mixins.json:BazMixin from mod modb->@Redirect::qux(I)V with priority 1000
\tat org.spongepowered.asm.mixin.injection.code.Injector.inject(Injector.java:271)
";
        let report = run(&fabric(), console, None, &mods, dir.path());
        assert_eq!(
            culprits(&report),
            vec![
                (
                    "moda.jar",
                    &CulpritReason::Incompatible {
                        other: "modb".into()
                    },
                    Confidence::Certain
                ),
                (
                    "coolmod.jar",
                    &CulpritReason::MixinFailed,
                    Confidence::Certain
                ),
            ]
        );
    }

    #[test]
    fn forge_suspected_mods_are_only_possible() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [
            info("create.jar", "create", "Create"),
            info("jei.jar", "jei", "JEI"),
        ];
        let crash = "\
---- Minecraft Crash Report ----
Description: Exception in server tick loop

java.lang.NullPointerException: Cannot invoke \"Object.hashCode()\" because \"key\" is null
\tat java.util.concurrent.ConcurrentHashMap.get(ConcurrentHashMap.java:936) ~[?:?]
\tat net.minecraft.server.level.ServerLevel.tick(ServerLevel.java:100) ~[server-1.20.1.jar%23190!/:?]

A detailed walkthrough of the error, its code path and all known details is as follows:
---------------------------------------------------------------------------------------

-- Head --
Thread: Server thread
Suspected Mods:
\tCreate (create), Version: 0.5.1.f
\t\tIssue tracker URL: https://github.com/Creators-of-Create/Create/issues
\t\tat TRANSFORMER/create@0.5.1.f/com.simibubi.create.Foo.tick(Foo.java:10)
Stacktrace:
\tat java.util.concurrent.ConcurrentHashMap.get(ConcurrentHashMap.java:936) ~[?:?]

-- System Details --
Details:
\tMod List:
\t\tjei.jar |Just Enough Items (jei), Version: 15.3 |jei |15.3 |DONE |Manifest: NOSIGNATURE
";
        let report = run(&forge(), "", Some(crash), &mods, dir.path());
        assert_eq!(
            culprits(&report),
            vec![(
                "create.jar",
                &CulpritReason::Suspected,
                Confidence::Possible
            )]
        );
    }

    #[test]
    fn watchdog_is_only_a_possible_culprit() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [info("slowmod.jar", "slowmod", "Slow Mod")];
        let crash = "\
---- Minecraft Crash Report ----
Description: Watching Server

java.lang.Error: Watchdog
\tat java.base/java.util.HashMap.get(HashMap.java:556)
\tat TRANSFORMER/slowmod@1.0/com.slow.Pathfinder.search(Pathfinder.java:88) ~[slowmod.jar%23150!/:1.0]
\tat TRANSFORMER/minecraft@1.20.1/net.minecraft.server.MinecraftServer.tickServer(MinecraftServer.java:828)
";
        let console = "\
[12:00:00] [Server Watchdog/ERROR]: A single server tick took 60.00 seconds (should be max 0.05)
[12:00:00] [Server Watchdog/ERROR]: Considering it to be crashed, server will forcibly shutdown.
";
        let report = run(&forge(), console, Some(crash), &mods, dir.path());
        assert_eq!(report.cause, Some(CrashCause::Watchdog));
        assert_eq!(
            culprits(&report),
            vec![(
                "slowmod.jar",
                &CulpritReason::Suspected,
                Confidence::Possible
            )]
        );
    }

    #[test]
    fn port_in_use_is_not_a_mod() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [info("create.jar", "create", "Create")];
        let console = "\
[12:00:00] [Server thread/WARN]: **** FAILED TO BIND TO PORT!
[12:00:00] [Server thread/WARN]: The exception was: java.net.BindException: Address already in use
[12:00:00] [Server thread/WARN]: Perhaps a server is already running on that port?
";
        let report = run(&forge(), console, None, &mods, dir.path());
        assert_eq!(report.cause, Some(CrashCause::PortInUse));
        assert!(report.culprits.is_empty());
    }

    #[test]
    fn client_only_mods_only_matter_for_client_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mut zoom = info("zoom.jar", "zoom", "Zoom");
        zoom.client_only = true;
        let mods = [
            info("create.jar", "create", "Create"),
            zoom,
            info("menus.jar", "menus", "Fancy Menus"),
        ];
        let sha1s = HashSet::from(["sha1-menus".to_string()]);

        // Un error que no es de clases del cliente: los mods de cliente no se tocan.
        let tick = "\
[12:30:11] [Server thread/ERROR]: Encountered an unexpected exception
java.lang.IllegalStateException: boom
\tat TRANSFORMER/create@0.5.1.f/com.simibubi.create.Foo.tick(Foo.java:10) ~[create.jar%23200!/:0.5.1.f]
";
        let mut report = run(&forge(), tick, None, &mods, dir.path());
        add_client_only(&mut report, &mods, &sha1s);
        assert_eq!(
            culprits(&report),
            vec![(
                "create.jar",
                &CulpritReason::ErrorInCode,
                Confidence::Likely
            )]
        );

        // Una clase del cliente sin saber de qué mod: los de cliente son los posibles.
        let client = "\
[12:00:00] [main/ERROR] [net.minecraft.server.Main/FATAL]: Failed to start the minecraft server
java.lang.RuntimeException: Attempted to load class net/minecraft/client/Minecraft for invalid dist DEDICATED_SERVER
\tat net.minecraftforge.fml.loading.RuntimeDistCleaner.processClassWithFlags(RuntimeDistCleaner.java:57)
";
        let mut report = run(&forge(), client, None, &mods, dir.path());
        assert!(report.culprits.is_empty());
        // Como en la app: primero con los metadatos y luego con Modrinth.
        add_client_only(&mut report, &mods, &HashSet::new());
        add_client_only(&mut report, &mods, &sha1s);
        assert_eq!(
            culprits(&report),
            vec![
                (
                    "menus.jar",
                    &CulpritReason::ClientOnly,
                    Confidence::Possible
                ),
                ("zoom.jar", &CulpritReason::ClientOnly, Confidence::Possible),
            ]
        );
    }

    #[test]
    fn disabled_mods_are_not_blamed_and_can_be_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let mut old = info("jei-old.jar.disabled", "jei", "JEI");
        old.enabled = false;
        let mut api = info("fabric-api.jar.disabled", "fabric-api", "Fabric API");
        api.enabled = false;
        let mods = [old, info("jei-new.jar", "jei", "JEI"), api];
        let console = "\
[main/ERROR] [net.minecraftforge.fml.loading.ModSorter/LOADING]: Missing or unsupported mandatory dependencies:
\tMod ID: 'fabric', Requested by: 'jei', Expected range: '*', Actual version: '[MISSING]'
";
        let report = run(&forge(), console, None, &mods, dir.path());
        assert!(report.culprits.is_empty());
        assert_eq!(missing(&report), vec![("fabric-api", vec!["jei-new.jar"])]);
        assert_eq!(
            report.missing[0].disabled_file.as_deref(),
            Some("fabric-api.jar.disabled")
        );
        assert_eq!(report.missing[0].version, None);
    }

    #[test]
    fn installed_mods_are_no_longer_missing() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [info("mowzie.jar", "mowziesmobs", "Mowzie's Mobs")];
        let console = "\
[main/ERROR] [net.minecraftforge.fml.loading.ModSorter/LOADING]: Missing or unsupported mandatory dependencies:
\tMod ID: 'geckolib', Requested by: 'mowziesmobs', Expected range: '[4.2,)', Actual version: '[MISSING]'
";
        let mut report = run(&forge(), console, None, &mods, dir.path());
        assert_eq!(missing(&report), vec![("geckolib", vec!["mowzie.jar"])]);
        let mut off = info("geckolib.jar.disabled", "geckolib", "GeckoLib");
        off.enabled = false;
        forget_installed(&mut report, &[mods[0].clone(), off]);
        assert_eq!(report.missing.len(), 1);
        forget_installed(&mut report, &[info("geckolib.jar", "geckolib", "GeckoLib")]);
        assert!(report.missing.is_empty());
    }

    #[test]
    fn a_client_only_mod_that_needs_another_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let mut iris = info("iris.jar", "iris", "Iris");
        iris.client_only = true;
        let mods = [iris];
        let console = "\
[12:00:00] [main/ERROR]: Incompatible mods found!
net.fabricmc.loader.impl.FormattedException: Some of your mods are incompatible with the game or each other!
More details:
\t - Mod 'Iris' (iris) 1.6.11 requires version 0.5.8 or later of mod 'Sodium' (sodium), which is missing!
";
        let report = run(&fabric(), console, None, &mods, dir.path());
        assert_eq!(
            culprits(&report),
            vec![("iris.jar", &CulpritReason::ClientOnly, Confidence::Certain)]
        );
        assert!(report.missing.is_empty());
    }

    #[test]
    fn nothing_to_blame() {
        let dir = tempfile::tempdir().unwrap();
        let mods = [info("create.jar", "create", "Create")];
        let report = run(
            &forge(),
            "[main/INFO]: Starting minecraft server version 1.20.1\n[main/WARN]: Mixin config create.mixins.json does not specify \"minVersion\" property\n",
            None,
            &mods,
            dir.path(),
        );
        assert!(report.culprits.is_empty());
        assert!(report.missing.is_empty());
        assert!(report.headline.is_none());
        assert!(report.cause.is_none());
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

    #[test]
    fn versions_in_any_language() {
        assert_eq!(version_text("4.2 or above"), "≥ 4.2");
        assert_eq!(version_text("0.5.8 or later"), "≥ 0.5.8");
        assert_eq!(version_text("below 2.0"), "< 2.0");
        assert_eq!(version_text("1.0 or above and below 2.0"), "≥ 1.0, < 2.0");
        assert_eq!(version_text("between 1.0 and 2.0 (inclusive)"), "1.0 – 2.0");
        assert_eq!(version_text("1.20.x"), "1.20.x");
    }

    #[test]
    fn helpers() {
        assert_eq!(pretty_range("[4.2,)").as_deref(), Some("≥ 4.2"));
        assert_eq!(
            pretty_range("[1.19.2,1.19.3)").as_deref(),
            Some("≥ 1.19.2, < 1.19.3")
        );
        assert_eq!(pretty_range("[1.20.1]").as_deref(), Some("1.20.1"));
        assert_eq!(pretty_range("*"), None);
        assert_eq!(
            version_cmp(Some("15.10.0"), Some("15.9.2")),
            Ordering::Greater
        );
        assert_eq!(version_cmp(None, Some("1")), Ordering::Less);
        assert_eq!(
            clean("§eGeckoLib§r \u{1b}[31mrojo\u{1b}[0m\r"),
            "GeckoLib rojo"
        );
        assert_eq!(norm_id("Cloth_Config"), norm_id("cloth-config"));
        assert_eq!(
            library_of("me.shedaniel.clothconfig2.api.ConfigBuilder"),
            "me.shedaniel.clothconfig2"
        );
        assert_eq!(
            member_owner("'void net.minecraft.world.level.Level.foo(int)'").as_deref(),
            Some("net.minecraft.world.level.Level")
        );
        assert!(is_obfuscated("field_1234") && is_obfuscated("f_12345_") && !is_obfuscated("foo"));
    }
}
