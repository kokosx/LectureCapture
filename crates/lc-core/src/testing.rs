//! Synthetic capture sources and a deterministic driver for pipeline tests.
//! These stand in for ScreenCaptureKit / WASAPI / whisper.cpp in automated tests only
//! and are never used by the application's recording path.

use anyhow::Result;
use crossbeam_channel::Sender;
use crate::audio::mixer::SourceKind;
use crate::clock::{Clock, ManualClock};
use crate::frame::Frame;
use crate::pipeline::*;
use crate::session::manifest::SourceDescriptor;
use crate::transcript::{RawSegment, RawToken};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub struct NoopHandle;
impl CaptureHandle for NoopHandle {
    fn stop(&mut self) {}
}

pub struct Tap<T>(pub Arc<Mutex<Option<Sender<T>>>>);
impl<T> Clone for Tap<T> {
    fn clone(&self) -> Self {
        Tap(self.0.clone())
    }
}
impl<T> Default for Tap<T> {
    fn default() -> Self {
        Tap(Arc::new(Mutex::new(None)))
    }
}

pub struct ScriptedVideo(pub Tap<VideoEvent>);
impl VideoSource for ScriptedVideo {
    fn describe(&self) -> SourceDescriptor {
        SourceDescriptor {
            kind: "window".into(),
            id: Some("test".into()),
            title: Some("Synthetic presentation".into()),
            app_name: Some("Test".into()),
            width: Some(1280),
            height: Some(720),
        }
    }
    fn start(&mut self, _fps: f32, tx: Sender<VideoEvent>) -> Result<Box<dyn CaptureHandle>> {
        *self.0 .0.lock() = Some(tx);
        Ok(Box::new(NoopHandle))
    }
}

pub struct ScriptedAudio(pub Tap<AudioEvent>);
impl AudioSource for ScriptedAudio {
    fn kind(&self) -> SourceKind {
        SourceKind::System
    }
    fn describe(&self) -> String {
        "synthetic system audio".into()
    }
    fn start(&mut self, tx: Sender<AudioEvent>) -> Result<Box<dyn CaptureHandle>> {
        *self.0 .0.lock() = Some(tx);
        Ok(Box::new(NoopHandle))
    }
}

/// Test transcriber: emits one word per 400 ms window that contains signal.
/// (Real transcription is tested in lc-whisper with real speech and a real model.)
pub struct EnergyTranscriber {
    pub calls: Arc<AtomicUsize>,
    pub fail: bool,
}

impl Transcriber for EnergyTranscriber {
    fn model_name(&self) -> String {
        "energy-test".into()
    }
    fn engine_name(&self) -> String {
        "test".into()
    }
    fn transcribe(&mut self, pcm: &[f32], _cancel: &AtomicBool) -> Result<TranscribeOutput> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            anyhow::bail!("synthetic engine failure");
        }
        let mut tokens = Vec::new();
        let win = 6_400; // 400 ms
        for (i, w) in pcm.chunks(win).enumerate() {
            let rms = (w.iter().map(|v| v * v).sum::<f32>() / w.len() as f32).sqrt();
            if rms > 0.03 {
                let t0 = i as u64 * 400;
                tokens.push(RawToken { t0_ms: t0 + 50, t1_ms: t0 + 350, text: format!(" słowo{n}_{i}") });
            }
        }
        if tokens.is_empty() {
            return Ok(TranscribeOutput::default());
        }
        let text = tokens.iter().map(|t| t.text.clone()).collect::<String>();
        Ok(TranscribeOutput {
            segments: vec![RawSegment {
                t0_ms: tokens[0].t0_ms,
                t1_ms: tokens.last().unwrap().t1_ms,
                text,
                tokens,
                no_speech_prob: 0.0,
            }],
            language: Some("pl".into()),
        })
    }
}

pub fn energy_factory(fail: bool) -> (TranscriberFactory, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let c2 = calls.clone();
    (Box::new(move || Ok(Box::new(EnergyTranscriber { calls: c2, fail }) as Box<dyn Transcriber>)), calls)
}

/// Upsample 16 kHz → 48 kHz by linear interpolation (exercise the resampler).
pub fn to_48k(x: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(x.len() * 3);
    for i in 0..x.len() {
        let a = x[i];
        let b = *x.get(i + 1).unwrap_or(&a);
        out.push(a);
        out.push(a + (b - a) / 3.0);
        out.push(a + 2.0 * (b - a) / 3.0);
    }
    out
}

pub struct Driver {
    pub clock: Arc<ManualClock>,
    pub video: Tap<VideoEvent>,
    pub audio: Tap<AudioEvent>,
    pub audio48: Vec<f32>,
    pub t_ms: u64,
}

fn wait_empty<T>(tx: &Sender<T>) {
    for _ in 0..2000 {
        if tx.is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_micros(200));
    }
}

impl Driver {
    /// Advance fake time to `until_ms`, delivering 10 ms audio blocks and a frame
    /// from `frame_at(t)` every 500 ms.
    pub fn run_until(&mut self, until_ms: u64, mut frame_at: impl FnMut(u64) -> Option<Frame>) {
        // `None` for audio-only recordings (`NoVideo`)
        let vtx = self.video.0.lock().clone();
        let atx = self.audio.0.lock().clone();
        while self.t_ms < until_ms {
            self.clock.set(self.t_ms);
            if self.t_ms % 500 == 0 {
                if let (Some(vtx), Some(f)) = (&vtx, frame_at(self.t_ms)) {
                    vtx.send(VideoEvent::Frame(f)).unwrap();
                    wait_empty(vtx);
                    std::thread::sleep(Duration::from_millis(3));
                }
            }
            if let Some(atx) = &atx {
                let i = (self.t_ms * 48) as usize;
                if i < self.audio48.len() {
                    let end = (i + 480).min(self.audio48.len());
                    atx.send(AudioEvent::Samples { source: SourceKind::System, rate: 48_000, data: self.audio48[i..end].to_vec() })
                        .unwrap();
                    wait_empty(&atx);
                }
            }
            self.t_ms += 10;
            self.clock.set(self.t_ms);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn wait_transcription(svc: &TranscriptionService) {
    for _ in 0..3000 {
        if svc.is_finished() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("transcription did not finish: {:?}", svc.status());
}

pub fn now(c: &ManualClock) -> u64 {
    c.now_ms()
}
