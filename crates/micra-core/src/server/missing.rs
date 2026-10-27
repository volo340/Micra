//! Dónde descargar los mods que faltan después de un error de arranque.
//!
//! Lo más fiable es lo que declaran en Modrinth los propios mods que los
//! piden (sus dependencias). Si no, se prueba el proyecto de Modrinth que se
//! llama como el mod y, por último, el buscador de Modrinth y CurseForge.
//! Solo vale un proyecto con versión para la versión de Minecraft y el loader
//! de este servidor, que es la que se instalaría.

use std::collections::{HashMap, HashSet};

use futures_util::future::join_all;

use crate::providers::curseforge::CurseForgeClient;
use crate::providers::modrinth::{ModrinthClient, Project};
use crate::providers::search::{self, dedupe_key, ContentHit, ContentKind, ContentQuery, Source};
use crate::server::config::ServerConfig;
use crate::server::diagnose::{FoundMod, MissingMod};
use crate::server::mods::ModInfo;

/// Como mucho, cuántos mods que faltan se buscan de una vez.
const MAX_LOOKUPS: usize = 8;

/// Mods cuyo proyecto de Modrinth no se llama como su id.
const SLUGS: &[(&str, &str)] = &[
    ("architectury", "architectury-api"),
    ("yet_another_config_lib_v3", "yacl"),
    ("yet_another_config_lib", "yacl"),
    ("kotlinforforge", "kotlin-for-forge"),
    ("forgeconfigapiport", "forge-config-api-port"),
    ("puzzleslib", "puzzles-lib"),
    ("resourcefullib", "resourceful-lib"),
    ("bookshelf", "bookshelf-lib"),
    ("owo", "owo-lib"),
    ("geckolib3", "geckolib"),
];

/// Lo que unos ponen al final del nombre y otros no («Curios API», «curios»).
const SUFFIXES: &[&str] = &[
    "library", "api", "lib", "core", "mod", "neoforge", "forge", "fabric", "quilt",
];

/// Busca dónde descargar los mods que faltan (los desactivados no: basta con
/// activarlos). Devuelve el id de cada uno que se encontró y su proyecto.
pub async fn find(
    modrinth: &ModrinthClient,
    curseforge: Option<&CurseForgeClient>,
    cfg: &ServerConfig,
    mods: &[ModInfo],
    missing: &[MissingMod],
) -> Vec<(String, FoundMod)> {
    let loaders = cfg.loader.modrinth_loaders(&cfg.mc_version);
    let wanted: Vec<&MissingMod> = missing
        .iter()
        .filter(|m| m.disabled_file.is_none() && m.found.is_none())
        .take(MAX_LOOKUPS)
        .collect();
    if loaders.is_empty() || wanted.is_empty() {
        return Vec::new();
    }
    let finder = Finder {
        modrinth,
        curseforge,
        cfg,
        loaders,
    };
    let declared = finder.declared(mods, &wanted).await;
    join_all(wanted.iter().map(|m| async {
        let found = match declared.get(&m.id) {
            Some(found) => Some(found.clone()),
            None => match finder.by_slug(&m.id).await {
                Some(found) => Some(found),
                None => finder.by_search(&m.id).await,
            },
        };
        found.map(|f| (m.id.clone(), f))
    }))
    .await
    .into_iter()
    .flatten()
    .collect()
}

struct Finder<'a> {
    modrinth: &'a ModrinthClient,
    curseforge: Option<&'a CurseForgeClient>,
    cfg: &'a ServerConfig,
    loaders: Vec<&'static str>,
}

