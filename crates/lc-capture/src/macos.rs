//! macOS backend based on ScreenCaptureKit (macOS 15+).
//!
//! * Video: an `SCStream` limited to the sampling FPS (`minimumFrameInterval`), BGRA,
//!   native pixel size, cursor hidden. ScreenCaptureKit only delivers *complete*
//!   frames when content changed and marks static periods as idle, so a static slide
//!   costs almost nothing.
//! * Audio: a separate `SCStream` capturing system audio (all apps or one app), with
//!   `excludesCurrentProcessAudio` so our own playback (audio test) can never loop.
//! * A supervisor thread restarts streams after errors (sleep/wake, window closed and
//!   re-opened, display reconfiguration) and reports `Lost` / `Restored`.

use super::*;
use anyhow::{anyhow, bail, Context, Result};
use crossbeam_channel::{unbounded, Receiver, Sender};
use lc_core::audio::mixer::SourceKind;
use lc_core::frame::{Frame, PixelFormat as FramePx};
use lc_core::pipeline::{AudioEvent, AudioSource, CaptureHandle, VideoEvent, VideoSource};
use lc_core::session::manifest::SourceDescriptor;
use parking_lot::Mutex;
use screencapturekit::cm::{CMSampleBufferSCExt, SCFrameStatus};
use screencapturekit::cv::CVPixelBufferLockFlags;
use screencapturekit::prelude::*;
use screencapturekit::screenshot_manager::{CGImageExt, SCScreenshotManager};
use screencapturekit::shareable_content::SCShareableContentInfo;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

pub fn permissions() -> Permissions {
    Permissions { screen: unsafe { CGPreflightScreenCaptureAccess() }, microphone: "unknown".into() }
}

/// Shows the system prompt (only on explicit user action).
pub fn request_screen_permission() -> bool {
    unsafe { CGRequestScreenCaptureAccess() }
}

fn content() -> Result<SCShareableContent> {
    SCShareableContent::create()
        .with_on_screen_windows_only(false)
        .with_exclude_desktop_windows(true)
        .get()
        .map_err(|e| anyhow!("ScreenCaptureKit: {e} – sprawdź uprawnienie „Nagrywanie ekranu” w Ustawieniach systemowych"))
}

pub fn list_sources() -> Result<SourceList> {
    let c = content()?;
    let me = std::process::id() as i32;
    let mut displays = Vec::new();
    for (i, d) in c.displays().iter().enumerate() {
        let filter = SCContentFilter::create().with_display(d).with_excluding_windows(&[]).build()?;
        let (w, h, scale) = SCShareableContentInfo::for_filter(&filter)
            .map(|info| {
                let (w, h) = info.pixel_size();
                (w, h, info.point_pixel_scale())
            })
            .unwrap_or((d.width(), d.height(), 1.0));
        displays.push(DisplayInfo { id: d.display_id(), name: format!("Monitor {} ({}×{})", i + 1, w, h), width: w, height: h, scale });
    }
    let mut windows = Vec::new();
    for w in c.windows() {
        let Some(app) = w.owning_application() else { continue };
        if app.process_id() == me || w.window_layer() != 0 {
            continue;
        }
        let f = w.frame();
        if f.size.width < 160.0 || f.size.height < 120.0 {
            continue;
        }
        let title = w.title().unwrap_or_default();
        let app_name = app.application_name();
        if title.is_empty() && !w.is_on_screen() {
            continue;
        }
        windows.push(WindowInfo {
            id: w.window_id(),
            title,
            app_name,
            bundle_id: app.bundle_identifier(),
            pid: app.process_id(),
            width: f.size.width as u32,
            height: f.size.height as u32,
            on_screen: w.is_on_screen(),
        });
    }
    // Teams first, then on-screen windows, then the rest
    windows.sort_by_key(|w| {
        let teams = w.bundle_id.contains("teams") || w.app_name.to_lowercase().contains("teams");
        (!teams, !w.on_screen, w.app_name.to_lowercase())
    });
    Ok(SourceList { displays, windows })
}

