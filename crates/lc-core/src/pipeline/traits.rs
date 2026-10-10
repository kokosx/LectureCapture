//! Interfaces implemented by platform backends (ScreenCaptureKit, Windows Graphics
//! Capture, WASAPI, cpal) and transcription engines (whisper.cpp). The recorder only
//! talks to these traits, so implementations can be swapped without touching the
//! pipeline – and tests use synthetic implementations.

use crate::audio::mixer::SourceKind;
use crate::frame::Frame;
use crate::session::manifest::SourceDescriptor;
use crate::transcript::RawSegment;
use anyhow::Result;
use crossbeam_channel::Sender;
use std::sync::atomic::AtomicBool;

#[derive(Debug)]
pub enum VideoEvent {
    /// A new frame with changed content (native resolution).
    Frame(Frame),
    /// Source reported "nothing changed" for this period.
    Idle,
    /// Source unavailable (window closed, display gone, permission revoked, sleep…).
    Lost(String),
    /// Source available again after `Lost`.
    Restored(String),
}

#[derive(Debug)]
pub enum AudioEvent {
    /// Mono samples at `rate` Hz.
    Samples { source: SourceKind, rate: u32, data: Vec<f32> },
    Lost { source: SourceKind, reason: String },
    Restored { source: SourceKind, detail: String },
}

/// Running capture; dropping or calling `stop` ends it.
pub trait CaptureHandle: Send {
    fn stop(&mut self);
}

pub trait VideoSource: Send {
    fn describe(&self) -> SourceDescriptor;
    /// Start delivering frames at roughly `fps` into `tx`. Implementations must never
    /// block on `tx` (use `try_send`) and should try to recover from transient loss
    /// themselves, reporting `Lost`/`Restored`.
    fn start(&mut self, fps: f32, tx: Sender<VideoEvent>) -> Result<Box<dyn CaptureHandle>>;
}

/// `SourceDescriptor::kind` of a lecture recorded without any picture.
pub const AUDIO_ONLY_KIND: &str = "audio";

/// Video source for audio-only lectures (in the lecture hall): delivers nothing and
/// needs no screen-recording permission.
pub struct NoVideo;

struct NoVideoHandle;
impl CaptureHandle for NoVideoHandle {
    fn stop(&mut self) {}
}

impl VideoSource for NoVideo {
    fn describe(&self) -> SourceDescriptor {
        SourceDescriptor { kind: AUDIO_ONLY_KIND.into(), id: None, title: None, app_name: None, width: None, height: None }
    }
    fn start(&mut self, _fps: f32, _tx: Sender<VideoEvent>) -> Result<Box<dyn CaptureHandle>> {
        // dropping `tx` closes the channel; the video thread then only serves control messages
        Ok(Box::new(NoVideoHandle))
    }
}

pub trait AudioSource: Send {
    fn kind(&self) -> SourceKind;
    fn describe(&self) -> String;
    fn start(&mut self, tx: Sender<AudioEvent>) -> Result<Box<dyn CaptureHandle>>;
}

#[derive(Debug, Default)]
pub struct TranscribeOutput {
    pub segments: Vec<RawSegment>,
    /// Detected / used language code.
    pub language: Option<String>,
}

pub trait Transcriber: Send {
    fn model_name(&self) -> String;
    fn engine_name(&self) -> String {
        "whisper.cpp".into()
    }
    /// Transcribe 16 kHz mono PCM. `cancel` is polled to abort long work.
    fn transcribe(&mut self, pcm: &[f32], cancel: &AtomicBool) -> Result<TranscribeOutput>;
}

/// Creates the transcriber lazily on the worker thread (model loading is slow).
pub type TranscriberFactory = Box<dyn FnOnce() -> Result<Box<dyn Transcriber>> + Send>;
