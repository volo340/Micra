//! Idioma de los textos que genera el núcleo: errores y notificaciones.
//!
//! La interfaz traduce sus propios textos; aquí solo están los mensajes que
//! nacen en Rust. Los errores existen en español e inglés (el resto de idiomas
//! usa el inglés) y las notificaciones, en todos los idiomas de la app.

use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    Es,
    En,
    Pt,
    Fr,
    De,
}

impl Lang {
    pub const ALL: [Lang; 5] = [Lang::Es, Lang::En, Lang::Pt, Lang::Fr, Lang::De];

    /// Idioma a partir de una etiqueta del sistema como `es-ES` o `pt_BR`.
    pub fn from_tag(tag: &str) -> Option<Lang> {
        let primary = tag.split(['-', '_', '.']).next()?.to_ascii_lowercase();
        Lang::ALL.into_iter().find(|l| l.code() == primary)
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::Es => "es",
            Lang::En => "en",
            Lang::Pt => "pt",
            Lang::Fr => "fr",
            Lang::De => "de",
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_lang(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::En,
        2 => Lang::Pt,
        3 => Lang::Fr,
        4 => Lang::De,
        _ => Lang::Es,
    }
}

/// `true` si los mensajes deben salir en español.
pub fn is_es() -> bool {
    lang() == Lang::Es
}

/// Mensaje en español o en inglés según el idioma activo. Acepta los mismos
/// argumentos que `format!`, también con nombres capturados:
/// `tr!("no existe {name}", "{name} does not exist")`.
#[macro_export]
macro_rules! tr {
    ($es:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::is_es() {
            format!($es $(, $arg)*)
        } else {
            format!($en $(, $arg)*)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        assert_eq!(Lang::from_tag("es-ES"), Some(Lang::Es));
        assert_eq!(Lang::from_tag("pt_BR.UTF-8"), Some(Lang::Pt));
        assert_eq!(Lang::from_tag("EN"), Some(Lang::En));
        assert_eq!(Lang::from_tag("ja-JP"), None);
    }
}