/// Resolved ScreenCaptureKit objects for a target.
struct Resolved {
    filter: SCContentFilter,
    width: u32,
    height: u32,
    desc: SourceDescriptor,
    window_id: Option<u32>,
}

fn resolve(target: &CaptureTarget) -> Result<Resolved> {
    let c = content()?;
    match target {
        CaptureTarget::Display { id } => {
            let displays = c.displays();
            let d = displays
                .iter()
                .find(|d| d.display_id() == *id)
                .or(displays.first())
                .context("Nie znaleziono monitora")?;
            // never capture our own windows when recording a whole display
            let me = std::process::id() as i32;
            let apps = c.applications();
            let own: Vec<&SCRunningApplication> = apps.iter().filter(|a| a.process_id() == me).collect();
            let filter = SCContentFilter::create().with_display(d).with_excluding_applications(&own, &[]).build()?;
            let (w, h) = SCShareableContentInfo::for_filter(&filter).map(|i| i.pixel_size()).unwrap_or((d.width(), d.height()));
            Ok(Resolved {
                filter,
                width: w,
                height: h,
                desc: SourceDescriptor {
                    kind: "display".into(),
                    id: Some(d.display_id().to_string()),
                    title: Some(format!("Monitor {}", d.display_id())),
                    app_name: None,
                    width: Some(w),
                    height: Some(h),
                },
                window_id: None,
            })
        }
        CaptureTarget::Window { id, bundle_id, title } => {
            let windows = c.windows();
            let by_id = windows.iter().find(|w| w.window_id() == *id);
            let same_app = |w: &&SCWindow| {
                bundle_id.as_ref().is_some_and(|b| w.owning_application().is_some_and(|a| &a.bundle_identifier() == b))
            };
            let w = by_id
                .or_else(|| windows.iter().filter(same_app).find(|w| w.title().as_ref() == title.as_ref()))
                .or_else(|| {
                    windows
                        .iter()
                        .filter(same_app)
                        .filter(|w| w.is_on_screen() && w.window_layer() == 0)
                        .max_by_key(|w| (w.frame().size.width * w.frame().size.height) as u64)
                })
                .context("Okno nie jest dostępne (zamknięte, zminimalizowane lub na innym biurku)")?;
            let filter = SCContentFilter::create().with_window(w).build()?;
            let (pw, ph) = SCShareableContentInfo::for_filter(&filter).map(|i| i.pixel_size()).unwrap_or_else(|| {
                let f = w.frame();
                ((f.size.width * 2.0) as u32, (f.size.height * 2.0) as u32)
            });
            let app = w.owning_application();
            Ok(Resolved {
                filter,
                width: pw.max(2),
                height: ph.max(2),
                desc: SourceDescriptor {
                    kind: "window".into(),
                    id: Some(w.window_id().to_string()),
                    title: w.title(),
                    app_name: app.as_ref().map(|a| a.application_name()),
                    width: Some(pw),
                    height: Some(ph),
                },
                window_id: Some(w.window_id()),
            })
        }
    }
}

fn video_config(w: u32, h: u32, fps: f32) -> SCStreamConfiguration {
    let interval = CMTime::new((1000.0 / fps.clamp(0.2, 30.0)) as i64, 1000);
    SCStreamConfiguration::new()
        .with_width(w)
        .with_height(h)
        .with_pixel_format(PixelFormat::BGRA)
        .with_shows_cursor(false)
        .with_minimum_frame_interval(&interval)
        .with_queue_depth(3)
}

/// One-off capture for previews / region selection.
pub fn snapshot(target: &CaptureTarget) -> Result<Frame> {
    let r = resolve(target)?;
    let cfg = video_config(r.width, r.height, 1.0);
    let img = SCScreenshotManager::capture_image(&r.filter, &cfg).map_err(|e| anyhow!("zrzut nie powiódł się: {e}"))?;
    let (w, h) = (img.width() as u32, img.height() as u32);
    let rgba = img.rgba_data().map_err(|e| anyhow!("{e}"))?;
    if rgba.len() < (w * h * 4) as usize {
        bail!("unexpected image buffer size");
    }
    Ok(Frame::new(w, h, FramePx::Rgba8, rgba))
}

