//! Comparación de versiones ("1.20.1", "21.1.77", "1.20.1-47.2.20", "26.1").

use std::cmp::Ordering;

/// Compara dos versiones por partes: los números como números y el resto
/// como texto. Una etiqueta de prerrelease (`-beta`, `-rc1`) queda por
/// debajo de la versión final: `21.0.3-beta < 21.0.3`.
pub fn compare(a: &str, b: &str) -> Ordering {
    let pa = parts(a);
    let pb = parts(b);
    for i in 0..pa.len().max(pb.len()) {
        match (pa.get(i), pb.get(i)) {
            (Some(x), Some(y)) => {
                let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(nx), Ok(ny)) => nx.cmp(&ny),
                    // Un número gana a una etiqueta: `1.0.1 > 1.0.beta`.
                    (Ok(_), Err(_)) => Ordering::Greater,
                    (Err(_), Ok(_)) => Ordering::Less,
                    (Err(_), Err(_)) => x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase()),
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            // Lo que sobra decide: un número más es una versión mayor
            // (`1.21.1 > 1.21`); una etiqueta, una prerrelease.
            (Some(x), None) => {
                return if is_number(x) {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
            (None, Some(y)) => {
                return if is_number(y) {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            }
            (None, None) => break,
        }
    }
    Ordering::Equal
}

/// `true` si `version` es igual o posterior a `min`.
pub fn at_least(version: &str, min: &str) -> bool {
    compare(version, min) != Ordering::Less
}

fn parts(v: &str) -> Vec<&str> {
    v.trim()
        .split(['.', '-', '+', '_'])
        .filter(|p| !p.is_empty())
        .collect()
}

fn is_number(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_versions() {
        assert_eq!(compare("1.20.1", "1.20.1"), Ordering::Equal);
        assert_eq!(compare("1.21", "1.20.4"), Ordering::Greater);
        assert_eq!(compare("1.21.1", "1.21"), Ordering::Greater);
        assert_eq!(compare("1.9", "1.10"), Ordering::Less);
        assert_eq!(compare("26.1", "1.21.11"), Ordering::Greater);
        assert_eq!(compare("21.0.3-beta", "21.0.3"), Ordering::Less);
        assert_eq!(compare("1.20.1-47.2.20", "1.20.1-47.10.0"), Ordering::Less);
        assert_eq!(compare("0.16.10", "0.16.9"), Ordering::Greater);
    }

    #[test]
    fn minimums() {
        assert!(at_least("1.20.3", "1.20.3"));
        assert!(at_least("1.21", "1.20.3"));
        assert!(at_least("26.1", "1.20.3"));
        assert!(!at_least("1.20.2", "1.20.3"));
        assert!(!at_least("1.8.9", "1.20.3"));
    }
}
