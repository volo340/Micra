//! Lectura y escritura de NBT, el formato binario de los mundos de Minecraft,
//! lo justo para cambiar un valor de `level.dat` sin perder nada más: el
//! árbol se lee entero y se vuelve a escribir igual, con los textos en su
//! codificación original (el UTF-8 modificado de Java).

use std::io::{Read, Write};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;

use crate::{tr, Error, Result};

/// Los archivos que lee Micra no pasan de unos 20 niveles; el límite evita
/// agotar la pila con un archivo dañado (Minecraft admite hasta 512).
const MAX_DEPTH: usize = 128;
/// `level.dat` ocupa unos KB: esto corta archivos dañados o maliciosos.
const MAX_SIZE: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<u8>),
    /// Bytes en UTF-8 modificado (igual que UTF-8 en texto ASCII).
    String(Vec<u8>),
    /// Tipo de los elementos y los elementos.
    List(u8, Vec<Tag>),
    /// En el orden del archivo.
    Compound(Vec<(Vec<u8>, Tag)>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    pub fn id(&self) -> u8 {
        match self {
            Tag::Byte(_) => 1,
            Tag::Short(_) => 2,
            Tag::Int(_) => 3,
            Tag::Long(_) => 4,
            Tag::Float(_) => 5,
            Tag::Double(_) => 6,
            Tag::ByteArray(_) => 7,
            Tag::String(_) => 8,
            Tag::List(..) => 9,
            Tag::Compound(_) => 10,
            Tag::IntArray(_) => 11,
            Tag::LongArray(_) => 12,
        }
    }

    /// Etiqueta hija de un compuesto.
    pub fn get(&self, name: &str) -> Option<&Tag> {
        match self {
            Tag::Compound(entries) => entries
                .iter()
                .find(|(n, _)| n == name.as_bytes())
                .map(|(_, t)| t),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Tag> {
        match self {
            Tag::Compound(entries) => entries
                .iter_mut()
                .find(|(n, _)| n == name.as_bytes())
                .map(|(_, t)| t),
            _ => None,
        }
    }

    /// Pone o sustituye una etiqueta hija (solo en compuestos).
    pub fn set(&mut self, name: &str, tag: Tag) {
        if let Tag::Compound(entries) = self {
            match entries.iter_mut().find(|(n, _)| n == name.as_bytes()) {
                Some((_, old)) => *old = tag,
                None => entries.push((name.as_bytes().to_vec(), tag)),
            }
        }
    }

    /// Un booleano guardado como número (0 o 1) o como texto ("true").
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Tag::Byte(v) => Some(*v != 0),
            Tag::Short(v) => Some(*v != 0),
            Tag::Int(v) => Some(*v != 0),
            Tag::Long(v) => Some(*v != 0),
            Tag::String(s) => match s.as_slice() {
                b"true" => Some(true),
                b"false" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }
}

/// Lee un archivo NBT comprimido con gzip (`level.dat` y los `.dat` de
/// `data/`). Devuelve el nombre de la raíz y la raíz.
pub fn read_gzip(compressed: &[u8]) -> Result<(Vec<u8>, Tag)> {
    let mut raw = Vec::new();
    GzDecoder::new(compressed)
        .take(MAX_SIZE + 1)
        .read_to_end(&mut raw)?;
    if raw.len() as u64 > MAX_SIZE {
        return Err(corrupt());
    }
    read(&raw)
}

/// Lee NBT sin comprimir.
pub fn read(raw: &[u8]) -> Result<(Vec<u8>, Tag)> {
    let mut reader = Reader { data: raw, pos: 0 };
    if reader.u8()? != 10 {
        return Err(corrupt());
    }
    let name = reader.short_bytes()?;
    let root = reader.payload(10, 0)?;
    Ok((name, root))
}

pub fn write(name: &[u8], root: &Tag) -> Vec<u8> {
    let mut out = vec![root.id()];
    put_short_bytes(&mut out, name);
    put_payload(&mut out, root);
    out
}

pub fn write_gzip(name: &[u8], root: &Tag) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&write(name, root))?;
    Ok(encoder.finish()?)
}

fn corrupt() -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        tr!("datos NBT dañados", "damaged NBT data"),
    ))
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|end| *end <= self.data.len())
            .ok_or_else(corrupt)?;
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.take(N)?.try_into().expect("take devuelve N bytes"))
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn len(&mut self) -> Result<usize> {
        usize::try_from(i32::from_be_bytes(self.array()?)).map_err(|_| corrupt())
    }

    fn short_bytes(&mut self) -> Result<Vec<u8>> {
        let n = u16::from_be_bytes(self.array()?) as usize;
        Ok(self.take(n)?.to_vec())
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    fn payload(&mut self, id: u8, depth: usize) -> Result<Tag> {
        if depth > MAX_DEPTH {
            return Err(corrupt());
        }
        Ok(match id {
            1 => Tag::Byte(i8::from_be_bytes(self.array()?)),
            2 => Tag::Short(i16::from_be_bytes(self.array()?)),
            3 => Tag::Int(i32::from_be_bytes(self.array()?)),
            4 => Tag::Long(i64::from_be_bytes(self.array()?)),
            5 => Tag::Float(f32::from_be_bytes(self.array()?)),
            6 => Tag::Double(f64::from_be_bytes(self.array()?)),
            7 => {
                let n = self.len()?;
                Tag::ByteArray(self.take(n)?.to_vec())
            }
            8 => Tag::String(self.short_bytes()?),
            9 => {
                let elem = self.u8()?;
                let n = self.len()?;
                // Cada elemento ocupa al menos un byte: no reservar de más.
                let mut items = Vec::with_capacity(n.min(self.remaining()));
                for _ in 0..n {
                    items.push(self.payload(elem, depth + 1)?);
                }
                Tag::List(elem, items)
            }
            10 => {
                let mut entries = Vec::new();
                loop {
                    let id = self.u8()?;
                    if id == 0 {
                        break;
                    }
                    let name = self.short_bytes()?;
                    entries.push((name, self.payload(id, depth + 1)?));
                }
                Tag::Compound(entries)
            }
            11 => {
                let n = self.len()?;
                let bytes = self.take(n.checked_mul(4).ok_or_else(corrupt)?)?;
                Tag::IntArray(
                    bytes
                        .chunks_exact(4)
                        .map(|c| i32::from_be_bytes(c.try_into().unwrap()))
                        .collect(),
                )
            }
            12 => {
                let n = self.len()?;
                let bytes = self.take(n.checked_mul(8).ok_or_else(corrupt)?)?;
                Tag::LongArray(
                    bytes
                        .chunks_exact(8)
                        .map(|c| i64::from_be_bytes(c.try_into().unwrap()))
                        .collect(),
                )
            }
            _ => return Err(corrupt()),
        })
    }
}