// ---------------------------------------------------------------------------------

enum Internal {
    Died(String),
    Resize(u32, u32),
}

struct FrameHandler {
    tx: Sender<VideoEvent>,
    internal: Sender<Internal>,
    size: Arc<(AtomicU32, AtomicU32)>,
    last_resize_req: Mutex<Option<Instant>>,
}

impl SCStreamOutputTrait for FrameHandler {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, of_type: SCStreamOutputType) {
        if !matches!(of_type, SCStreamOutputType::Screen) {
            return;
        }
        match sample.frame_status() {
            Some(SCFrameStatus::Complete) | None => {}
            Some(SCFrameStatus::Idle) => {
                let _ = self.tx.try_send(VideoEvent::Idle);
                return;
            }
            _ => return,
        }
        let Some(pb) = sample.pixel_buffer() else { return };
        let Ok(guard) = pb.lock(CVPixelBufferLockFlags::READ_ONLY) else { return };
        let (w, h, bpr) = (guard.width(), guard.height(), guard.bytes_per_row());
        let base = guard.base_address();
        if base.is_null() || w == 0 || h == 0 {
            return;
        }
        let data = unsafe { std::slice::from_raw_parts(base, bpr * h) }.to_vec();
        drop(guard);
        let frame = Frame { width: w as u32, height: h as u32, stride: bpr, format: FramePx::Bgra8, data: Arc::new(data) };
        let _ = self.tx.try_send(VideoEvent::Frame(frame));

        // Window resized? Keep capturing at native resolution.
        if let (Some(rect), Some(scale), Some(cscale)) = (sample.content_rect(), sample.scale_factor(), sample.content_scale()) {
            if cscale > 0.0 {
                let nw = (rect.size.width / cscale * scale).round() as u32;
                let nh = (rect.size.height / cscale * scale).round() as u32;
                let cw = self.size.0.load(Ordering::Relaxed);
                let ch = self.size.1.load(Ordering::Relaxed);
                let off = |a: u32, b: u32| (a as f64 - b as f64).abs() / b.max(1) as f64 > 0.02;
                if nw > 16 && nh > 16 && (off(nw, cw) || off(nh, ch)) {
                    let mut last = self.last_resize_req.lock();
                    if last.is_none_or(|t| t.elapsed() > Duration::from_secs(2)) {
                        *last = Some(Instant::now());
                        let _ = self.internal.send(Internal::Resize(nw, nh));
                    }
                }
            }
        }
    }
}

struct StreamDelegate {
    internal: Sender<Internal>,
}

impl SCStreamDelegateTrait for StreamDelegate {
    fn did_stop_with_error(&self, error: SCError) {
        let _ = self.internal.send(Internal::Died(format!("{error}")));
    }
}

struct Running {
    stream: SCStream,
    size: Arc<(AtomicU32, AtomicU32)>,
    window_id: Option<u32>,
}

fn start_video_stream(target: &CaptureTarget, fps: f32, tx: &Sender<VideoEvent>, internal: &Sender<Internal>) -> Result<(Running, SourceDescriptor)> {
    let r = resolve(target)?;
    let size = Arc::new((AtomicU32::new(r.width), AtomicU32::new(r.height)));
    let cfg = video_config(r.width, r.height, fps);
    let mut stream = SCStream::new_with_delegate(&r.filter, &cfg, StreamDelegate { internal: internal.clone() })?;
    stream.add_output_handler(
        FrameHandler { tx: tx.clone(), internal: internal.clone(), size: size.clone(), last_resize_req: Mutex::new(None) },
        SCStreamOutputType::Screen,
    )?;
    stream.start_capture().map_err(|e| anyhow!("start_capture: {e}"))?;
    Ok((Running { stream, size, window_id: r.window_id }, r.desc))
}

fn window_exists(id: u32) -> Option<bool> {
    let c = content().ok()?;
    Some(c.windows().iter().any(|w| w.window_id() == id))
}

pub struct SckVideoSource {
    target: CaptureTarget,
    desc: SourceDescriptor,
}

