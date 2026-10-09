//! Whisper model catalogue, download with integrity check, local storage.
//!
//! Models come from the official whisper.cpp repository on Hugging Face
//! (`ggerganov/whisper.cpp`, MIT licence). Every download is verified against the
//! SHA-256 published by Hugging Face's LFS metadata (pinned below) before use.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, Serialize)]
pub struct ModelInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub file: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    pub default: bool,
}

const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";

pub const CATALOG: &[ModelInfo] = &[
    ModelInfo {
        id: "tiny",
        label: "Whisper Tiny (multilingual)",
        description: "Najszybszy, najmniej dokładny. ~75 MB.",
        file: "ggml-tiny.bin",
        size: 77_691_713,
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
        default: false,
    },
    ModelInfo {
        id: "base",
        label: "Whisper Base (multilingual)",
        description: "Domyślny – dobry kompromis szybkości i jakości. ~148 MB.",
        file: "ggml-base.bin",
        size: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        default: true,
    },
    ModelInfo {
        id: "small",
        label: "Whisper Small (multilingual)",
        description: "Dokładniejszy, wolniejszy. ~488 MB.",
        file: "ggml-small.bin",
        size: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        default: false,
    },
    ModelInfo {
        id: "large-v3-turbo-q5",
        label: "Whisper Large v3 Turbo (q5_0)",
        description: "Najlepsza jakość dla języka polskiego; wymaga szybkiego komputera (Apple Silicon). ~574 MB.",
        file: "ggml-large-v3-turbo-q5_0.bin",
        size: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        default: false,
    },
];

pub fn find(id: &str) -> Option<&'static ModelInfo> {
    CATALOG.iter().find(|m| m.id == id)
}

#[derive(Clone, Debug, Serialize)]
pub struct ModelStatus {
    pub info: ModelInfo,
    pub installed: bool,
    pub verified: bool,
    pub path: Option<String>,
}

pub struct ModelManager {
    dir: PathBuf,
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

impl ModelManager {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path_of(&self, m: &ModelInfo) -> PathBuf {
        self.dir.join(m.file)
    }

    fn marker(&self, m: &ModelInfo) -> PathBuf {
        self.dir.join(format!("{}.verified", m.file))
    }

    pub fn status(&self) -> Vec<ModelStatus> {
        CATALOG
            .iter()
            .map(|m| {
                let p = self.path_of(m);
                let installed = std::fs::metadata(&p).map(|md| md.len() == m.size).unwrap_or(false);
                let verified = installed
                    && std::fs::read_to_string(self.marker(m)).map(|s| s.trim() == m.sha256).unwrap_or(false);
                ModelStatus {
                    info: m.clone(),
                    installed,
                    verified,
                    path: installed.then(|| p.to_string_lossy().into_owned()),
                }
            })
            .collect()
    }

    /// Path of an installed and verified model (verifies once if the marker is missing).
    pub fn ready_path(&self, id: &str) -> Result<PathBuf> {
        let m = find(id).with_context(|| format!("nieznany model „{id}”"))?;
        let p = self.path_of(m);
        if !p.exists() {
            bail!("Model {} nie jest pobrany", m.label);
        }
        if std::fs::read_to_string(self.marker(m)).map(|s| s.trim() == m.sha256).unwrap_or(false) {
            return Ok(p);
        }
        let got = sha256_file(&p)?;
        if got != m.sha256 {
            bail!("Plik modelu {} jest uszkodzony (SHA-256 się nie zgadza) – pobierz go ponownie", m.file);
        }
        std::fs::write(self.marker(m), m.sha256)?;
        Ok(p)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let m = find(id).context("nieznany model")?;
        let _ = std::fs::remove_file(self.marker(m));
        let p = self.path_of(m);
        if p.exists() {
            std::fs::remove_file(p)?;
        }
        Ok(())
    }

    /// Download (resumable is not needed for < 600 MB) with streaming SHA-256 check.
    /// `progress(downloaded, total)`.
    pub fn download(&self, id: &str, cancel: &AtomicBool, mut progress: impl FnMut(u64, u64)) -> Result<PathBuf> {
        let m = find(id).with_context(|| format!("nieznany model „{id}”"))?;
        std::fs::create_dir_all(&self.dir)?;
        let url = format!("{BASE_URL}{}", m.file);
        let resp = ureq::get(&url)
            .set("User-Agent", concat!("LectureCapture/", env!("CARGO_PKG_VERSION")))
            .call()
            .with_context(|| format!("pobieranie {url}"))?;
        let total = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(m.size);
        let part = self.dir.join(format!("{}.part", m.file));
        let mut out = std::io::BufWriter::new(std::fs::File::create(&part)?);
        let mut reader = resp.into_reader();
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 256 * 1024];
        let mut done = 0u64;
        loop {
            if cancel.load(Ordering::SeqCst) {
                drop(out);
                let _ = std::fs::remove_file(&part);
                bail!("pobieranie anulowane");
            }
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
            out.write_all(&buf[..n])?;
            done += n as u64;
            progress(done, total);
        }
        out.flush()?;
        out.get_ref().sync_all()?;
        drop(out);
        let got = hex::encode(h.finalize());
        if got != m.sha256 || done != m.size {
            let _ = std::fs::remove_file(&part);
            bail!("weryfikacja integralności nie powiodła się (SHA-256 {got}, {done} B) – plik odrzucony");
        }
        let dest = self.path_of(m);
        std::fs::rename(&part, &dest)?;
        std::fs::write(self.marker(m), m.sha256)?;
        Ok(dest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_is_consistent() {
        assert_eq!(CATALOG.iter().filter(|m| m.default).count(), 1);
        for m in CATALOG {
            assert_eq!(m.sha256.len(), 64);
            assert!(m.file.starts_with("ggml-"));
        }
    }

    #[test]
    fn corrupt_model_is_rejected() {
        let d = tempfile::tempdir().unwrap();
        let mm = ModelManager::new(d.path());
        std::fs::write(d.path().join("ggml-tiny.bin"), b"not a model").unwrap();
        assert!(mm.ready_path("tiny").is_err());
        assert!(!mm.status()[0].installed, "size mismatch → not installed");
    }
}
