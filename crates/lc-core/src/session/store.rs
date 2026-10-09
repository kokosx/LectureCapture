//! In-memory lecture state persisted continuously to the lecture folder.

use super::fsutil::{append_line, atomic_write};
use super::layout::LectureDir;
use super::manifest::*;
use anyhow::{bail, Context, Result};
use chrono::Duration;
use std::path::Path;

pub struct NewLecture {
    pub title: String,
    pub capture: CaptureInfo,
    pub audio: AudioInfo,
    pub transcription: TranscriptionInfo,
}

pub struct LectureSession {
    pub dir: LectureDir,
    pub manifest: Manifest,
    last_save_alive_ms: u64,
}

impl LectureSession {
    pub fn create(parent: &Path, spec: NewLecture, started_at: Time) -> Result<Self> {
        let dir = LectureDir::create_new(parent, &spec.title, &started_at)?;
        std::fs::create_dir_all(dir.internal_dir())?;
        atomic_write(&dir.lock_path(), std::process::id().to_string().as_bytes())?;
        let manifest = Manifest {
            schema: SCHEMA.into(),
            schema_version: SCHEMA_VERSION,
            app: AppInfo { name: "LectureCapture".into(), version: crate::APP_VERSION.into() },
            lecture: LectureInfo {
                id: uuid::Uuid::new_v4().to_string(),
                title: spec.title.clone(),
                folder: dir.folder_name(),
                started_at,
                ended_at: None,
                duration_ms: None,
                last_alive_ms: 0,
                status: LectureStatus::Recording,
                notes: Vec::new(),
            },
            capture: spec.capture,
            audio: spec.audio,
            transcription: spec.transcription,
            files: FilesInfo::default(),
            slides: Vec::new(),
            timeline: Vec::new(),
            gaps: Vec::new(),
            events: Vec::new(),
        };
        let mut s = Self { dir, manifest, last_save_alive_ms: 0 };
        s.event(0, "lecture_started", serde_json::json!({ "title": spec.title }));
        s.save()?;
        Ok(s)
    }

    pub fn open(root: impl Into<std::path::PathBuf>) -> Result<Self> {
        let dir = LectureDir::open(root);
        let text = std::fs::read_to_string(dir.manifest_path())
            .with_context(|| format!("read {}", dir.manifest_path().display()))?;
        let manifest: Manifest = serde_json::from_str(&text).context("parse manifest.json")?;
        if manifest.schema_version > SCHEMA_VERSION {
            bail!("manifest schema {} is newer than supported {}", manifest.schema_version, SCHEMA_VERSION);
        }
        let alive = manifest.lecture.last_alive_ms;
        Ok(Self { dir, manifest, last_save_alive_ms: alive })
    }

    pub fn at(&self, ms: u64) -> Time {
        self.manifest.lecture.started_at + Duration::milliseconds(ms as i64)
    }

    /// Record an event in the manifest and append it to the on-disk journal.
    pub fn event(&mut self, t_ms: u64, kind: &str, detail: serde_json::Value) {
        let ev = TimelineEvent { t_ms, at: self.at(t_ms), kind: kind.into(), detail };
        if let Ok(line) = serde_json::to_string(&ev) {
            if let Err(e) = append_line(&self.dir.events_path(), &line) {
                log::warn!("journal append failed: {e}");
            }
        }
        self.manifest.events.push(ev);
        self.manifest.lecture.last_alive_ms = self.manifest.lecture.last_alive_ms.max(t_ms);
    }

    pub fn save(&mut self) -> Result<()> {
        self.manifest.refresh_derived();
        let json = serde_json::to_vec_pretty(&self.manifest)?;
        atomic_write(&self.dir.manifest_path(), &json)?;
        self.last_save_alive_ms = self.manifest.lecture.last_alive_ms;
        Ok(())
    }

    /// Periodic liveness update (persisted every ~15 s).
    pub fn heartbeat(&mut self, now_ms: u64) -> Result<()> {
        self.manifest.lecture.last_alive_ms = self.manifest.lecture.last_alive_ms.max(now_ms);
        if now_ms.saturating_sub(self.last_save_alive_ms) >= 15_000 {
            self.save()?;
        }
        Ok(())
    }

    pub fn next_slide_id(&self) -> u32 {
        self.manifest.slides.iter().map(|s| s.id).max().unwrap_or(0) + 1
    }

    pub fn add_slide(&mut self, slide: Slide) -> Result<()> {
        let t = slide.captured_ms;
        let detail = serde_json::json!({ "slide_id": slide.id, "file": slide.file, "trigger": slide.trigger });
        self.manifest.slides.push(slide);
        self.event(t, "slide_saved", detail);
        self.save()
    }

    pub fn update_slide(&mut self, id: u32, t_ms: u64, f: impl FnOnce(&mut Slide)) -> Result<()> {
        let Some(s) = self.manifest.slides.iter_mut().find(|s| s.id == id) else {
            bail!("slide {id} not found");
        };
        f(s);
        self.event(t_ms, "slide_updated", serde_json::json!({ "slide_id": id }));
        self.save()
    }

    pub fn current_occurrence(&self) -> Option<&Occurrence> {
        self.manifest.timeline.last().filter(|o| o.end_ms.is_none())
    }

    fn next_occurrence_id(&self) -> String {
        let n = self
            .manifest
            .timeline
            .iter()
            .filter_map(|o| o.id.strip_prefix("occ-").and_then(|n| n.parse::<u32>().ok()))
            .max()
            .unwrap_or(0)
            + 1;
        format!("occ-{n:04}")
    }

