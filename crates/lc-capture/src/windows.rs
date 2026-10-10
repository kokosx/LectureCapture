//! Windows backend: Windows Graphics Capture (via `windows-capture`) for frames and
//! WASAPI loopback (via cpal) for system audio.
//!
//! Status: implemented and compiled in CI on Windows; the automated test-suite runs on
//! Windows in CI, but real capture has not yet been verified on a physical Windows
//! machine – see README "Known limitations".

use super::*;
use anyhow::{anyhow, Context, Result};
use crossbeam_channel::Sender;
use lc_core::audio::mixer::SourceKind;
use lc_core::frame::{Frame, PixelFormat};
use lc_core::pipeline::{AudioSource, CaptureHandle, VideoEvent, VideoSource};
use lc_core::session::manifest::SourceDescriptor;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows_capture::capture::{CaptureControl, Context as WcContext, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame as WcFrame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, MinimumUpdateIntervalSettings,
    SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

pub fn permissions() -> Permissions {
    // Windows has no global screen-capture permission; the yellow capture border
    // (where it cannot be disabled) shows the user that capture is active.
    Permissions { screen: true, microphone: "unknown".into() }
}

pub fn reset_permissions(_bundle_id: &str) -> Result<()> {
    Ok(())
}

pub fn request_screen_permission() -> bool {
    true
}

fn hwnd_id(w: &Window) -> u32 {
    w.as_raw_hwnd() as usize as u32
}

pub fn list_sources() -> Result<SourceList> {
    let mut displays = Vec::new();
    for (i, m) in Monitor::enumerate().map_err(|e| anyhow!("{e}"))?.into_iter().enumerate() {
        let (w, h) = (m.width().unwrap_or(0), m.height().unwrap_or(0));
        displays.push(DisplayInfo { id: i as u32, name: format!("Monitor {} ({}×{})", i + 1, w, h), width: w, height: h, scale: 1.0 });
    }
    let me = std::process::id();
    let mut windows = Vec::new();
    for w in Window::enumerate().map_err(|e| anyhow!("{e}"))? {
        let title = w.title().unwrap_or_default();
        let pid = w.process_id().unwrap_or(0);
        if title.is_empty() || pid == me || !w.is_valid() {
            continue;
        }
        let (ww, wh) = (w.width().unwrap_or(0), w.height().unwrap_or(0));
        if ww < 160 || wh < 120 {
            continue;
        }
        let exe = w.process_name().unwrap_or_default();
        windows.push(WindowInfo {
            id: hwnd_id(&w),
            title,
            app_name: exe.trim_end_matches(".exe").to_string(),
            bundle_id: exe,
            pid: pid as i32,
            width: ww as u32,
            height: wh as u32,
            on_screen: true,
        });
    }
    windows.sort_by_key(|w| (!w.app_name.to_lowercase().contains("teams"), w.app_name.to_lowercase()));
    Ok(SourceList { displays, windows })
}

enum Item {
    Window(Window),
    Monitor(Monitor),
}

fn resolve(target: &CaptureTarget) -> Result<(Item, SourceDescriptor)> {
    match target {
        CaptureTarget::Display { id } => {
            let m = Monitor::from_index(*id as usize + 1).or_else(|_| Monitor::primary()).map_err(|e| anyhow!("{e}"))?;
            let (w, h) = (m.width().ok(), m.height().ok());
            Ok((
                Item::Monitor(m),
                SourceDescriptor { kind: "display".into(), id: Some(id.to_string()), title: Some(format!("Monitor {}", id + 1)), app_name: None, width: w, height: h },
            ))
        }
        CaptureTarget::Window { id, bundle_id, title } => {
            let by_id = Window::from_raw_hwnd(*id as usize as *mut std::ffi::c_void);
            let w = if by_id.is_valid() {
                by_id
            } else {
                // window re-created (e.g. Teams meeting re-joined): same exe + title, else same exe
                Window::enumerate()
                    .map_err(|e| anyhow!("{e}"))?
                    .into_iter()
                    .filter(|w| bundle_id.as_ref().is_some_and(|b| w.process_name().ok().as_ref() == Some(b)))
                    .max_by_key(|w| (w.title().ok() == *title, w.width().unwrap_or(0) * w.height().unwrap_or(0)))
                    .context("Okno nie jest dostępne")?
            };
            let desc = SourceDescriptor {
                kind: "window".into(),
                id: Some(hwnd_id(&w).to_string()),
                title: w.title().ok(),
                app_name: w.process_name().ok(),
                width: w.width().ok().map(|v| v as u32),
                height: w.height().ok().map(|v| v as u32),
            };
            Ok((Item::Window(w), desc))
        }
    }
}

struct Flags {
    tx: Sender<VideoEvent>,
    interval: Duration,
    once: bool,
}

struct Handler {
    tx: Sender<VideoEvent>,
    interval: Duration,
    last: Option<Instant>,
    once: bool,
    scratch: Vec<u8>,
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = Flags;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: WcContext<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self { tx: ctx.flags.tx, interval: ctx.flags.interval, last: None, once: ctx.flags.once, scratch: Vec::new() })
    }

    fn on_frame_arrived(&mut self, frame: &mut WcFrame, control: InternalCaptureControl) -> Result<(), Self::Error> {
        if self.last.is_some_and(|t| t.elapsed() < self.interval) {
            return Ok(());
        }
        self.last = Some(Instant::now());
        let buf = frame.buffer()?;
        let (w, h) = (buf.width(), buf.height());
        let data = buf.as_nopadding_buffer(&mut self.scratch).to_vec();
        let f = Frame::new(w, h, PixelFormat::Rgba8, data);
        let _ = self.tx.try_send(VideoEvent::Frame(f));
        if self.once {
            control.stop();
        }
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        let _ = self.tx.try_send(VideoEvent::Lost("źródło obrazu zostało zamknięte".into()));
        Ok(())
    }
}

