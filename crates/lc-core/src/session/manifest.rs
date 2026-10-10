//! `manifest.json` – the machine-readable description of a lecture folder.
//!
//! Versioned (`schema` + `schema_version`) so other tools can rely on it. All paths are
//! relative to the lecture folder and use `/` separators; times are RFC 3339 with the
//! local UTC offset plus millisecond offsets from lecture start (`*_ms`).

use crate::audio::mixer::SourceKind;
use crate::config::{AudioRetention, DetectorConfig};
use crate::frame::NormRect;
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "lecturecapture/manifest";
pub const SCHEMA_VERSION: u32 = 1;

pub type Time = DateTime<FixedOffset>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: String,
    pub schema_version: u32,
    pub app: AppInfo,
    pub lecture: LectureInfo,
    pub capture: CaptureInfo,
    pub audio: AudioInfo,
    pub transcription: TranscriptionInfo,
    pub files: FilesInfo,
    pub slides: Vec<Slide>,
    /// Chronological list of slide display intervals ("occurrences").
    pub timeline: Vec<Occurrence>,
    /// Periods with missing or degraded data.
    pub gaps: Vec<Gap>,
    /// Full event log (also streamed to `.lc/events.jsonl` while recording).
    pub events: Vec<TimelineEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LectureStatus {
    Recording,
    Completed,
    /// Recording ended unexpectedly; data was recovered on next start.
    Recovered,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LectureInfo {
    pub id: String,
    pub title: String,
    /// Folder name (not an absolute path).
    pub folder: String,
    pub started_at: Time,
    pub ended_at: Option<Time>,
    pub duration_ms: Option<u64>,
    /// Last moment the recorder was known to be alive (crash recovery).
    pub last_alive_ms: u64,
    pub status: LectureStatus,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SourceDescriptor {
    /// `window`, `display` or `region`.
    pub kind: String,
    pub id: Option<String>,
    pub title: Option<String>,
    pub app_name: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptureInfo {
    pub source: SourceDescriptor,
    pub crop: Option<NormRect>,
    pub detector: DetectorConfig,
    pub image_format: String,
    pub webp_archive: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioInfo {
    /// `audio/recording.ogg`, or `null` when deleted after transcription.
    pub file: Option<String>,
    pub container: String,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub bitrate: u32,
    pub sources: Vec<SourceKind>,
    pub retention: AudioRetention,
    pub duration_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptionStatus {
    Disabled,
    Pending,
    Running,
    Completed,
    /// Some chunks failed; the rest is available.
    Partial,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranscriptionInfo {
    pub enabled: bool,
    pub engine: String,
    pub model: String,
    pub language: String,
    pub status: TranscriptionStatus,
    pub chunks_total: u64,
    pub chunks_done: u64,
    pub chunks_failed: u64,
    pub error: Option<String>,
    #[serde(default)]
    pub detected_languages: Vec<String>,
    pub completed_at: Option<Time>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FilesInfo {
    pub slides_dir: String,
    pub lecture_md: String,
    pub prompt_md: String,
    pub transcript_full_md: String,
    pub transcript_by_slide_md: String,
    pub transcript_segments_jsonl: String,
}

impl Default for FilesInfo {
    fn default() -> Self {
        Self {
            slides_dir: "slides".into(),
            lecture_md: "lecture.md".into(),
            prompt_md: "PROMPT.md".into(),
            transcript_full_md: "transcript/full.md".into(),
            transcript_by_slide_md: "transcript/by-slide.md".into(),
            transcript_segments_jsonl: "transcript/segments.jsonl".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlideTrigger {
    /// Detected automatically after a stable change.
    Auto,
    /// Content never stabilized; saved after the timeout.
    Unstable,
    /// Build step of the previous slide (reveal mode `separate`).
    Build,
    /// Manual capture (button / shortcut).
    Manual,
    /// Image found on disk after a crash without timeline information.
    Recovered,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Slide {
    /// Sequential slide number (also the file name).
    pub id: u32,
    pub file: String,
    pub archive_file: Option<String>,
    /// SHA-256 of the PNG file.
    pub sha256: String,
    /// 64-bit perceptual difference hash (hex) of the analysed region.
    pub dhash: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    /// When the image was taken (wall clock).
    pub captured_at: Time,
    /// When the image was taken (ms since lecture start).
    pub captured_ms: u64,
    pub trigger: SlideTrigger,
    pub build_of: Option<u32>,
    /// Number of build steps merged into this image (reveal mode `merge`).
    #[serde(default)]
    pub updates: u32,
    /// Derived: display intervals of this slide (ids into `timeline`).
    #[serde(default)]
    pub occurrences: Vec<String>,
    /// Derived: first time the slide appeared / last time it disappeared.
    pub display_start_ms: Option<u64>,
    pub display_end_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Occurrence {
    pub id: String,
    pub slide_id: u32,
    pub start_ms: u64,
    /// `None` while the slide is still on screen.
    pub end_ms: Option<u64>,
    pub start_at: Time,
    pub end_at: Option<Time>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapKind {
    Paused,
    VideoLost,
    AudioLost,
    NoAudioSignal,
    Crash,
    TranscriptionFailed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Gap {
    pub kind: GapKind,
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub t_ms: u64,
    pub at: Time,
    pub kind: String,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub detail: serde_json::Value,
}

impl Manifest {
    /// Recorded without picture (lecture hall mode, microphone only).
    pub fn is_audio_only(&self) -> bool {
        self.capture.source.kind == crate::pipeline::AUDIO_ONLY_KIND
    }

    pub fn slide(&self, id: u32) -> Option<&Slide> {
        self.slides.iter().find(|s| s.id == id)
    }

    pub fn end_ms(&self) -> u64 {
        self.lecture.duration_ms.unwrap_or(self.lecture.last_alive_ms)
    }

    /// Recompute fields derived from the timeline.
    pub fn refresh_derived(&mut self) {
        for s in &mut self.slides {
            let occ: Vec<&Occurrence> = self.timeline.iter().filter(|o| o.slide_id == s.id).collect();
            s.occurrences = occ.iter().map(|o| o.id.clone()).collect();
            s.display_start_ms = occ.iter().map(|o| o.start_ms).min();
            s.display_end_ms = occ.iter().filter_map(|o| o.end_ms).max();
        }
    }

    /// Timeline intervals with an end (open interval closed at `end_ms`).
    pub fn closed_timeline(&self) -> Vec<(String, u32, u64, u64)> {
        let end = self.end_ms();
        self.timeline
            .iter()
            .map(|o| (o.id.clone(), o.slide_id, o.start_ms, o.end_ms.unwrap_or(end).max(o.start_ms)))
            .collect()
    }
}
