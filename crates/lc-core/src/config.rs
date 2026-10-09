//! User-configurable settings with sensible defaults. Every threshold used by the
//! detector and the audio pipeline lives here so it can be tuned from the UI and is
//! recorded in `manifest.json` for reproducibility.

use crate::frame::NormRect;
use serde::{Deserialize, Serialize};

/// How incremental "build" animations (bullet points appearing one by one) are handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevealMode {
    /// Keep one slide and overwrite its image with the most complete state (default).
    Merge,
    /// Save every build step as a separate slide (linked via `build_of`).
    Separate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DetectorConfig {
    /// Sampling rate of the screen (frames per second). 0.5–4 are sensible.
    pub sample_fps: f32,
    /// Width (px) of the down-scaled grayscale image used only for analysis.
    pub analysis_width: u32,
    /// Per-pixel luma difference (0–255) above which a pixel counts as changed.
    /// High enough to ignore video-compression noise in Teams screen sharing.
    pub pixel_threshold: u8,
    /// Search radius (analysis px) for shift-tolerant comparison – absorbs 1–2 px
    /// jitter / re-scaling of the shared screen.
    pub shift_tolerance: u32,
    /// Block size (analysis px) of the change grid.
    pub block_size: u32,
    /// Fraction of changed pixels for a block to count as changed.
    pub block_change_ratio: f32,
    /// Changes whose connected blocks fit in an N×N box are treated as cursor /
    /// tiny UI noise and ignored.
    pub cursor_max_blocks: u32,
    /// More than this many small isolated changes at once *is* significant.
    pub max_small_components: u32,
    /// Consecutive identical samples required before a new slide is accepted.
    pub stable_frames: u32,
    /// If content never stabilizes (video in slide, animation), save anyway after this.
    pub max_unstable_ms: u64,
    /// Minimum time between two committed slide changes.
    pub min_change_interval_ms: u64,
    /// Max dHash distance (of 64 bits) for two slides to be considered candidates
    /// for deduplication (confirmed with a pixel comparison).
    pub dedupe_hash_distance: u32,
    pub reveal_mode: RevealMode,
    /// Ignore uniform (blank/black) frames, e.g. when screen sharing stops.
    pub ignore_blank: bool,
    /// Regions (normalized, relative to the captured region) excluded from analysis.
    pub masks: Vec<NormRect>,
    /// Ignore regions with continuous motion (lecturer's webcam, video): they are
    /// masked automatically and never trigger new slides on their own.
    pub motion_filter: bool,
    /// A moving region stays masked until it has been still for this long.
    pub motion_hold_ms: u64,
    /// If at least this fraction of the image is moving, the frame is treated as
    /// live video (camera only, no slide) and nothing is saved.
    pub live_video_fraction: f32,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            sample_fps: 2.0,
            analysis_width: 320,
            pixel_threshold: 28,
            shift_tolerance: 1,
            block_size: 16,
            block_change_ratio: 0.03,
            cursor_max_blocks: 2,
            max_small_components: 3,
            stable_frames: 3,
            max_unstable_ms: 15_000,
            min_change_interval_ms: 800,
            dedupe_hash_distance: 10,
            reveal_mode: RevealMode::Merge,
            ignore_blank: true,
            masks: Vec::new(),
            motion_filter: true,
            motion_hold_ms: 4_000,
            live_video_fraction: 0.6,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioRetention {
    /// Keep `audio/recording.ogg` (Opus) permanently.
    Keep,
    /// Delete the recording once the transcription finished successfully.
    DeleteAfterTranscription,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VadConfig {
    pub frame_ms: u32,
    /// Speech if frame energy exceeds the adaptive noise floor by this many dB…
    pub threshold_above_floor_db: f32,
    /// …and is above this absolute level (dBFS).
    pub absolute_threshold_db: f32,
    pub min_speech_ms: u32,
    /// Silence needed to close a speech chunk.
    pub hangover_ms: u32,
    pub preroll_ms: u32,
    /// Chunks are force-split at the quietest point near this length.
    pub max_chunk_ms: u32,
    pub min_chunk_ms: u32,
    /// Audio overlap between force-split chunks (dedup happens on word level).
    pub overlap_ms: u32,
    pub cut_search_ms: u32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            frame_ms: 20,
            threshold_above_floor_db: 9.0,
            absolute_threshold_db: -52.0,
            min_speech_ms: 200,
            hangover_ms: 800,
            preroll_ms: 300,
            max_chunk_ms: 28_000,
            min_chunk_ms: 600,
            overlap_ms: 1_000,
            cut_search_ms: 5_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    pub capture_system: bool,
    pub capture_microphone: bool,
    /// Optional microphone device name (None = system default).
    pub microphone_device: Option<String>,
    /// Windows only: name of the output device used for WASAPI loopback.
    pub loopback_device: Option<String>,
    /// macOS only: restrict system audio to one application (bundle id), e.g. Teams.
    pub only_application: Option<String>,
    pub microphone_gain: f32,
    pub opus_bitrate: u32,
    pub retention: AudioRetention,
    /// Warn when the mixed signal stays below this level for `silence_warn_ms`.
    pub silence_threshold_db: f32,
    pub silence_warn_ms: u64,
    pub vad: VadConfig,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            capture_system: true,
            capture_microphone: false,
            microphone_device: None,
            loopback_device: None,
            only_application: None,
            microphone_gain: 1.0,
            opus_bitrate: 32_000,
            retention: AudioRetention::Keep,
            silence_threshold_db: -60.0,
            silence_warn_ms: 30_000,
            vad: VadConfig::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TranscriptionConfig {
    pub enabled: bool,
    /// Model id from the model catalogue (`tiny`, `base`, `small`, `large-v3-turbo-q5`).
    pub model: String,
    /// ISO code (`pl`, `en`) or `auto`.
    pub language: String,
    /// CPU threads for whisper.cpp (GPU/Metal is used when available).
    pub threads: u32,
    /// Transcribe while recording (otherwise only after Stop).
    pub live: bool,
    pub beam_size: u32,
    /// Optional domain prompt (course name, terminology) to bias recognition.
    pub initial_prompt: Option<String>,
}

impl Default for TranscriptionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model: "base".into(),
            language: "pl".into(),
            threads: 4,
            live: true,
            beam_size: 1,
            initial_prompt: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    #[default]
    Png,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputConfig {
    /// Additionally store a WebP-lossless archive copy next to every PNG.
    pub webp_archive: bool,
    /// zlib level for PNG (lossless at every level; higher = smaller & slower).
    pub png_compression: PngCompression,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PngCompression {
    Fast,
    #[default]
    Balanced,
    Best,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self { webp_archive: false, png_compression: PngCompression::Balanced }
    }
}