fn start_capture(item: Item, flags: Flags) -> Result<CaptureControl<Handler, Box<dyn std::error::Error + Send + Sync>>> {
    let interval = flags.interval;
    macro_rules! settings {
        ($it:expr) => {
            Settings::new(
                $it,
                CursorCaptureSettings::WithoutCursor,
                DrawBorderSettings::WithoutBorder,
                SecondaryWindowSettings::Default,
                MinimumUpdateIntervalSettings::Custom(interval),
                DirtyRegionSettings::Default,
                ColorFormat::Rgba8,
                flags,
            )
        };
    }
    let r = match item {
        Item::Window(w) => Handler::start_free_threaded(settings!(w)),
        Item::Monitor(m) => Handler::start_free_threaded(settings!(m)),
    };
    r.map_err(|e| anyhow!("Windows Graphics Capture: {e}"))
}

pub fn snapshot(target: &CaptureTarget) -> Result<Frame> {
    let (item, _) = resolve(target)?;
    let (tx, rx) = crossbeam_channel::bounded(4);
    let ctl = start_capture(item, Flags { tx, interval: Duration::from_millis(10), once: true })?;
    let frame = loop {
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(VideoEvent::Frame(f)) => break f,
            Ok(_) => continue,
            Err(_) => return Err(anyhow!("brak klatki ze źródła")),
        }
    };
    let _ = ctl.stop();
    Ok(frame)
}

pub struct WgcVideoSource {
    target: CaptureTarget,
    desc: SourceDescriptor,
}

pub fn video_source(target: CaptureTarget) -> Result<Box<dyn VideoSource>> {
    let (_, desc) = resolve(&target)?;
    Ok(Box::new(WgcVideoSource { target, desc }))
}

struct Supervisor {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl CaptureHandle for Supervisor {
    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop();
    }
}

impl VideoSource for WgcVideoSource {
    fn describe(&self) -> SourceDescriptor {
        self.desc.clone()
    }

    fn start(&mut self, fps: f32, tx: Sender<VideoEvent>) -> Result<Box<dyn CaptureHandle>> {
        let interval = Duration::from_secs_f32(1.0 / fps.clamp(0.2, 30.0));
        // intercept Lost events to drive reconnection
        let (itx, irx) = crossbeam_channel::bounded::<VideoEvent>(4);
        let (item, _) = resolve(&self.target)?;
        let ctl = start_capture(item, Flags { tx: itx.clone(), interval, once: false })?;
        let control = Arc::new(Mutex::new(Some(ctl)));
        let stop = Arc::new(AtomicBool::new(false));
        let (stop2, target) = (stop.clone(), self.target.clone());
        let thread = std::thread::Builder::new().name("lc-wgc".into()).spawn(move || {
            while !stop2.load(Ordering::SeqCst) {
                match irx.recv_timeout(Duration::from_millis(300)) {
                    Ok(VideoEvent::Lost(reason)) => {
                        if let Some(c) = control.lock().take() {
                            let _ = c.stop();
                        }
                        let _ = tx.send(VideoEvent::Lost(reason));
                        while !stop2.load(Ordering::SeqCst) {
                            std::thread::sleep(Duration::from_secs(3));
                            if let Ok((item, d)) = resolve(&target) {
                                if let Ok(c) = start_capture(item, Flags { tx: itx.clone(), interval, once: false }) {
                                    *control.lock() = Some(c);
                                    let what = [d.app_name, d.title].into_iter().flatten().collect::<Vec<_>>().join(" — ");
                                    let _ = tx.send(VideoEvent::Restored(what));
                                    break;
                                }
                            }
                        }
                    }
                    Ok(ev) => {
                        let _ = tx.try_send(ev);
                    }
                    Err(_) => {}
                }
            }
            if let Some(c) = control.lock().take() {
                let _ = c.stop();
            }
        })?;
        Ok(Box::new(Supervisor { stop, thread: Some(thread) }))
    }
}

pub fn system_audio_source(cfg: &lc_core::config::AudioConfig) -> Result<Box<dyn AudioSource>> {
    Ok(Box::new(super::cpal_source::CpalSource {
        kind: SourceKind::System,
        device: super::cpal_source::DeviceSel::Loopback(cfg.loopback_device.clone()),
    }))
}
