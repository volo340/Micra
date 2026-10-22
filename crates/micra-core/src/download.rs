use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use tokio::io::AsyncWriteExt;

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgo {
    Sha1,
    Sha256,
    Sha512,
}

#[derive(Debug, Clone)]
pub struct ExpectedHash {
    pub algo: HashAlgo,
    pub hex: String,
}

impl ExpectedHash {
    pub fn new(algo: HashAlgo, hex: impl Into<String>) -> Self {
        Self { algo, hex: hex.into().to_ascii_lowercase() }
    }
}

/// Callback de progreso: (bytes descargados, total si el servidor lo informa).
pub type Progress<'a> = &'a (dyn Fn(u64, Option<u64>) + Send + Sync);

enum Hasher {
    Sha1(Sha1),
    Sha256(Sha256),
    Sha512(Sha512),
}

impl Hasher {
    fn new(algo: HashAlgo) -> Self {
        match algo {
            HashAlgo::Sha1 => Self::Sha1(Sha1::new()),
            HashAlgo::Sha256 => Self::Sha256(Sha256::new()),
            HashAlgo::Sha512 => Self::Sha512(Sha512::new()),
        }
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Self::Sha1(h) => h.update(data),
            Self::Sha256(h) => h.update(data),
            Self::Sha512(h) => h.update(data),
        }
    }

    fn finish(self) -> String {
        match self {
            Self::Sha1(h) => hex::encode(h.finalize()),
            Self::Sha256(h) => hex::encode(h.finalize()),
            Self::Sha512(h) => hex::encode(h.finalize()),
        }
    }
}

/// Descarga `url` a `dest` en streaming, verificando el hash si se indica.
///
/// Escribe primero en `<dest>.part` y solo renombra al final, así un corte
/// de red nunca deja un archivo a medias con el nombre definitivo.
/// Si `dest` ya existe y su hash coincide, no se vuelve a descargar.
pub async fn download_file(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expected: Option<&ExpectedHash>,
    progress: Option<Progress<'_>>,
) -> Result<()> {
    if let Some(expected) = expected {
        if dest.exists() && hash_file(dest, expected.algo).await? == expected.hex {
            return Ok(());
        }
    }
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let tmp = part_path(dest);
    let response = client.get(url).send().await?.error_for_status()?;
    let total = response.content_length();
    let mut file = tokio::fs::File::create(&tmp).await?;
    let mut hasher = expected.map(|e| Hasher::new(e.algo));
    let mut downloaded = 0u64;
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if let Some(h) = hasher.as_mut() {
            h.update(&chunk);
        }
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if let Some(cb) = progress {
            cb(downloaded, total);
        }
    }
    file.flush().await?;
    drop(file);

    if let (Some(expected), Some(hasher)) = (expected, hasher) {
        let actual = hasher.finish();
        if actual != expected.hex {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(Error::HashMismatch {
                file: dest.display().to_string(),
                expected: expected.hex.clone(),
                actual,
            });
        }
    }
    tokio::fs::rename(&tmp, dest).await?;
    Ok(())
}

pub async fn hash_file(path: &Path, algo: HashAlgo) -> Result<String> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut file = std::fs::File::open(path)?;
        let mut hasher = Hasher::new(algo);
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hasher.finish())
    })
    .await
    .expect("la tarea de hash no debería entrar en pánico")
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}