impl Finder<'_> {
    /// Si el proyecto tiene versión para este servidor.
    async fn has_version(&self, project_id: &str) -> bool {
        self.modrinth
            .project_versions_any(project_id, Some(&self.cfg.mc_version), &self.loaders)
            .await
            .is_ok_and(|v| !v.is_empty())
    }

    /// Las dependencias que declaran en Modrinth los mods que piden los que
    /// faltan, y cuál de ellas es cada uno.
    async fn declared(
        &self,
        mods: &[ModInfo],
        wanted: &[&MissingMod],
    ) -> HashMap<String, FoundMod> {
        let mut out = HashMap::new();
        let hashes: Vec<String> = mods.iter().map(|m| m.sha1.clone()).collect();
        let Ok(versions) = self.modrinth.versions_by_hashes(&hashes).await else {
            return out;
        };
        // Lo que ya está instalado no es lo que falta.
        let installed: HashSet<&str> = versions.values().map(|v| v.project_id.as_str()).collect();
        let sha1_of: HashMap<&str, &str> = mods
            .iter()
            .map(|m| (m.file_name.as_str(), m.sha1.as_str()))
            .collect();
        // De cada mod que pide alguno: sus dependencias que no están y si son
        // obligatorias.
        let mut deps_of: HashMap<&str, Vec<(&str, bool)>> = HashMap::new();
        for r in wanted.iter().flat_map(|m| &m.required_by) {
            let Some(version) = sha1_of
                .get(r.file_name.as_str())
                .and_then(|sha1| versions.get(*sha1))
            else {
                continue;
            };
            let deps = version
                .dependencies
                .iter()
                .filter_map(|d| {
                    let required = match d.dependency_type.as_str() {
                        "required" => true,
                        "optional" => false,
                        _ => return None,
                    };
                    let id = d.project_id.as_deref()?;
                    (!installed.contains(id)).then_some((id, required))
                })
                .collect();
            deps_of.insert(r.file_name.as_str(), deps);
        }
        let mut ids: Vec<String> = deps_of
            .values()
            .flatten()
            .map(|(id, _)| id.to_string())
            .collect();
        ids.sort();
        ids.dedup();
        let Ok(projects) = self.modrinth.projects(&ids).await else {
            return out;
        };
        let projects: HashMap<&str, &Project> =
            projects.iter().map(|p| (p.id.as_str(), p)).collect();

        for m in wanted {
            let mut candidates: Vec<(&Project, bool)> = Vec::new();
            for r in &m.required_by {
                for (id, required) in deps_of.get(r.file_name.as_str()).into_iter().flatten() {
                    let Some(p) = projects.get(id) else {
                        continue;
                    };
                    // Instalado desde otro sitio (CurseForge, a mano).
                    if mods.iter().any(|i| looks_installed(i, p)) {
                        continue;
                    }
                    match candidates.iter_mut().find(|(c, _)| c.id == p.id) {
                        Some((_, r)) => *r |= *required,
                        None => candidates.push((p, *required)),
                    }
                }
            }
            // Si esos mods no piden ningún otro de los que faltan, su única
            // dependencia obligatoria que no está tiene que ser esta.
            let alone = !wanted.iter().any(|o| {
                o.id != m.id
                    && o.required_by
                        .iter()
                        .any(|r| m.required_by.iter().any(|x| x.file_name == r.file_name))
            });
            let Some(project) = pick_declared(&m.id, &candidates, alone) else {
                continue;
            };
            if self.has_version(&project.id).await {
                out.insert(
                    m.id.clone(),
                    FoundMod {
                        source: Source::Modrinth,
                        project_id: project.id.clone(),
                        title: project.title.clone(),
                    },
                );
            }
        }
        out
    }

    /// El proyecto de Modrinth que se llama como el mod.
    async fn by_slug(&self, id: &str) -> Option<FoundMod> {
        for slug in slug_candidates(id) {
            let Ok(Some(project)) = self.modrinth.project(&slug).await else {
                continue;
            };
            let is_mod = project.project_type.is_empty() || project.project_type == "mod";
            if is_mod && self.has_version(&project.id).await {
                return Some(FoundMod {
                    source: Source::Modrinth,
                    project_id: project.id,
                    title: project.title,
                });
            }
        }
        None
    }

    /// El primer resultado del buscador (ya filtrado para este servidor) que
    /// se llama como el mod.
    async fn by_search(&self, id: &str) -> Option<FoundMod> {
        let query = ContentQuery {
            query: id.replace(['_', '-'], " "),
            kind: ContentKind::Mod,
            game_version: Some(self.cfg.mc_version.clone()),
            loaders: self.loaders.iter().map(|l| l.to_string()).collect(),
            cursor: None,
        };
        let page = search::search(self.modrinth, self.curseforge, &query)
            .await
            .ok()?;
        let hit = pick_hit(id, &page.hits)?;
        Some(FoundMod {
            source: hit.source,
            project_id: hit.id.clone(),
            title: hit.title.clone(),
        })
    }
}

