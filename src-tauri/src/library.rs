//! Lecture library: scanning folders, summaries and detail views.
//!
//! Layout inside a library root:
//!
//! ```text
//! <root>/2026-10-09_Wyklad/                 lecture without a subject
//! <root>/Algorytmy i struktury danych/      subject (plain folder, no manifest.json)
//!        2026-10-09_Drzewa-binarne/         lecture of that subject
//! ```

use lc_core::session::fsutil::dir_size;
use lc_core::session::manifest::*;
use lc_core::session::LectureSession;
use lc_core::transcript::{Assignment, SegmentRecord};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
pub struct LectureSummary {
    pub path: String,
    pub title: String,
    pub folder: String,
    /// Subject (parent folder name) or `None` when stored directly in a root.
    pub subject: Option<String>,
    pub started_at: String,
    pub duration_ms: u64,
    pub slides: usize,
    pub status: LectureStatus,
    pub transcription: TranscriptionStatus,
    pub model: String,
    pub thumbnail: Option<String>,
    pub size_bytes: u64,
    pub has_audio: bool,
}

fn is_lecture(p: &Path) -> bool {
    p.join("manifest.json").is_file()
}

fn is_hidden(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

/// Subject folders directly inside `root` (any non-hidden folder that is not a lecture).
pub fn subject_dirs(root: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(root) else { return Vec::new() };
    let mut out: Vec<PathBuf> =
        rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() && !is_hidden(p) && !is_lecture(p)).collect();
    out.sort();
    out
}

/// All lecture folders: directly in the roots and one level down in subject folders.
pub fn lecture_dirs(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push_lectures = |dir: &Path| {
        if let Ok(rd) = std::fs::read_dir(dir) {
            out.extend(rd.flatten().map(|e| e.path()).filter(|p| is_lecture(p)));
        }
    };
    for root in roots {
        push_lectures(root);
        for sub in subject_dirs(root) {
            push_lectures(&sub);
        }
    }
    out
}

pub fn scan(roots: &[PathBuf]) -> Vec<LectureSummary> {
    let mut out = Vec::new();
    for p in lecture_dirs(roots) {
        match LectureSession::open(&p) {
            Ok(s) => out.push(summary(&s, roots)),
            Err(err) => log::warn!("skip {}: {err:#}", p.display()),
        }
    }
    out.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    out
}

/// Subject of a lecture folder: the name of its parent unless the parent is a root.
pub fn subject_of(lecture: &Path, roots: &[PathBuf]) -> Option<String> {
    let parent = lecture.parent()?;
    if roots.iter().any(|r| r == parent) {
        return None;
    }
    let grand = parent.parent()?;
    roots.iter().any(|r| r == grand).then(|| parent.file_name().map(|n| n.to_string_lossy().into_owned())).flatten()
}

#[derive(Clone, Debug, Serialize)]
pub struct SubjectInfo {
    pub name: String,
    pub path: String,
    pub lectures: usize,
}

/// Subjects of the default root (where new subjects are created) plus any other root.
pub fn subjects(roots: &[PathBuf]) -> Vec<SubjectInfo> {
    let mut out: Vec<SubjectInfo> = Vec::new();
    for root in roots {
        for d in subject_dirs(root) {
            let name = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let lectures = std::fs::read_dir(&d).map(|rd| rd.flatten().filter(|e| is_lecture(&e.path())).count()).unwrap_or(0);
            match out.iter_mut().find(|s| s.name == name) {
                Some(s) => s.lectures += lectures,
                None => out.push(SubjectInfo { name, path: d.to_string_lossy().into_owned(), lectures }),
            }
        }
    }
    out.sort_by_key(|s| s.name.to_lowercase());
    out
}