pub fn video_source(target: CaptureTarget) -> Result<Box<dyn VideoSource>> {
    let r = resolve(&target)?;
    Ok(Box::new(SckVideoSource { target, desc: r.desc }))
}

struct SupervisorHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl CaptureHandle for SupervisorHandle {
    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for SupervisorHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

impl VideoSource for SckVideoSource {
    fn describe(&self) -> SourceDescriptor {
        self.desc.clone()
    }

    fn start(&mut self, fps: f32, tx: Sender<VideoEvent>) -> Result<Box<dyn CaptureHandle>> {
        let (itx, irx): (Sender<Internal>, Receiver<Internal>) = unbounded();
        let (running, _) = start_video_stream(&self.target, fps, &tx, &itx)?;
        let stop = Arc::new(AtomicBool::new(false));
        let (stop2, target) = (stop.clone(), self.target.clone());
        let thread = std::thread::Builder::new().name("lc-sck-video".into()).spawn(move || {
            let mut cur = Some(running);
            let mut last_check = Instant::now();
            while !stop2.load(Ordering::SeqCst) {
                let msg = irx.recv_timeout(Duration::from_millis(300)).ok();
                let mut died: Option<String> = None;
                match msg {
                    Some(Internal::Died(reason)) => died = Some(reason),
                    Some(Internal::Resize(w, h)) => {
                        if let Some(r) = &cur {
                            let cfg = video_config(w, h, fps);
                            match r.stream.update_configuration(&cfg) {
                                Ok(()) => {
                                    r.size.0.store(w, Ordering::Relaxed);
                                    r.size.1.store(h, Ordering::Relaxed);
                                    log::info!("capture resized to {w}x{h}");
                                }
                                Err(e) => log::warn!("update_configuration: {e}"),
                            }
                        }
                    }
                    None => {}
                }
                // Detect a closed window (Teams meeting ended / window hidden).
                if died.is_none() && last_check.elapsed() > Duration::from_secs(4) {
                    last_check = Instant::now();
                    if let Some(id) = cur.as_ref().and_then(|r| r.window_id) {
                        if window_exists(id) == Some(false) {
                            died = Some("okno zostało zamknięte".into());
                        }
                    }
                }
                if let Some(reason) = died {
                    if let Some(r) = cur.take() {
                        let _ = r.stream.stop_capture();
                    }
                    let _ = tx.send(VideoEvent::Lost(reason));
                    // try to get the source back
                    while !stop2.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_secs(3));
                        while irx.try_recv().is_ok() {}
                        match start_video_stream(&target, fps, &tx, &itx) {
                            Ok((r, d)) => {
                                cur = Some(r);
                                let what = [d.app_name, d.title].into_iter().flatten().collect::<Vec<_>>().join(" — ");
                                let _ = tx.send(VideoEvent::Restored(what));
                                break;
                            }
                            Err(e) => log::debug!("video restart failed: {e:#}"),
                        }
                    }
                }
            }
            if let Some(r) = cur.take() {
                let _ = r.stream.stop_capture();
            }
        })?;
        Ok(Box::new(SupervisorHandle { stop, thread: Some(thread) }))
    }
}

// ---------------------------------------------------------------------------------
// System audio

struct AudioHandler {
    tx: Sender<AudioEvent>,
}

impl SCStreamOutputTrait for AudioHandler {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, of_type: SCStreamOutputType) {
        if !matches!(of_type, SCStreamOutputType::Audio) {
            return;
        }
        let Ok(list) = sample.audio_buffer_list() else { return };
        let bufs: Vec<Vec<f32>> = (&list)
            .into_iter()
            .map(|b| b.data().chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
            .collect();
        let channels_in_first = (&list).into_iter().next().map(|b| b.number_channels as usize).unwrap_or(1);
        let mono = if bufs.len() == 1 {
            lc_core::audio::downmix_interleaved(&bufs[0], channels_in_first.max(1))
        } else {
            let planes: Vec<&[f32]> = bufs.iter().map(|b| b.as_slice()).collect();
            lc_core::audio::downmix_planar(&planes)
        };
        if !mono.is_empty() {
            let _ = self.tx.try_send(AudioEvent::Samples { source: SourceKind::System, rate: 48_000, data: mono });
        }
    }
}