/// De las dependencias que declaran los mods que lo piden, la que es el mod
/// que falta: la que se llama como él o, si solo falta uno, la única
/// obligatoria.
fn pick_declared<'a>(
    id: &str,
    candidates: &[(&'a Project, bool)],
    alone: bool,
) -> Option<&'a Project> {
    if let Some((p, _)) = candidates
        .iter()
        .find(|(p, _)| same_mod(id, &p.slug) || same_mod(id, &p.title))
    {
        return Some(p);
    }
    let mut required = candidates.iter().filter(|(_, r)| *r);
    match (alone, required.next(), required.next()) {
        (true, Some((p, _)), None) => Some(p),
        _ => None,
    }
}

/// El primer resultado que se llama como el mod.
fn pick_hit<'a>(id: &str, hits: &'a [ContentHit]) -> Option<&'a ContentHit> {
    hits.iter().find(|h| same_mod(id, &h.title))
}

/// Si un mod instalado es este proyecto (por su id o su nombre).
fn looks_installed(installed: &ModInfo, project: &Project) -> bool {
    let id = installed.mod_id.as_deref().unwrap_or("");
    let name = installed.name.as_deref().unwrap_or("");
    [id, name]
        .iter()
        .any(|n| same_mod(n, &project.slug) || same_mod(n, &project.title))
}

/// Si un id y un nombre son del mismo mod: «cloth_config» y «Cloth Config
/// API», «curios» y «Curios API», «geckolib3» y «GeckoLib».
fn same_mod(a: &str, b: &str) -> bool {
    let (a, b) = (dedupe_key(a), dedupe_key(b));
    if a.chars().count() < 3 || b.chars().count() < 3 {
        return false;
    }
    a == b || bare(&a) == bare(&b)
}

/// Sin el número de versión ni la terminación habitual del final.
fn bare(key: &str) -> &str {
    let digits = key.trim_end_matches(|c: char| c.is_ascii_digit());
    let key = if digits.len() < key.len() {
        digits.strip_suffix('v').unwrap_or(digits)
    } else {
        key
    };
    SUFFIXES
        .iter()
        .find_map(|s| key.strip_suffix(s).filter(|rest| rest.chars().count() >= 3))
        .unwrap_or(key)
}

/// Slugs de Modrinth que puede tener el proyecto de un mod.
fn slug_candidates(id: &str) -> Vec<String> {
    let id = id.to_ascii_lowercase();
    let mut out: Vec<String> = Vec::new();
    let known = SLUGS
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, s)| s.to_string());
    for slug in known.into_iter().chain([id.clone(), id.replace('_', "-")]) {
        if is_slug(&slug) && !out.contains(&slug) {
            out.push(slug);
        }
    }
    out
}

