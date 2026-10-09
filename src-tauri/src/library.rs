//! Lecture library: scanning folders, summaries and detail views.

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

pub fn scan(roots: &[PathBuf]) -> Vec<LectureSummary> {
    let mut out = Vec::new();
    for root in roots {
        let Ok(rd) = std::fs::read_dir(root) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if !p.join("manifest.json").is_file() {
                continue;
            }
            match LectureSession::open(&p) {
                Ok(s) => out.push(summary(&s)),
                Err(err) => log::warn!("skip {}: {err:#}", p.display()),
            }
        }
    }
    out.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    out
}

pub fn summary(s: &LectureSession) -> LectureSummary {
    let m = &s.manifest;
    LectureSummary {
        path: s.dir.root.to_string_lossy().into_owned(),
        title: m.lecture.title.clone(),
        folder: m.lecture.folder.clone(),
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

pub fn detail(path: &Path) -> anyhow::Result<LectureDetail> {
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