struct NullScreen;
impl SCStreamOutputTrait for NullScreen {
    fn did_output_sample_buffer(&self, _s: CMSampleBuffer, _t: SCStreamOutputType) {}
}

pub struct SckSystemAudio {
    only_app: Option<String>,
}

pub fn system_audio_source(cfg: &lc_core::config::AudioConfig) -> Result<Box<dyn AudioSource>> {
    Ok(Box::new(SckSystemAudio { only_app: cfg.only_application.clone() }))
}

fn start_audio_stream(only_app: &Option<String>, tx: &Sender<AudioEvent>, internal: &Sender<Internal>) -> Result<SCStream> {
    let c = content()?;
    let displays = c.displays();
    let d = displays.first().context("brak monitora dla strumienia audio")?;
    let apps = c.applications();
    let filter = match only_app {
        Some(bundle) => {
            let sel: Vec<&SCRunningApplication> = apps.iter().filter(|a| &a.bundle_identifier() == bundle).collect();
            if sel.is_empty() {
                bail!("aplikacja {bundle} nie jest uruchomiona");
            }
            SCContentFilter::create().with_display(d).with_including_applications(&sel, &[]).build()?
        }
        None => SCContentFilter::create().with_display(d).with_excluding_windows(&[]).build()?,
    };
    let cfg = SCStreamConfiguration::new()
        .with_width(64)
        .with_height(64)
        .with_minimum_frame_interval(&CMTime::new(1, 1))
        .with_captures_audio(true)
        .with_sample_rate(48_000)
        .with_channel_count(2)
        .with_excludes_current_process_audio(true);
    let mut stream = SCStream::new_with_delegate(&filter, &cfg, StreamDelegate { internal: internal.clone() })?;
    stream.add_output_handler(AudioHandler { tx: tx.clone() }, SCStreamOutputType::Audio)?;
    stream.add_output_handler(NullScreen, SCStreamOutputType::Screen)?;
    stream.start_capture().map_err(|e| anyhow!("start_capture (audio): {e}"))?;
    Ok(stream)
}

impl AudioSource for SckSystemAudio {
    fn kind(&self) -> SourceKind {
        SourceKind::System
    }

    fn describe(&self) -> String {
        match &self.only_app {
            Some(b) => format!("dźwięk aplikacji {b} (ScreenCaptureKit)"),
            None => "dźwięk systemowy (ScreenCaptureKit)".into(),
        }
    }

    fn start(&mut self, tx: Sender<AudioEvent>) -> Result<Box<dyn CaptureHandle>> {
        let (itx, irx) = unbounded();
        let stream = start_audio_stream(&self.only_app, &tx, &itx)?;
        let stop = Arc::new(AtomicBool::new(false));
        let (stop2, only_app) = (stop.clone(), self.only_app.clone());
        let thread = std::thread::Builder::new().name("lc-sck-audio".into()).spawn(move || {
            let mut cur = Some(stream);
            while !stop2.load(Ordering::SeqCst) {
                if let Ok(Internal::Died(reason)) = irx.recv_timeout(Duration::from_millis(300)) {
                    if let Some(s) = cur.take() {
                        let _ = s.stop_capture();
                    }
                    let _ = tx.send(AudioEvent::Lost { source: SourceKind::System, reason });
                    while !stop2.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_secs(3));
                        while irx.try_recv().is_ok() {}
                        match start_audio_stream(&only_app, &tx, &itx) {
                            Ok(s) => {
                                cur = Some(s);
                                let _ = tx.send(AudioEvent::Restored {
                                    source: SourceKind::System,
                                    detail: "strumień audio wznowiony".into(),
                                });
                                break;
                            }
                            Err(e) => log::debug!("audio restart failed: {e:#}"),
                        }
                    }
                }
            }
            if let Some(s) = cur.take() {
                let _ = s.stop_capture();
            }
        })?;
        Ok(Box::new(SupervisorHandle { stop, thread: Some(thread) }))
    }
}
