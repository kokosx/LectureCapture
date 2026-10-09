//! Snapshot of the recorder state for the UI (polled ~2×/s).

use super::transcription::TranscriptionStatusView;
use crate::audio::mixer::SourceStats;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecState {
    Recording,
    Paused,
    Stopping,
    Finished,
}

#[derive(Clone, Debug, Serialize)]
pub struct VideoStatus {
    /// `starting`, `ok`, `lost`
    pub state: String,
    pub detail: Option<String>,
    pub frames: u64,
    pub idle_ticks: u64,
    pub dropped_frames: u64,
    pub last_frame_ms: Option<u64>,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct AudioStatus {
    pub level_db: f32,
    pub peak: f32,
    pub sources: Vec<SourceStats>,
    pub silent_for_ms: u64,
    pub no_signal: bool,
    pub recorded_ms: u64,
    pub lost: Option<String>,
    pub speech_ratio: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Warning {
    pub t_ms: u64,
    /// `info`, `warning`, `error`
    pub level: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RecentSegment {
    pub start_ms: u64,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RecorderStatus {
    pub state: RecState,
    pub title: String,
    pub lecture_dir: String,
    pub elapsed_ms: u64,
    pub slides: usize,
    pub occurrences: usize,
    pub last_slide_id: Option<u32>,
    pub last_slide_path: Option<String>,
    pub current_slide_id: Option<u32>,
    pub video: VideoStatus,
    pub audio: AudioStatus,
    pub transcription: TranscriptionStatusView,
    pub lecture_bytes: u64,
    pub free_bytes: Option<u64>,
    pub warnings: Vec<Warning>,
    pub recent_segments: Vec<RecentSegment>,
}

impl RecorderStatus {
    pub fn new(title: &str, dir: &str) -> Self {
        Self {
            state: RecState::Recording,
            title: title.into(),
            lecture_dir: dir.into(),
            elapsed_ms: 0,
            slides: 0,
            occurrences: 0,
            last_slide_id: None,
            last_slide_path: None,
            current_slide_id: None,
            video: VideoStatus {
                state: "starting".into(),
                detail: None,
                frames: 0,
                idle_ticks: 0,
                dropped_frames: 0,
                last_frame_ms: None,
                width: 0,
                height: 0,
            },
            audio: AudioStatus {
                level_db: -120.0,
                peak: 0.0,
                sources: vec![],
                silent_for_ms: 0,
                no_signal: false,
                recorded_ms: 0,
                lost: None,
                speech_ratio: 0.0,
            },
            transcription: TranscriptionStatusView::default(),
            lecture_bytes: 0,
            free_bytes: None,
            warnings: vec![],
            recent_segments: vec![],
        }
    }

    pub fn warn(&mut self, t_ms: u64, level: &str, message: impl Into<String>) {
        let message = message.into();
        log::warn!("[{t_ms} ms] {level}: {message}");
        self.warnings.push(Warning { t_ms, level: level.into(), message });
        if self.warnings.len() > 50 {
            self.warnings.remove(0);
        }
    }
}
