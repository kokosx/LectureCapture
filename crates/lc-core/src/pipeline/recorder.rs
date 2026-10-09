//! The recorder: owns the lecture session and the worker threads.

use super::status::*;
use super::traits::*;
use super::transcription::{enqueue_chunk, SegmentCallback, TranscriptionService};
use crate::audio::level::LevelMeter;
use crate::audio::mixer::{Block, Mixer, SourceKind};
use crate::audio::opus::OggOpusWriter;
use crate::audio::resample::Resampler;
use crate::audio::vad::Segmenter;
use crate::clock::Clock;
use crate::config::{AudioConfig, DetectorConfig, OutputConfig, TranscriptionConfig};
use crate::detect::{DetectorEvent, SlideDetector};
use crate::frame::NormRect;
use crate::session::manifest::*;
use crate::session::store::NewLecture;
use crate::session::{LectureDir, LectureSession};
use anyhow::{Context, Result};
use crossbeam_channel::{bounded, select, unbounded, Receiver, Sender};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct RecorderConfig {
    pub title: String,
    /// Parent folder in which the lecture folder is created.
    pub output_parent: PathBuf,
    pub crop: Option<NormRect>,
    pub detector: DetectorConfig,
    pub audio: AudioConfig,
    pub transcription: TranscriptionConfig,
    pub output: OutputConfig,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Control {
    /// Pause/resume at the given lecture time (ms).
    Pause(u64),
    Resume(u64),
    Capture,
    SetMic(bool),
    Stop,
    Crash,
}

enum WriterMsg {
    Detected(DetectorEvent),
    OpenGap(GapKind, u64, Option<String>),
    CloseGap(GapKind, u64),
    Stop,
}

pub struct StopOutcome {
    pub lecture_dir: PathBuf,
    pub transcription: Option<Arc<TranscriptionService>>,
}

pub struct Recorder {
    session: Arc<Mutex<LectureSession>>,
    status: Arc<Mutex<RecorderStatus>>,
    clock: Arc<dyn Clock>,
    paused: Arc<AtomicBool>,
    video_ctrl: Sender<Control>,
    audio_ctrl: Sender<Control>,
    writer_tx: Sender<WriterMsg>,
    video_thread: Option<JoinHandle<()>>,
    audio_thread: Option<JoinHandle<()>>,
    writer_thread: Option<JoinHandle<()>>,
    handles: Vec<Box<dyn CaptureHandle>>,
    transcription: Option<Arc<TranscriptionService>>,
}

impl Recorder {
    pub fn start(
        cfg: RecorderConfig,
        mut video: Box<dyn VideoSource>,
        audio_sources: Vec<Box<dyn AudioSource>>,
        transcriber: Option<TranscriberFactory>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self> {
        let (video_tx, video_rx) = bounded::<VideoEvent>(3);
        let (audio_tx, audio_rx) = bounded::<AudioEvent>(2048);
        let mut handles: Vec<Box<dyn CaptureHandle>> = Vec::new();
        let mut early_warnings = Vec::new();

        // Start the capture sources first: if the screen cannot be captured we fail
        // before creating an empty lecture folder.
        let video_desc = video.describe();
        handles.push(video.start(cfg.detector.sample_fps, video_tx).context("Nie można uruchomić przechwytywania obrazu")?);
        let mut kinds = Vec::new();
        for mut a in audio_sources {
            let kind = a.kind();
            let desc = a.describe();
            match a.start(audio_tx.clone()) {
                Ok(h) => {
                    handles.push(h);
                    kinds.push(kind);
                }
                Err(e) => early_warnings.push(format!("Nie udało się uruchomić źródła audio „{desc}”: {e:#}")),
            }
        }
        drop(audio_tx);

        let transcription_enabled = cfg.transcription.enabled && transcriber.is_some();
        let session = LectureSession::create(
            &cfg.output_parent,
            NewLecture {
                title: cfg.title.clone(),
                capture: CaptureInfo {
                    source: video_desc,
                    crop: cfg.crop,
                    detector: cfg.detector.clone(),
                    image_format: "png".into(),
                    webp_archive: cfg.output.webp_archive,
                },
                audio: AudioInfo {
                    file: Some(LectureDir::recording_rel().into()),
                    container: "ogg".into(),
                    codec: "opus".into(),
                    sample_rate: 16_000,
                    channels: 1,
                    bitrate: cfg.audio.opus_bitrate,
                    sources: kinds.clone(),
                    retention: cfg.audio.retention,
                    duration_ms: None,
                },
                transcription: TranscriptionInfo {
                    enabled: transcription_enabled,
                    engine: "whisper.cpp".into(),
                    model: cfg.transcription.model.clone(),
                    language: cfg.transcription.language.clone(),
                    status: if transcription_enabled { TranscriptionStatus::Pending } else { TranscriptionStatus::Disabled },
                    chunks_total: 0,
                    chunks_done: 0,
                    chunks_failed: 0,
                    error: None,
                    detected_languages: vec![],
                    completed_at: None,
                },
            },
            clock.started_at().fixed_offset(),
        );
        let session = match session {
            Ok(s) => s,
            Err(e) => {
                for h in &mut handles {
                    h.stop();
                }
                return Err(e.context("Nie można utworzyć folderu wykładu"));
            }
        };
        let dir = session.dir.clone();
        let session = Arc::new(Mutex::new(session));
        let mut st = RecorderStatus::new(&cfg.title, &dir.root.to_string_lossy());
        for w in early_warnings {
            st.warn(0, "warning", w);
        }
        if kinds.is_empty() {
            st.warn(0, "warning", "Brak aktywnego źródła audio – nagrywane będą tylko slajdy.");
        }
        let status = Arc::new(Mutex::new(st));

        let transcription = if transcription_enabled {
            let st2 = status.clone();
            let cb: SegmentCallback = Arc::new(move |r| {
                let mut s = st2.lock();
                s.recent_segments.push(RecentSegment { start_ms: r.start_ms, text: r.text.clone() });
                let n = s.recent_segments.len();
                if n > 12 {
                    s.recent_segments.drain(..n - 12);
                }
            });
            Some(TranscriptionService::start(
                session.clone(),
                transcriber.unwrap(),
                cfg.transcription.live,
                Some(clock.clone()),
                Some(cb),
            ))
        } else {
            None
        };

        let paused = Arc::new(AtomicBool::new(false));
        let (video_ctrl, video_ctrl_rx) = unbounded();
        let (audio_ctrl, audio_ctrl_rx) = unbounded();
        let (writer_tx, writer_rx) = unbounded();

        let writer_thread = {
            let (session, status, output) = (session.clone(), status.clone(), cfg.output.clone());
            std::thread::Builder::new()
                .name("lc-writer".into())
                .spawn(move || writer_loop(writer_rx, session, status, output))?
        };
        let video_thread = {
            let ctx = VideoCtx {
                rx: video_rx,
                ctrl: video_ctrl_rx,
                writer: writer_tx.clone(),
                status: status.clone(),
                clock: clock.clone(),
                detector: SlideDetector::new(cfg.detector.clone(), cfg.crop),
                interval: Duration::from_secs_f32(1.0 / cfg.detector.sample_fps.clamp(0.2, 10.0)),
            };
            std::thread::Builder::new().name("lc-video".into()).spawn(move || video_loop(ctx))?
        };
        let audio_thread = {
            let ctx = AudioCtx {
                rx: audio_rx,
                ctrl: audio_ctrl_rx,
                writer: writer_tx.clone(),
                status: status.clone(),
                session: session.clone(),
                clock: clock.clone(),
                cfg: cfg.audio.clone(),
                kinds,
                transcription: transcription.clone(),
                dir,
            };
            std::thread::Builder::new().name("lc-audio".into()).spawn(move || audio_loop(ctx))?
        };

        Ok(Self {
            session,
            status,
            clock,
            paused,
            video_ctrl,
            audio_ctrl,
            writer_tx,
            video_thread: Some(video_thread),
            audio_thread: Some(audio_thread),
            writer_thread: Some(writer_thread),
            handles,
            transcription,
        })
    }

    pub fn session(&self) -> Arc<Mutex<LectureSession>> {
        self.session.clone()
    }

    pub fn lecture_dir(&self) -> PathBuf {
        self.session.lock().dir.root.clone()
    }

    pub fn status(&self) -> RecorderStatus {
        let mut s = self.status.lock().clone();
        if s.state == RecState::Recording || s.state == RecState::Paused {
            s.elapsed_ms = self.clock.now_ms();
        }
        if let Some(t) = &self.transcription {
            s.transcription = t.status();
        } else {
            s.transcription.state = "disabled".into();
        }
        s
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn pause(&self) {
        if !self.paused.swap(true, Ordering::SeqCst) {
            let t = self.clock.now_ms();
            let _ = self.writer_tx.send(WriterMsg::OpenGap(GapKind::Paused, t, None));
            let _ = self.video_ctrl.send(Control::Pause(t));
            let _ = self.audio_ctrl.send(Control::Pause(t));
            self.status.lock().state = RecState::Paused;
        }
    }

    pub fn resume(&self) {
        if self.paused.swap(false, Ordering::SeqCst) {
            let t = self.clock.now_ms();
            let _ = self.writer_tx.send(WriterMsg::CloseGap(GapKind::Paused, t));
            let _ = self.video_ctrl.send(Control::Resume(t));
            let _ = self.audio_ctrl.send(Control::Resume(t));
            self.status.lock().state = RecState::Recording;
        }
    }

    /// Save the current screen as a slide right now.
    pub fn capture_slide(&self) {
        let _ = self.video_ctrl.send(Control::Capture);
    }

    pub fn set_microphone_enabled(&self, enabled: bool) {
        let _ = self.audio_ctrl.send(Control::SetMic(enabled));
    }

    fn shutdown(&mut self, ctrl: Control) {
        for h in &mut self.handles {
            h.stop();
        }
        let _ = self.video_ctrl.send(ctrl);
        let _ = self.audio_ctrl.send(ctrl);
        if let Some(t) = self.video_thread.take() {
            let _ = t.join();
        }
        if let Some(t) = self.audio_thread.take() {
            let _ = t.join();
        }
        let _ = self.writer_tx.send(WriterMsg::Stop);
        if let Some(t) = self.writer_thread.take() {
            let _ = t.join();
        }
    }

    /// Stop capturing and finalize the folder. Transcription of the remaining queue
    /// continues in the background (see the returned service).
    pub fn stop(mut self) -> Result<StopOutcome> {
        self.status.lock().state = RecState::Stopping;
        let end_ms = self.clock.now_ms();
        self.shutdown(Control::Stop);
        let dir = {
            let mut s = self.session.lock();
            s.finish(end_ms, LectureStatus::Completed)?;
            crate::export::write_documents(&s)?;
            s.dir.root.clone()
        };
        if let Some(t) = &self.transcription {
            t.finish();
        }
        self.status.lock().state = RecState::Finished;
        Ok(StopOutcome { lecture_dir: dir, transcription: self.transcription.clone() })
    }

    /// Test helper: terminate like a crash (no finalization, files left as-is).
    #[doc(hidden)]
    pub fn simulate_crash(mut self) -> PathBuf {
        self.shutdown(Control::Crash);
        if let Some(t) = &self.transcription {
            t.cancel();
            t.join();
        }
        self.lecture_dir()
    }
}

// ---------------------------------------------------------------------------------
// writer thread: PNG encoding + manifest/timeline updates (strictly ordered)

fn writer_loop(rx: Receiver<WriterMsg>, session: Arc<Mutex<LectureSession>>, status: Arc<Mutex<RecorderStatus>>, output: OutputConfig) {
    let dir = session.lock().dir.clone();
    while let Ok(msg) = rx.recv() {
        let result: Result<()> = (|| {
            match msg {
                WriterMsg::Stop => return Err(anyhow::anyhow!("__stop")),
                WriterMsg::Detected(DetectorEvent::NewSlide { slide_id, at_ms, frame, build_of, unstable, manual, dhash }) => {
                    let rel = LectureDir::slide_rel(slide_id);
                    let path = dir.abs(&rel);
                    let (sha, bytes) = crate::export::write_slide_png(&frame, &path, output.png_compression)
                        .with_context(|| format!("Nie można zapisać slajdu {rel}"))?;
                    let archive_file = if output.webp_archive {
                        let arel = LectureDir::slide_archive_rel(slide_id);
                        let webp = crate::export::encode_webp_lossless(&frame)?;
                        crate::session::fsutil::atomic_write(&dir.abs(&arel), &webp)?;
                        Some(arel)
                    } else {
                        None
                    };
                    let mut s = session.lock();
                    let captured_ms = at_ms;
                    let slide = Slide {
                        id: slide_id,
                        file: rel,
                        archive_file,
                        sha256: sha,
                        dhash: format!("{dhash:016x}"),
                        width: frame.width,
                        height: frame.height,
                        bytes,
                        captured_at: s.at(captured_ms),
                        captured_ms,
                        trigger: if manual {
                            SlideTrigger::Manual
                        } else if build_of.is_some() {
                            SlideTrigger::Build
                        } else if unstable {
                            SlideTrigger::Unstable
                        } else {
                            SlideTrigger::Auto
                        },
                        build_of,
                        updates: 0,
                        occurrences: vec![],
                        display_start_ms: None,
                        display_end_ms: None,
                    };
                    s.add_slide(slide)?;
                    s.start_occurrence(slide_id, at_ms)?;
                    let (n, occ) = (s.manifest.slides.len(), s.manifest.timeline.len());
                    drop(s);
                    let mut st = status.lock();
                    st.slides = n;
                    st.occurrences = occ;
                    st.last_slide_id = Some(slide_id);
                    st.current_slide_id = Some(slide_id);
                    st.last_slide_path = Some(path.to_string_lossy().into_owned());
                }
                WriterMsg::Detected(DetectorEvent::UpdateSlide { slide_id, at_ms, frame, dhash }) => {
                    let rel = LectureDir::slide_rel(slide_id);
                    let path = dir.abs(&rel);
                    let (sha, bytes) = crate::export::write_slide_png(&frame, &path, output.png_compression)?;
                    if output.webp_archive {
                        let webp = crate::export::encode_webp_lossless(&frame)?;
                        crate::session::fsutil::atomic_write(&dir.abs(&LectureDir::slide_archive_rel(slide_id)), &webp)?;
                    }
                    session.lock().update_slide(slide_id, at_ms, |s| {
                        s.sha256 = sha;
                        s.bytes = bytes;
                        s.width = frame.width;
                        s.height = frame.height;
                        s.dhash = format!("{dhash:016x}");
                        s.updates += 1;
                    })?;
                    let mut st = status.lock();
                    st.last_slide_id = Some(slide_id);
                    st.last_slide_path = Some(path.to_string_lossy().into_owned());
                }
                WriterMsg::Detected(DetectorEvent::Revisit { slide_id, at_ms }) => {
                    let mut s = session.lock();
                    s.start_occurrence(slide_id, at_ms)?;
                    let occ = s.manifest.timeline.len();
                    let path = s.manifest.slide(slide_id).map(|sl| dir.abs(&sl.file));
                    drop(s);
                    let mut st = status.lock();
                    st.occurrences = occ;
                    st.current_slide_id = Some(slide_id);
                    st.last_slide_path = path.map(|p| p.to_string_lossy().into_owned()).or(st.last_slide_path.take());
                }
                WriterMsg::Detected(DetectorEvent::AlreadyCaptured { slide_id }) => {
                    let mut st = status.lock();
                    let t = st.elapsed_ms;
                    st.warn(t, "info", format!("Ten slajd jest już zapisany (#{slide_id})."));
                }
                WriterMsg::OpenGap(kind, t, detail) => {
                    let mut s = session.lock();
                    if matches!(kind, GapKind::Paused | GapKind::VideoLost) {
                        s.end_occurrence(t);
                    }
                    s.open_gap(kind, t, detail)?;
                }
                WriterMsg::CloseGap(kind, t) => session.lock().close_gap(kind, t)?,
            }
            Ok(())
        })();
        if let Err(e) = result {
            if e.to_string() == "__stop" {
                break;
            }
            let mut st = status.lock();
            let t = st.elapsed_ms;
            st.warn(t, "error", format!("{e:#}"));
        }
    }
}

// ---------------------------------------------------------------------------------
// video thread: sampling + slide detection

struct VideoCtx {
    rx: Receiver<VideoEvent>,
    ctrl: Receiver<Control>,
    writer: Sender<WriterMsg>,
    status: Arc<Mutex<RecorderStatus>>,
    clock: Arc<dyn Clock>,
    detector: SlideDetector,
    interval: Duration,
}

fn video_loop(mut c: VideoCtx) {
    let mut paused = false;
    let mut last_input = Instant::now();
    let mut source_open = true;
    loop {
        let mut out: Option<DetectorEvent> = None;
        select! {
            recv(c.ctrl) -> msg => match msg {
                Ok(Control::Pause(_)) => { paused = true; c.detector.reset_reference(); }
                Ok(Control::Resume(_)) => { paused = false; c.detector.reset_reference(); }
                Ok(Control::Capture) => {
                    let t = c.clock.now_ms();
                    out = c.detector.manual_capture(t);
                    if out.is_none() {
                        c.status.lock().warn(t, "warning", "Brak obrazu do zapisania – źródło nie dostarczyło jeszcze klatki.");
                    }
                }
                Ok(Control::SetMic(_)) => {}
                Ok(Control::Stop) | Ok(Control::Crash) | Err(_) => break,
            },
            recv(c.rx) -> ev => match ev {
                Ok(VideoEvent::Frame(f)) => {
                    last_input = Instant::now();
                    let t = c.clock.now_ms();
                    {
                        let mut st = c.status.lock();
                        st.video.frames += 1;
                        st.video.width = f.width;
                        st.video.height = f.height;
                        st.video.last_frame_ms = Some(t);
                        if st.video.state != "lost" { st.video.state = "ok".into(); }
                    }
                    if !paused {
                        out = c.detector.push_frame(&f, t);
                    }
                }
                Ok(VideoEvent::Idle) => {
                    last_input = Instant::now();
                    if !paused { out = c.detector.push_idle(c.clock.now_ms()); }
                    c.status.lock().video.idle_ticks += 1;
                }
                Ok(VideoEvent::Lost(reason)) => {
                    let t = c.clock.now_ms();
                    c.detector.reset_reference();
                    let mut st = c.status.lock();
                    st.video.state = "lost".into();
                    st.video.detail = Some(reason.clone());
                    st.current_slide_id = None;
                    st.warn(t, "error", format!("Utracono obraz: {reason}"));
                    drop(st);
                    let _ = c.writer.send(WriterMsg::OpenGap(GapKind::VideoLost, t, Some(reason)));
                }
                Ok(VideoEvent::Restored(detail)) => {
                    let t = c.clock.now_ms();
                    c.detector.reset_reference();
                    let mut st = c.status.lock();
                    st.video.state = "ok".into();
                    st.video.detail = None;
                    st.warn(t, "info", format!("Obraz przywrócony: {detail}"));
                    drop(st);
                    let _ = c.writer.send(WriterMsg::CloseGap(GapKind::VideoLost, t));
                }
                Err(_) => {
                    if source_open {
                        source_open = false;
                        log::info!("video source channel closed");
                    }
                    // keep serving control messages until Stop
                    if let Ok(m) = c.ctrl.recv() {
                        if matches!(m, Control::Stop | Control::Crash) { break; }
                    } else { break; }
                }
            },
            default(c.interval) => {}
        }
        // Static screen: sources may deliver nothing → treat silence as "unchanged".
        if out.is_none() && !paused && last_input.elapsed() >= c.interval && c.detector.last_frame().is_some() {
            last_input = Instant::now();
            out = c.detector.push_idle(c.clock.now_ms());
        }
        if let Some(e) = out {
            let _ = c.writer.send(WriterMsg::Detected(e));
        }
        let stats = c.detector.stats.clone();
        c.status.lock().video.idle_ticks = stats.idle_ticks;
    }
}

// ---------------------------------------------------------------------------------
// audio thread: resample → mix → Opus + VAD → transcription queue

struct AudioCtx {
    rx: Receiver<AudioEvent>,
    ctrl: Receiver<Control>,
    writer: Sender<WriterMsg>,
    status: Arc<Mutex<RecorderStatus>>,
    session: Arc<Mutex<LectureSession>>,
    clock: Arc<dyn Clock>,
    cfg: AudioConfig,
    kinds: Vec<SourceKind>,
    transcription: Option<Arc<TranscriptionService>>,
    dir: LectureDir,
}

struct AudioState {
    opus: Option<OggOpusWriter>,
    seg: Segmenter,
    meter: LevelMeter,
    no_signal: bool,
    /// Pause intervals on the timeline (samples); applied by stream position, not by
    /// arrival time, because the mixer emits audio ~1 s behind real time.
    pauses: Vec<(u64, Option<u64>)>,
    /// Timeline position of the next emitted sample.
    pos: u64,
    in_pause: bool,
}

impl AudioState {
    fn paused_at(&self, p: u64) -> bool {
        self.pauses.iter().any(|&(a, b)| p >= a && b.is_none_or(|b| p < b))
    }
    /// Next pause boundary strictly after `p`.
    fn next_boundary(&self, p: u64) -> Option<u64> {
        self.pauses
            .iter()
            .flat_map(|&(a, b)| [Some(a), b])
            .flatten()
            .filter(|&x| x > p)
            .min()
    }
    fn currently_paused(&self) -> bool {
        self.pauses.last().is_some_and(|(_, b)| b.is_none())
    }
}

fn audio_loop(c: AudioCtx) {
    let mut mixer = Mixer::new(1_000);
    for k in &c.kinds {
        let gain = if *k == SourceKind::Microphone { c.cfg.microphone_gain } else { 1.0 };
        mixer.add_source(*k, gain);
    }
    let mut resamplers: HashMap<SourceKind, Resampler> = HashMap::new();
    let opus = match OggOpusWriter::create(&c.dir.recording_path(), c.cfg.opus_bitrate) {
        Ok(w) => Some(w),
        Err(e) => {
            c.status.lock().warn(0, "error", format!("Nie można utworzyć pliku audio: {e:#}"));
            None
        }
    };
    let mut st = AudioState {
        opus,
        seg: Segmenter::new(c.cfg.vad.clone(), 0, 1),
        meter: LevelMeter::new(c.cfg.silence_threshold_db),
        no_signal: false,
        pauses: Vec::new(),
        pos: 0,
        in_pause: false,
    };
    let mut last_pull = Instant::now();
    let mut last_house = Instant::now() - Duration::from_secs(60);
    let mut last_disk = Instant::now() - Duration::from_secs(60);
    let mut crashed = false;
    let tick = Duration::from_millis(100);
    loop {
        select! {
            recv(c.ctrl) -> msg => match msg {
                Ok(Control::Pause(t)) => st.pauses.push((t * 16, None)),
                Ok(Control::Resume(t)) => {
                    if let Some(last) = st.pauses.last_mut() { last.1 = Some(t * 16); }
                }
                Ok(Control::SetMic(on)) => mixer.set_enabled(SourceKind::Microphone, on),
                Ok(Control::Capture) => {}
                Ok(Control::Crash) => { crashed = true; break; }
                Ok(Control::Stop) | Err(_) => break,
            },
            recv(c.rx) -> ev => match ev {
                Ok(AudioEvent::Samples { source, rate, data }) => {
                    let r = resamplers.entry(source).or_insert_with(|| Resampler::new(rate, 16_000));
                    if r.in_rate() != rate {
                        *r = Resampler::new(rate, 16_000);
                        mixer.reset_source(source);
                    }
                    let mut out = Vec::with_capacity(data.len() / 2 + 64);
                    r.process(&data, &mut out);
                    mixer.push(source, &out, c.clock.now_ms());
                }
                Ok(AudioEvent::Lost { source, reason }) => {
                    let t = c.clock.now_ms();
                    mixer.reset_source(source);
                    let mut s = c.status.lock();
                    s.audio.lost = Some(reason.clone());
                    s.warn(t, "error", format!("Utracono audio ({source:?}): {reason}"));
                    drop(s);
                    let _ = c.writer.send(WriterMsg::OpenGap(GapKind::AudioLost, t, Some(reason)));
                }
                Ok(AudioEvent::Restored { source, detail }) => {
                    let t = c.clock.now_ms();
                    mixer.reset_source(source);
                    let mut s = c.status.lock();
                    s.audio.lost = None;
                    s.warn(t, "info", format!("Audio przywrócone ({source:?}): {detail}"));
                    drop(s);
                    let _ = c.writer.send(WriterMsg::CloseGap(GapKind::AudioLost, t));
                }
                Err(_) => {
                    // all sources ended; wait for Stop while keeping the timeline going
                    if let Ok(m) = c.ctrl.recv_timeout(tick) {
                        match m {
                            Control::Stop => break,
                            Control::Crash => { crashed = true; break; }
                            Control::Pause(t) => st.pauses.push((t * 16, None)),
                            Control::Resume(t) => {
                                if let Some(last) = st.pauses.last_mut() { last.1 = Some(t * 16); }
                            }
                            _ => {}
                        }
                    }
                }
            },
            default(tick) => {}
        }
        if last_pull.elapsed() >= tick {
            last_pull = Instant::now();
            let blocks = mixer.pull(c.clock.now_ms(), 16_000 * 30);
            process_blocks(&c, &mut st, blocks, &mixer);
        }
        if last_house.elapsed() >= Duration::from_secs(1) {
            last_house = Instant::now();
            let now = c.clock.now_ms();
            if let Err(e) = c.session.lock().heartbeat(now) {
                c.status.lock().warn(now, "error", format!("Zapis manifestu nie powiódł się: {e:#}"));
            }
            if last_disk.elapsed() >= Duration::from_secs(10) {
                last_disk = Instant::now();
                let size = crate::session::fsutil::dir_size(&c.dir.root);
                let free = crate::util::free_space(&c.dir.root);
                let mut s = c.status.lock();
                s.lecture_bytes = size;
                s.free_bytes = free;
                if let Some(f) = free {
                    if f < 500 * 1024 * 1024 {
                        s.warn(now, "error", format!("Mało miejsca na dysku: {} MB", f / 1024 / 1024));
                    }
                }
                if !c.dir.root.exists() {
                    s.warn(now, "error", "Folder wykładu zniknął lub jest niedostępny!");
                }
            }
        }
    }
    if crashed {
        // simulate abrupt termination: drop the writer without finishing the stream
        drop(st.opus.take());
        return;
    }
    let blocks = mixer.drain_all();
    process_blocks(&c, &mut st, blocks, &mixer);
    if let Some(ch) = st.seg.flush() {
        enqueue(&c, ch);
    }
    if let Some(w) = st.opus.take() {
        let ms = w.samples_written() / 16;
        if let Err(e) = w.finish() {
            c.status.lock().warn(ms, "error", format!("Nie można domknąć pliku audio: {e:#}"));
        }
        c.session.lock().manifest.audio.duration_ms = Some(ms);
    }
}

fn enqueue(c: &AudioCtx, chunk: crate::audio::vad::SpeechChunk) {
    let Some(ts) = &c.transcription else { return };
    match enqueue_chunk(&c.dir, &chunk) {
        Ok(()) => {
            c.session.lock().manifest.transcription.chunks_total += 1;
            ts.notify();
        }
        Err(e) => c.status.lock().warn(chunk.start / 16, "error", format!("Kolejka transkrypcji: {e:#}")),
    }
}

fn process_blocks(c: &AudioCtx, st: &mut AudioState, blocks: Vec<Block>, mixer: &Mixer) {
    let transcribe = c.transcription.is_some();
    for b in blocks {
        // split the block at pause boundaries
        let total = b.len();
        let mut off = 0u64;
        while off < total {
            let p = st.pos;
            let len = st.next_boundary(p).map(|nb| (nb - p).min(total - off)).unwrap_or(total - off);
            let paused = st.paused_at(p);
            if paused && !st.in_pause {
                // close the utterance exactly at the pause
                if let Some(ch) = st.seg.flush() {
                    enqueue(c, ch);
                }
            }
            st.in_pause = paused;
            let pos_ms = p / 16;
            let mut chunks = Vec::new();
            match (&b, paused) {
                (Block::Samples(v), false) => {
                    let part = &v[off as usize..(off + len) as usize];
                    if let Some(w) = &mut st.opus {
                        if let Err(e) = w.write(part) {
                            c.status.lock().warn(pos_ms, "error", format!("Zapis audio: {e:#}"));
                        }
                    }
                    st.meter.process(part, pos_ms);
                    if transcribe {
                        chunks.extend(st.seg.push(part));
                    } else {
                        st.seg.push_silence(len);
                    }
                }
                (Block::Samples(v), true) => {
                    // paused: keep the timeline aligned with silence, but meter the input
                    if let Some(w) = &mut st.opus {
                        let _ = w.write_silence(len);
                    }
                    st.meter.process(&v[off as usize..(off + len) as usize], pos_ms);
                    st.seg.push_silence(len);
                }
                (Block::Silence(_), _) => {
                    if let Some(w) = &mut st.opus {
                        let _ = w.write_silence(len);
                    }
                    st.meter.process(&[0.0; 160], pos_ms);
                    chunks.extend(st.seg.push_silence(len));
                }
            }
            for ch in chunks {
                enqueue(c, ch);
            }
            st.pos += len;
            off += len;
        }
    }
    // status + no-signal detection (on the audio timeline)
    let pos_ms = st.seg.position() / 16;
    let silent_for = st.meter.silent_for(pos_ms);
    let mut s = c.status.lock();
    s.audio.level_db = st.meter.level_db;
    s.audio.peak = st.meter.peak;
    s.audio.sources = mixer.stats();
    s.audio.silent_for_ms = silent_for;
    s.audio.recorded_ms = mixer.written() / 16;
    s.audio.speech_ratio = if st.seg.total_frames > 0 { st.seg.speech_frames as f32 / st.seg.total_frames as f32 } else { 0.0 };
    if !st.currently_paused() && silent_for >= c.cfg.silence_warn_ms && !st.no_signal {
        st.no_signal = true;
        s.audio.no_signal = true;
        let start = pos_ms.saturating_sub(silent_for);
        s.warn(pos_ms, "warning", format!(
            "Brak sygnału audio od {} s – sprawdź, czy dźwięk z Teams jest odtwarzany i przechwytywany.",
            silent_for / 1000
        ));
        let _ = c.writer.send(WriterMsg::OpenGap(GapKind::NoAudioSignal, start, None));
    } else if st.no_signal && silent_for == 0 {
        st.no_signal = false;
        s.audio.no_signal = false;
        s.warn(pos_ms, "info", "Sygnał audio powrócił.");
        let _ = c.writer.send(WriterMsg::CloseGap(GapKind::NoAudioSignal, pos_ms));
    }
}