fn put_short_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(bytes);
}

fn put_len(out: &mut Vec<u8>, len: usize) {
    out.extend_from_slice(&(len as i32).to_be_bytes());
}

fn put_payload(out: &mut Vec<u8>, tag: &Tag) {
    match tag {
        Tag::Byte(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Short(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Int(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Long(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Float(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Double(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::ByteArray(bytes) => {
            put_len(out, bytes.len());
            out.extend_from_slice(bytes);
        }
        Tag::String(bytes) => put_short_bytes(out, bytes),
        Tag::List(elem, items) => {
            out.push(*elem);
            put_len(out, items.len());
            for item in items {
                put_payload(out, item);
            }
        }
        Tag::Compound(entries) => {
            for (name, value) in entries {
                out.push(value.id());
                put_short_bytes(out, name);
                put_payload(out, value);
            }
            out.push(0);
        }
        Tag::IntArray(values) => {
            put_len(out, values.len());
            for v in values {
                out.extend_from_slice(&v.to_be_bytes());
            }
        }
        Tag::LongArray(values) => {
            put_len(out, values.len());
            for v in values {
                out.extend_from_slice(&v.to_be_bytes());
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn compound(entries: Vec<(&str, Tag)>) -> Tag {
        Tag::Compound(
            entries
                .into_iter()
                .map(|(n, t)| (n.as_bytes().to_vec(), t))
                .collect(),
        )
    }

    pub fn string(s: &str) -> Tag {
        Tag::String(s.as_bytes().to_vec())
    }

    fn sample() -> Tag {
        compound(vec![(
            "Data",
            compound(vec![
                ("LevelName", string("Mundo de prueba ñ")),
                ("hardcore", Tag::Byte(0)),
                ("Time", Tag::Long(123_456_789_012)),
                ("BorderSize", Tag::Double(59_999_968.0)),
                ("SpawnAngle", Tag::Float(-12.5)),
                ("version", Tag::Int(19133)),
                ("Short", Tag::Short(-3)),
                ("Bytes", Tag::ByteArray(vec![1, 2, 255])),
                ("Ints", Tag::IntArray(vec![-1, 0, 7])),
                ("Longs", Tag::LongArray(vec![i64::MIN, i64::MAX])),
                ("Empty", Tag::List(0, vec![])),
                (
                    "Datapacks",
                    Tag::List(8, vec![string("vanilla"), string("file/x.zip")]),
                ),
                (
                    "GameRules",
                    compound(vec![("doDaylightCycle", string("true"))]),
                ),
                (
                    "Players",
                    Tag::List(10, vec![compound(vec![("x", Tag::Int(1))])]),
                ),
            ]),
        )])
    }

    #[test]
    fn roundtrip_keeps_every_byte() {
        let root = sample();
        let raw = write(b"", &root);
        let (name, parsed) = read(&raw).unwrap();
        assert!(name.is_empty());
        assert_eq!(parsed, root);
        assert_eq!(write(&name, &parsed), raw);

        let gz = write_gzip(b"", &root).unwrap();
        assert_eq!(read_gzip(&gz).unwrap().1, root);
    }

    #[test]
    fn edits_compounds() {
        let mut root = sample();
        let data = root.get_mut("Data").unwrap();
        data.set("hardcore", Tag::Byte(1));
        data.set("difficulty_settings", compound(vec![]));
        assert_eq!(data.get("hardcore"), Some(&Tag::Byte(1)));
        assert!(data.get("difficulty_settings").is_some());
        assert_eq!(
            data.get("GameRules")
                .unwrap()
                .get("doDaylightCycle")
                .unwrap()
                .as_bool(),
            Some(true)
        );
        assert_eq!(string("verdad").as_bool(), None);
    }

    #[test]
    fn rejects_damaged_data() {
        let raw = write(b"", &sample());
        for cut in [0, 1, 5, raw.len() / 2, raw.len() - 1] {
            assert!(read(&raw[..cut]).is_err(), "cortado en {cut}");
        }
        // Una lista que dice tener dos mil millones de elementos.
        let mut huge = vec![10, 0, 0, 9, 0, 1, b'l', 1];
        huge.extend_from_slice(&i32::MAX.to_be_bytes());
        assert!(read(&huge).is_err());
        // Demasiado anidado.
        let mut deep = vec![10, 0, 0];
        for _ in 0..200 {
            deep.extend_from_slice(&[10, 0, 1, b'a']);
        }
        assert!(read(&deep).is_err());
        assert!(read_gzip(b"no es gzip").is_err());
    }
}