/// Solo lo que puede ser un slug: va en la dirección de la petición.
fn is_slug(s: &str) -> bool {
    (2..=64).contains(&s.len())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str, slug: &str, title: &str) -> Project {
        Project {
            id: id.into(),
            slug: slug.into(),
            title: title.into(),
            icon_url: None,
            server_side: "required".into(),
            project_type: "mod".into(),
        }
    }

    fn hit(source: Source, id: &str, title: &str) -> ContentHit {
        ContentHit {
            source,
            id: id.into(),
            title: title.into(),
            author: None,
            description: String::new(),
            icon_url: None,
            downloads: 0,
            game_version: None,
            dedupe_key: dedupe_key(title),
        }
    }

    #[test]
    fn names_of_the_same_mod() {
        assert!(same_mod("cloth_config", "Cloth Config API"));
        assert!(same_mod("cloth-config", "cloth-config"));
        assert!(same_mod("curios", "Curios API"));
        assert!(same_mod("geckolib3", "GeckoLib"));
        assert!(same_mod("architectury", "Architectury API"));
        assert!(same_mod("yet_another_config_lib_v3", "YetAnotherConfigLib"));
        assert!(same_mod("kotlinforforge", "Kotlin for Forge"));
        assert!(same_mod("fabric-api", "Fabric API"));
        assert!(same_mod("ftblibrary", "FTB Library (Forge)"));
        assert!(same_mod("playeranimator", "playerAnimator"));
        assert!(!same_mod("create", "Createaddition"));
        assert!(!same_mod("jei", "Just Enough Items"));
        assert!(!same_mod("forgeconfigapiport", "Forge"));
        assert!(!same_mod("fabric-api", "Fabric Language Kotlin"));
        assert!(!same_mod("sodium", "Sodium Extra"));
    }

    #[test]
    fn slugs_to_try() {
        assert_eq!(
            slug_candidates("cloth_config"),
            ["cloth_config", "cloth-config"]
        );
        assert_eq!(
            slug_candidates("architectury"),
            ["architectury-api", "architectury"]
        );
        assert_eq!(slug_candidates("jei"), ["jei"]);
        // Nada que se salga de la dirección de la petición.
        assert!(slug_candidates("../search?x").is_empty());
        assert!(slug_candidates("a").is_empty());
    }

    #[test]
    fn the_declared_dependency_that_is_missing() {
        let anim = project("anim", "playeranimator", "playerAnimator");
        let cloth = project("cloth", "cloth-config", "Cloth Config API");
        let core = project(
            "core",
            "supermartijn642s-core-lib",
            "SuperMartijn642's Core Lib",
        );
        let both = [(&anim, true), (&cloth, true)];
        assert_eq!(
            pick_declared("playeranimator", &both, false).map(|p| p.id.as_str()),
            Some("anim")
        );
        assert_eq!(
            pick_declared("cloth_config", &both, false).map(|p| p.id.as_str()),
            Some("cloth")
        );
        // Otro nombre: vale si es la única obligatoria y solo falta este.
        let only = [(&core, true)];
        assert_eq!(
            pick_declared("supermartijn642corelib", &only, true).map(|p| p.id.as_str()),
            Some("core")
        );
        assert!(pick_declared("supermartijn642corelib", &only, false).is_none());
        let optional = [(&core, false)];
        assert!(pick_declared("supermartijn642corelib", &optional, true).is_none());
        assert!(pick_declared("geckolib", &both, true).is_none());
    }

    #[test]
    fn the_search_result_with_the_same_name() {
        let hits = [
            hit(Source::Modrinth, "a", "Cloth Config Extras"),
            hit(
                Source::Curseforge,
                "123",
                "Cloth Config API (Fabric/Forge/NeoForge)",
            ),
        ];
        let found = pick_hit("cloth_config", &hits).unwrap();
        assert_eq!(
            (found.source, found.id.as_str()),
            (Source::Curseforge, "123")
        );
        assert!(pick_hit("geckolib", &hits).is_none());
    }

    #[test]
    fn installed_from_somewhere_else() {
        let m = ModInfo {
            file_name: "cloth-config.jar".into(),
            enabled: true,
            size: 0,
            sha1: String::new(),
            mod_id: Some("cloth_config".into()),
            name: None,
            version: None,
            from_pack: false,
            client_only: false,
        };
        assert!(looks_installed(
            &m,
            &project("cloth", "cloth-config", "Cloth Config API")
        ));
        assert!(!looks_installed(
            &m,
            &project("anim", "playeranimator", "playerAnimator")
        ));
    }
}