/// Validated subject folder name (kept human readable, only unsafe characters removed).
pub fn subject_name(input: &str) -> anyhow::Result<String> {
    let cleaned: String = input
        .trim()
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { '-' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim().to_string();
    let cleaned: String = cleaned.chars().take(80).collect();
    let cleaned = cleaned.trim_end_matches(['.', ' ']).to_string();
    if cleaned.is_empty() {
        anyhow::bail!("Podaj nazwę przedmiotu.");
    }
    Ok(cleaned)
}

/// Move a lecture folder into `dest_parent`, keeping its name unique there.
pub fn move_lecture(lecture: &Path, dest_parent: &Path) -> anyhow::Result<PathBuf> {
    use anyhow::Context;
    if lecture.parent() == Some(dest_parent) {
        return Ok(lecture.to_path_buf());
    }
    std::fs::create_dir_all(dest_parent).with_context(|| format!("create {}", dest_parent.display()))?;
    let name = lecture.file_name().context("invalid lecture path")?.to_string_lossy().into_owned();
    let mut dest = dest_parent.join(&name);
    let mut n = 2;
    while dest.exists() {
        dest = dest_parent.join(format!("{name}_{n}"));
        n += 1;
    }
    std::fs::rename(lecture, &dest).with_context(|| format!("move {} → {}", lecture.display(), dest.display()))?;
    let mut s = LectureSession::open(&dest)?;
    let folder = s.dir.folder_name();
    if s.manifest.lecture.folder != folder {
        s.manifest.lecture.folder = folder;
        s.save()?;
        lc_core::export::write_documents(&s)?;
    }
    Ok(dest)
}

pub fn summary(s: &LectureSession, roots: &[PathBuf]) -> LectureSummary {
    let m = &s.manifest;
    LectureSummary {
        path: s.dir.root.to_string_lossy().into_owned(),
        title: m.lecture.title.clone(),
        folder: m.lecture.folder.clone(),
        subject: subject_of(&s.dir.root, roots),
        started_at: m.lecture.started_at.to_rfc3339(),
        duration_ms: m.end_ms(),
        slides: m.slides.len(),
        status: m.lecture.status,
        transcription: m.transcription.status,
        model: m.transcription.model.clone(),
        thumbnail: m.slides.first().map(|sl| s.dir.abs(&sl.file).to_string_lossy().into_owned()),
        size_bytes: dir_size(&s.dir.root),
        has_audio: m.audio.file.as_ref().is_some_and(|f| s.dir.abs(f).exists()),
    }
}

#[derive(Serialize)]
pub struct LectureDetail {
    pub path: String,
    pub subject: Option<String>,
    pub manifest: Manifest,
    /// slide id → absolute image path
    pub slide_paths: std::collections::BTreeMap<u32, String>,
    pub records: Vec<SegmentRecord>,
    pub assignment: Assignment,
    pub pending_chunks: usize,
    pub failed_chunks: usize,
    pub size_bytes: u64,
    pub has_audio: bool,
    pub prompt_exists: bool,
}

fn count_wavs(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "wav")).count())
        .unwrap_or(0)
}

pub fn detail(path: &Path, roots: &[PathBuf]) -> anyhow::Result<LectureDetail> {
    let s = LectureSession::open(path)?;
    let data = lc_core::export::load_transcript(&s)?;
    let slide_paths = s
        .manifest
        .slides
        .iter()
        .map(|sl| (sl.id, s.dir.abs(&sl.file).to_string_lossy().into_owned()))
        .collect();
    Ok(LectureDetail {
        path: s.dir.root.to_string_lossy().into_owned(),
        subject: subject_of(&s.dir.root, roots),
        slide_paths,
        records: data.records,
        assignment: data.assignment,
        pending_chunks: count_wavs(&s.dir.pending_dir()),
        failed_chunks: count_wavs(&s.dir.pending_dir().join("failed")),
        size_bytes: dir_size(&s.dir.root),
        has_audio: s.manifest.audio.file.as_ref().is_some_and(|f| s.dir.abs(f).exists()),
        prompt_exists: s.dir.abs("PROMPT.md").exists(),
        manifest: s.manifest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_names_are_sanitized() {
        assert_eq!(subject_name("  Algorytmy i struktury danych ").unwrap(), "Algorytmy i struktury danych");
        assert_eq!(subject_name("Fizyka: mechanika/optyka").unwrap(), "Fizyka- mechanika-optyka");
        assert_eq!(subject_name("..ukryty").unwrap(), "ukryty");
        assert!(subject_name("   ").is_err());
    }

    #[test]
    fn finds_lectures_in_subject_folders() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().to_path_buf();
        let mk = |p: PathBuf| {
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("manifest.json"), "{}").unwrap();
        };
        mk(root.join("2026-10-01_A"));
        mk(root.join("Analiza").join("2026-10-02_B"));
        std::fs::create_dir_all(root.join("Puste")).unwrap();
        std::fs::create_dir_all(root.join(".lc")).unwrap();
        let roots = vec![root.clone()];
        let mut dirs = lecture_dirs(&roots);
        dirs.sort();
        assert_eq!(dirs.len(), 2);
        assert_eq!(subject_of(&root.join("2026-10-01_A"), &roots), None);
        assert_eq!(subject_of(&root.join("Analiza").join("2026-10-02_B"), &roots).as_deref(), Some("Analiza"));
        let subs = subjects(&roots);
        assert_eq!(subs.iter().map(|s| (s.name.as_str(), s.lectures)).collect::<Vec<_>>(), vec![("Analiza", 1), ("Puste", 0)]);
    }
}
