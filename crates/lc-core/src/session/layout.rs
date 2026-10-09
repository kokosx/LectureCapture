//! Folder layout of a lecture:
//!
//! ```text
//! 2026-10-09_Algorytmy-i-struktury-danych/
//!   slides/001.png …
//!   audio/recording.ogg
//!   transcript/full.md, segments.jsonl, by-slide.md
//!   manifest.json, lecture.md, PROMPT.md
//!   .lc/            internal: events.jsonl, recording.lock, pending/ (transcription queue)
//! ```

use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct LectureDir {
    pub root: PathBuf,
}

impl LectureDir {
    pub fn open(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Create a fresh, uniquely named lecture folder inside `parent`.
    pub fn create_new<Tz: TimeZone>(parent: &Path, title: &str, started: &DateTime<Tz>) -> Result<Self>
    where
        Tz::Offset: std::fmt::Display,
    {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        let base = format!("{}_{}", started.format("%Y-%m-%d"), crate::util::slugify(title, 60));
        let mut name = base.clone();
        let mut n = 2;
        while parent.join(&name).exists() {
            name = format!("{base}_{n}");
            n += 1;
        }
        let dir = Self { root: parent.join(name) };
        for d in [dir.slides_dir(), dir.audio_dir(), dir.transcript_dir(), dir.pending_dir()] {
            std::fs::create_dir_all(&d).with_context(|| format!("create {}", d.display()))?;
        }
        Ok(dir)
    }

    pub fn folder_name(&self) -> String {
        self.root.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }

    pub fn abs(&self, rel: &str) -> PathBuf {
        rel.split('/').fold(self.root.clone(), |p, part| p.join(part))
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.root.join("manifest.json")
    }
    pub fn internal_dir(&self) -> PathBuf {
        self.root.join(".lc")
    }
    pub fn events_path(&self) -> PathBuf {
        self.internal_dir().join("events.jsonl")
    }
    pub fn lock_path(&self) -> PathBuf {
        self.internal_dir().join("recording.lock")
    }
    pub fn pending_dir(&self) -> PathBuf {
        self.internal_dir().join("pending")
    }
    pub fn slides_dir(&self) -> PathBuf {
        self.root.join("slides")
    }
    pub fn audio_dir(&self) -> PathBuf {
        self.root.join("audio")
    }
    pub fn transcript_dir(&self) -> PathBuf {
        self.root.join("transcript")
    }
    pub fn recording_rel() -> &'static str {
        "audio/recording.ogg"
    }
    pub fn recording_path(&self) -> PathBuf {
        self.abs(Self::recording_rel())
    }
    pub fn segments_path(&self) -> PathBuf {
        self.abs("transcript/segments.jsonl")
    }

    pub fn slide_rel(id: u32) -> String {
        format!("slides/{id:03}.png")
    }
    pub fn slide_archive_rel(id: u32) -> String {
        format!("slides/archive/{id:03}.webp")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_folder_names() {
        let d = tempfile::tempdir().unwrap();
        let t = chrono::Local.with_ymd_and_hms(2026, 10, 9, 10, 0, 0).unwrap();
        let a = LectureDir::create_new(d.path(), "Algorytmy i struktury danych", &t).unwrap();
        let b = LectureDir::create_new(d.path(), "Algorytmy i struktury danych", &t).unwrap();
        assert_eq!(a.folder_name(), "2026-10-09_Algorytmy-i-struktury-danych");
        assert_eq!(b.folder_name(), "2026-10-09_Algorytmy-i-struktury-danych_2");
        assert!(a.slides_dir().is_dir() && a.pending_dir().is_dir());
        assert_eq!(LectureDir::slide_rel(7), "slides/007.png");
        assert_eq!(LectureDir::slide_rel(1234), "slides/1234.png");
    }
}