    /// The given slide becomes visible at `at_ms` (closes the previous occurrence).
    pub fn start_occurrence(&mut self, slide_id: u32, at_ms: u64) -> Result<String> {
        let mut at_ms = at_ms;
        if let Some(cur) = self.current_occurrence() {
            if cur.slide_id == slide_id {
                return Ok(cur.id.clone());
            }
            at_ms = at_ms.max(cur.start_ms);
        }
        self.end_occurrence(at_ms);
        let id = self.next_occurrence_id();
        self.manifest.timeline.push(Occurrence {
            id: id.clone(),
            slide_id,
            start_ms: at_ms,
            end_ms: None,
            start_at: self.at(at_ms),
            end_at: None,
        });
        self.event(at_ms, "slide_shown", serde_json::json!({ "slide_id": slide_id, "occurrence": id }));
        self.save()?;
        Ok(id)
    }

    /// Nothing is shown any more (pause, source lost, stop).
    pub fn end_occurrence(&mut self, at_ms: u64) {
        let at = self.at(at_ms);
        if let Some(o) = self.manifest.timeline.last_mut() {
            if o.end_ms.is_none() {
                o.end_ms = Some(at_ms.max(o.start_ms));
                o.end_at = Some(at);
            }
        }
    }

    pub fn open_gap(&mut self, kind: GapKind, at_ms: u64, detail: Option<String>) -> Result<()> {
        if self.manifest.gaps.iter().any(|g| g.kind == kind && g.end_ms.is_none()) {
            return Ok(());
        }
        self.manifest.gaps.push(Gap { kind, start_ms: at_ms, end_ms: None, detail: detail.clone() });
        self.event(at_ms, "gap_started", serde_json::json!({ "kind": kind, "detail": detail }));
        self.save()
    }

    pub fn close_gap(&mut self, kind: GapKind, at_ms: u64) -> Result<()> {
        let mut closed = false;
        for g in self.manifest.gaps.iter_mut().filter(|g| g.kind == kind && g.end_ms.is_none()) {
            g.end_ms = Some(at_ms.max(g.start_ms));
            closed = true;
        }
        if closed {
            self.event(at_ms, "gap_ended", serde_json::json!({ "kind": kind }));
            self.save()?;
        }
        Ok(())
    }

    pub fn finish(&mut self, end_ms: u64, status: LectureStatus) -> Result<()> {
        self.end_occurrence(end_ms);
        for g in self.manifest.gaps.iter_mut().filter(|g| g.end_ms.is_none()) {
            g.end_ms = Some(end_ms.max(g.start_ms));
        }
        self.manifest.lecture.ended_at = Some(self.at(end_ms));
        self.manifest.lecture.duration_ms = Some(end_ms);
        self.manifest.lecture.status = status;
        self.event(end_ms, "lecture_stopped", serde_json::json!({ "status": status }));
        self.save()?;
        let _ = std::fs::remove_file(self.dir.lock_path());
        Ok(())
    }

    // ----- post-recording edits ------------------------------------------------

    /// Delete a wrong/duplicate slide. Its display time is merged into the previous
    /// occurrence (or the next one if it was first), so no transcript is orphaned.
    pub fn delete_slide(&mut self, id: u32) -> Result<()> {
        let Some(idx) = self.manifest.slides.iter().position(|s| s.id == id) else {
            bail!("slide {id} not found");
        };
        let slide = self.manifest.slides.remove(idx);
        let tl = std::mem::take(&mut self.manifest.timeline);
        let mut out: Vec<Occurrence> = Vec::with_capacity(tl.len());
        let mut carry_start: Option<(u64, Time)> = None;
        for o in tl {
            if o.slide_id == id {
                if let Some(prev) = out.last_mut() {
                    prev.end_ms = o.end_ms;
                    prev.end_at = o.end_at;
                } else {
                    carry_start = Some((o.start_ms, o.start_at));
                }
                continue;
            }
            let mut o = o;
            if let Some((s, at)) = carry_start.take() {
                o.start_ms = s;
                o.start_at = at;
            }
            // merge consecutive occurrences of the same slide created by the deletion
            if let Some(prev) = out.last_mut() {
                if prev.slide_id == o.slide_id {
                    prev.end_ms = o.end_ms;
                    prev.end_at = o.end_at;
                    continue;
                }
            }
            out.push(o);
        }
        self.manifest.timeline = out;
        for rel in std::iter::once(&slide.file).chain(slide.archive_file.iter()) {
            let p = self.dir.abs(rel);
            if p.exists() {
                std::fs::remove_file(&p).with_context(|| format!("delete {}", p.display()))?;
            }
        }
        let t = self.manifest.end_ms();
        self.event(t, "slide_deleted", serde_json::json!({ "slide_id": id }));
        self.save()
    }

    /// Move the boundary between an occurrence and its predecessor.
    pub fn set_occurrence_start(&mut self, occ_id: &str, new_start_ms: u64) -> Result<()> {
        let Some(i) = self.manifest.timeline.iter().position(|o| o.id == occ_id) else {
            bail!("occurrence {occ_id} not found");
        };
        let end = self.manifest.end_ms();
        let upper = self.manifest.timeline[i].end_ms.unwrap_or(end);
        let lower = if i > 0 { self.manifest.timeline[i - 1].start_ms } else { 0 };
        if new_start_ms <= lower && i > 0 || new_start_ms >= upper {
            bail!("new start must lie between {lower} ms and {upper} ms");
        }
        let at = self.at(new_start_ms);
        self.manifest.timeline[i].start_ms = new_start_ms;
        self.manifest.timeline[i].start_at = at;
        if i > 0 {
            self.manifest.timeline[i - 1].end_ms = Some(new_start_ms);
            self.manifest.timeline[i - 1].end_at = Some(at);
        }
        self.event(end, "boundary_moved", serde_json::json!({ "occurrence": occ_id, "start_ms": new_start_ms }));
        self.save()
    }
}
