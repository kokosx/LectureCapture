//! LectureCapture core library.
//!
//! Everything in this crate is platform-independent and deterministic so it can be
//! unit- and integration-tested without screen/audio permissions:
//!
//! * [`detect`] – slide change detection on sampled frames,
//! * [`audio`] – resampling, mixing, level metering, VAD segmentation, Ogg/Opus, WAV,
//! * [`transcript`] – word-level merge of transcription chunks and slide assignment,
//! * [`session`] – on-disk lecture folder, manifest, journal, atomic writes, recovery,
//! * [`export`] – Markdown / PROMPT.md generation and ZIP export,
//! * [`pipeline`] – the recorder that wires capture backends, detector, audio and
//!   transcription queues together via the traits in [`pipeline::traits`].

pub mod audio;
pub mod clock;
pub mod config;
pub mod detect;
pub mod export;
pub mod frame;
pub mod pipeline;
pub mod session;
pub mod synth;
pub mod testing;
pub mod transcript;
pub mod util;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
