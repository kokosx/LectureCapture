//! Recording pipeline.
//!
//! ```text
//!  VideoSource ──frames──▶ [video thread] SlideDetector ──▶ [writer thread] PNG + manifest
//!  AudioSource(s) ─pcm──▶ [audio thread] resample → Mixer → Opus file
//!                                                   └─▶ VAD → .lc/pending/*.wav
//!                                [transcription thread] pending → Transcriber → segments.jsonl
//! ```
//!
//! Each stage has its own thread and queue. Video frames are dropped (never blocked)
//! when analysis falls behind; transcription runs at low priority from an on-disk
//! queue, so a slow model can never stall slide or audio capture, and nothing large
//! is held in RAM.

pub mod recorder;
pub mod status;
pub mod traits;
pub mod transcription;

pub use recorder::{Recorder, RecorderConfig};
pub use status::*;
pub use traits::*;
pub use transcription::{TranscriptionService, TranscriptionStatusView};
