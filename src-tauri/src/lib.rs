//! LectureCapture desktop app (Tauri 2): commands exposed to the React UI.

mod library;
mod meeting;
pub mod selftest;
mod settings;

use lc_capture::CaptureTarget;
use lc_core::clock::SystemClock;
use lc_core::config::AudioConfig;
use lc_core::frame::{Frame, NormRect};
use lc_core::pipeline::transcription::retranscribe as run_retranscribe;
use lc_core::pipeline::{AudioEvent, AudioSource, Recorder, RecorderConfig, RecorderStatus, TranscriberFactory, TranscriptionService, TranscriptionStatusView};
use lc_core::session::recovery::{self, RecoveryReport};
use lc_core::session::LectureSession;
use lc_whisper::{ModelManager, ModelStatus, WhisperOptions, WhisperTranscriber};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use settings::Settings;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("{e:#}")
}

#[derive(Clone, Debug, Serialize, Default)]
struct JobStatus {
    /// `running`, `done`, `failed`, `cancelled`
    state: String,
    progress: f32,
    error: Option<String>,
    last_text: Option<String>,
    kind: String,
}

#[derive(Clone, Debug, Serialize, Default)]
struct DownloadStatus {
    state: String,
    downloaded: u64,
    total: u64,
    error: Option<String>,
}

struct AppState {
    settings: Mutex<Settings>,
    settings_path: PathBuf,
    models: ModelManager,
    recorder: Mutex<Option<Recorder>>,
    rec_started: Mutex<Option<Instant>>,
    keep_awake: Mutex<Option<keepawake::KeepAwake>>,
    services: Mutex<HashMap<String, Arc<TranscriptionService>>>,
    jobs: Arc<Mutex<HashMap<String, JobStatus>>>,
    job_cancel: Mutex<HashMap<String, Arc<AtomicBool>>>,
    downloads: Arc<Mutex<HashMap<String, DownloadStatus>>>,
    download_cancel: Mutex<HashMap<String, Arc<AtomicBool>>>,
    recoveries: Mutex<Vec<RecoveryReport>>,
    auto_stop: Mutex<Option<meeting::AutoStop>>,
}

impl AppState {
    fn save_settings(&self) {
        if let Err(e) = self.settings.lock().save(&self.settings_path) {
            log::error!("save settings: {e:#}");
        }
    }

    fn factory(&self, model: &str, cfg: &lc_core::config::TranscriptionConfig, language: &str) -> CmdResult<TranscriberFactory> {
        let path = self.models.ready_path(model).map_err(err)?;
        let model = model.to_string();
        let opts = WhisperOptions {
            language: language.to_string(),
            threads: cfg.threads.max(1),
            beam_size: cfg.beam_size.max(1),
            use_gpu: true,
            initial_prompt: cfg.initial_prompt.clone(),
        };
        Ok(Box::new(move || {
            let t = WhisperTranscriber::load(&path, &model, opts)?;
            Ok(Box::new(t) as Box<dyn lc_core::pipeline::Transcriber>)
        }))
    }
}

fn allow_dir(app: &AppHandle, dir: &Path) {
    if let Err(e) = app.asset_protocol_scope().allow_directory(dir, true) {
        log::warn!("asset scope: {e}");
    }
}

// ---------------------------------------------------------------- settings / system

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().clone()
}

#[tauri::command]
fn save_settings(state: State<AppState>, settings: Settings) -> CmdResult<()> {
    *state.settings.lock() = settings;
    state.save_settings();
    Ok(())
}

#[derive(Serialize)]
struct SystemInfo {
    platform: String,
    arch: String,
    version: String,
    whisper: String,
    models_dir: String,
    free_bytes: Option<u64>,
    lectures_root: String,
    recording: bool,
}

#[tauri::command]
fn system_info(state: State<AppState>) -> SystemInfo {
    let root = state.settings.lock().lectures_root.clone();
    let probe = if root.exists() { root.clone() } else { dirs::home_dir().unwrap_or_default() };
    SystemInfo {
        platform: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        version: env!("CARGO_PKG_VERSION").into(),
        whisper: lc_whisper::system_info(),
        models_dir: state.models.dir().to_string_lossy().into_owned(),
        free_bytes: lc_core::util::free_space(&probe),
        lectures_root: root.to_string_lossy().into_owned(),
        recording: state.recorder.lock().is_some(),
    }
}

#[tauri::command]
fn disk_free(path: String) -> Option<u64> {
    let p = PathBuf::from(&path);
    let probe = p.ancestors().find(|a| a.exists()).map(|a| a.to_path_buf()).unwrap_or(p);
    lc_core::util::free_space(&probe)
}

#[tauri::command]
fn permissions() -> lc_capture::Permissions {
    lc_capture::permissions()
}

#[tauri::command]
fn request_screen_permission() -> bool {
    lc_capture::request_screen_permission()
}

// ---------------------------------------------------------------- sources & preview

#[tauri::command]
async fn list_sources() -> CmdResult<lc_capture::SourceList> {
    tauri::async_runtime::spawn_blocking(lc_capture::list_sources).await.map_err(err)?.map_err(err)
}

#[tauri::command]
fn list_audio_devices() -> lc_capture::AudioDevices {
    lc_capture::list_audio_devices()
}

#[derive(Serialize)]
struct Preview {
    width: u32,
    height: u32,
    data_url: String,
}

/// Box-downscale for previews only (saved slides are never scaled).
fn downscale(f: &Frame, max_w: u32) -> Frame {
    if f.width <= max_w {
        return f.clone();
    }
    let scale = f.width as f32 / max_w as f32;
    let (w, h) = (max_w, ((f.height as f32 / scale).round() as u32).max(1));
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = ((x as f32 * scale) as u32, (y as f32 * scale) as u32);
            let rgb = f.rgb_at(sx.min(f.width - 1), sy.min(f.height - 1));
            out.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }
    Frame::new(w, h, lc_core::frame::PixelFormat::Rgba8, out)
}

#[tauri::command]
async fn snapshot(target: CaptureTarget) -> CmdResult<Preview> {
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<Preview> {
        use base64::Engine;
        let f = lc_capture::snapshot(&target)?;
        let small = downscale(&f, 1400);
        let png = lc_core::export::encode_png(&small, lc_core::config::PngCompression::Fast)?;
        Ok(Preview {
            width: f.width,
            height: f.height,
            data_url: format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)),
        })
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[derive(Deserialize, Clone)]
struct AudioSelection {
    capture_system: bool,
    capture_microphone: bool,
    microphone_device: Option<String>,
    loopback_device: Option<String>,
    only_application: Option<String>,
}

fn build_audio_sources(sel: &AudioSelection, cfg: &AudioConfig) -> (Vec<Box<dyn AudioSource>>, Vec<String>) {
    let mut out: Vec<Box<dyn AudioSource>> = Vec::new();
    let mut errors = Vec::new();
    if sel.capture_system {
        #[cfg(windows)]
        {
            out.push(Box::new(lc_capture::cpal_source::CpalSource {
                kind: lc_core::audio::mixer::SourceKind::System,
                device: lc_capture::cpal_source::DeviceSel::Loopback(sel.loopback_device.clone()),
            }));
        }
        #[cfg(not(windows))]
        {
            let _ = &sel.loopback_device;
            match lc_capture::system_audio_source(cfg) {
                Ok(s) => out.push(s),
                Err(e) => errors.push(err(e)),
            }
        }
    }
    if sel.capture_microphone {
        out.push(lc_capture::microphone_source(sel.microphone_device.clone()));
    }
    (out, errors)
}

#[derive(Serialize)]
struct AudioTestSource {
    kind: String,
    description: String,
    started: bool,
    error: Option<String>,
    buffers: u64,
    seconds_received: f32,
    level_db: f32,
    peak: f32,
}

#[tauri::command]
async fn audio_test(state: State<'_, AppState>, selection: AudioSelection, seconds: Option<f32>) -> CmdResult<Vec<AudioTestSource>> {
    let mut cfg = state.settings.lock().audio.clone();
    cfg.only_application = selection.only_application.clone();
    let secs = seconds.unwrap_or(3.0).clamp(1.0, 10.0);
    tauri::async_runtime::spawn_blocking(move || {
        let (sources, errors) = build_audio_sources(&selection, &cfg);
        let mut results: Vec<AudioTestSource> = errors
            .into_iter()
            .map(|e| AudioTestSource {
                kind: "system".into(),
                description: "dźwięk systemowy".into(),
                started: false,
                error: Some(e),
                buffers: 0,
                seconds_received: 0.0,
                level_db: -120.0,
                peak: 0.0,
            })
            .collect();
        let (tx, rx) = crossbeam_channel::bounded::<AudioEvent>(4096);
        let mut handles = Vec::new();
        let mut meta = Vec::new();
        for mut s in sources {
            let kind = s.kind();
            let desc = s.describe();
            match s.start(tx.clone()) {
                Ok(h) => {
                    handles.push(h);
                    meta.push((kind, desc, None));
                }
                Err(e) => meta.push((kind, desc, Some(err(e)))),
            }
        }
        let start = Instant::now();
        let mut acc: HashMap<String, (u64, f64, u64, f32, u32)> = HashMap::new();
        while start.elapsed() < Duration::from_secs_f32(secs) {
            if let Ok(AudioEvent::Samples { source, rate, data }) = rx.recv_timeout(Duration::from_millis(100)) {
                let e = acc.entry(format!("{source:?}")).or_insert((0, 0.0, 0, 0.0, rate));
                e.0 += 1;
                e.1 += data.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>();
                e.2 += data.len() as u64;
                e.3 = e.3.max(lc_core::audio::level::peak(&data));
                e.4 = rate;
            }
        }
        for mut h in handles {
            h.stop();
        }
        for (kind, desc, error) in meta {
            let key = format!("{kind:?}");
            let (buffers, energy, samples, peak, rate) = acc.get(&key).copied().unwrap_or((0, 0.0, 0, 0.0, 48_000));
            let level_db = if samples > 0 { 10.0 * ((energy / samples as f64) + 1e-12).log10() as f32 } else { -120.0 };
            results.push(AudioTestSource {
                kind: format!("{kind:?}").to_lowercase(),
                description: desc,
                started: error.is_none(),
                error,
                buffers,
                seconds_received: samples as f32 / rate.max(1) as f32,
                level_db,
                peak,
            });
        }
        results
    })
    .await
    .map_err(err)
}

// ---------------------------------------------------------------- models

#[tauri::command]
fn models_status(state: State<AppState>) -> Vec<ModelStatus> {
    state.models.status()
}

#[tauri::command]
fn download_model(state: State<AppState>, id: String) -> CmdResult<()> {
    if state.downloads.lock().get(&id).is_some_and(|d| d.state == "running") {
        return Ok(());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    state.download_cancel.lock().insert(id.clone(), cancel.clone());
    let downloads = state.downloads.clone();
    downloads.lock().insert(id.clone(), DownloadStatus { state: "running".into(), ..Default::default() });
    let mm = ModelManager::new(state.models.dir());
    std::thread::spawn(move || {
        let d2 = downloads.clone();
        let id2 = id.clone();
        let r = mm.download(&id, &cancel, move |done, total| {
            if let Some(s) = d2.lock().get_mut(&id2) {
                s.downloaded = done;
                s.total = total;
            }
        });
        let mut g = downloads.lock();
        let s = g.entry(id).or_default();
        match r {
            Ok(_) => s.state = "done".into(),
            Err(e) => {
                s.state = if cancel.load(Ordering::SeqCst) { "cancelled".into() } else { "failed".into() };
                s.error = Some(err(e));
            }
        }
    });
    Ok(())
}

#[tauri::command]
fn download_status(state: State<AppState>) -> HashMap<String, DownloadStatus> {
    state.downloads.lock().clone()
}

#[tauri::command]
fn cancel_download(state: State<AppState>, id: String) {
    if let Some(c) = state.download_cancel.lock().get(&id) {
        c.store(true, Ordering::SeqCst);
    }
}

#[tauri::command]
fn delete_model(state: State<AppState>, id: String) -> CmdResult<()> {
    state.models.delete(&id).map_err(err)
}

// ---------------------------------------------------------------- recording

#[derive(Deserialize)]
struct StartRequest {
    title: String,
    output_dir: Option<String>,
    /// Subject folder inside the output directory (created when missing).
    #[serde(default)]
    subject: Option<String>,
    /// Finish automatically at this local time (`HH:MM`).
    #[serde(default)]
    stop_at: Option<String>,
    #[serde(default)]
    leave_meeting: bool,
    target: CaptureTarget,
    crop: Option<NormRect>,
    audio: AudioSelection,
    transcription: bool,
    model: String,
    language: String,
    live: bool,
}

#[tauri::command]
async fn start_recording(app: AppHandle, state: State<'_, AppState>, req: StartRequest) -> CmdResult<RecorderStatus> {
    if state.recorder.lock().is_some() {
        return Err("Nagrywanie już trwa.".into());
    }
    let mut settings = state.settings.lock().clone();
    if !settings.consent_acknowledged {
        return Err("Najpierw potwierdź informację o zasadach nagrywania wykładów.".into());
    }
    if !lc_capture::permissions().screen {
        return Err("Brak uprawnienia „Nagrywanie ekranu i dźwięku systemowego”. Nadaj je w Ustawieniach systemowych → Prywatność i ochrona, a następnie uruchom aplikację ponownie.".into());
    }
    let title = if req.title.trim().is_empty() { "Wykład".to_string() } else { req.title.trim().to_string() };
    let base = req.output_dir.as_ref().filter(|s| !s.trim().is_empty()).map(PathBuf::from).unwrap_or(settings.lectures_root.clone());
    let subject = match req.subject.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(s) => Some(library::subject_name(s).map_err(err)?),
        None => None,
    };
    let parent = match &subject {
        Some(s) => base.join(s),
        None => base.clone(),
    };
    let auto_stop = match req.stop_at.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(hhmm) => {
            let at = meeting::next_occurrence(hhmm, chrono::Local::now()).map_err(err)?;
            Some(meeting::AutoStop {
                at: at.to_rfc3339(),
                at_time: at,
                leave_meeting: req.leave_meeting,
                window: meeting::MeetingWindow::from_target(&req.target),
            })
        }
        None => None,
    };
    let mut audio_cfg = settings.audio.clone();
    audio_cfg.capture_system = req.audio.capture_system;
    audio_cfg.capture_microphone = req.audio.capture_microphone;
    audio_cfg.microphone_device = req.audio.microphone_device.clone();
    audio_cfg.loopback_device = req.audio.loopback_device.clone();
    audio_cfg.only_application = req.audio.only_application.clone();
    let mut tcfg = settings.transcription.clone();
    tcfg.enabled = req.transcription;
    tcfg.model = req.model.clone();
    tcfg.language = req.language.clone();
    tcfg.live = req.live;
    let factory = if req.transcription { Some(state.factory(&req.model, &tcfg, &req.language)?) } else { None };

    let cfg = RecorderConfig {
        title,
        output_parent: parent.clone(),
        crop: req.crop.filter(|c| !c.is_full()),
        detector: settings.detector.clone(),
        audio: audio_cfg.clone(),
        transcription: tcfg.clone(),
        output: settings.output.clone(),
    };
    let target = req.target.clone();
    let sel = req.audio.clone();
    let rec = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<Recorder> {
        let video = lc_capture::video_source(target)?;
        let (audio, errors) = build_audio_sources(&sel, &audio_cfg);
        for e in errors {
            log::warn!("audio source: {e}");
        }
        Recorder::start(cfg, video, audio, factory, Arc::new(SystemClock::new()))
    })
    .await
    .map_err(err)?
    .map_err(err)?;

    allow_dir(&app, &rec.lecture_dir());
    let status = rec.status();
    *state.recorder.lock() = Some(rec);
    *state.rec_started.lock() = Some(Instant::now());
    *state.auto_stop.lock() = auto_stop;

    // remember choices
    settings.last_target = Some(req.target);
    settings.last_crop = req.crop;
    settings.audio = state.settings.lock().audio.clone();
    settings.audio.capture_microphone = req.audio.capture_microphone;
    settings.audio.microphone_device = req.audio.microphone_device;
    settings.transcription = tcfg;
    if base != settings.lectures_root && !settings.known_roots.contains(&base) {
        settings.known_roots.push(base);
    }
    settings.last_subject = subject;
    settings.auto_leave_meeting = req.leave_meeting;
    let keep = settings.keep_awake;
    let shortcut = settings.capture_shortcut.clone();
    *state.settings.lock() = settings;
    state.save_settings();

    if keep {
        match keepawake::Builder::default()
            .idle(true)
            .reason("Nagrywanie wykładu")
            .app_name("LectureCapture")
            .app_reverse_domain("app.lecturecapture")
            .create()
        {
            Ok(k) => *state.keep_awake.lock() = Some(k),
            Err(e) => log::warn!("keepawake: {e}"),
        }
    }
    if !shortcut.is_empty() {
        let h = app.clone();
        if let Err(e) = app.global_shortcut().on_shortcut(shortcut.as_str(), move |_app, _s, ev| {
            if ev.state == ShortcutState::Pressed {
                if let Some(r) = h.state::<AppState>().recorder.lock().as_ref() {
                    r.capture_slide();
                }
                let _ = h.emit("slide-capture-requested", ());
            }
        }) {
            log::warn!("global shortcut: {e}");
        }
    }
    Ok(status)
}

#[tauri::command]
fn recording_status(state: State<AppState>) -> Option<RecorderStatus> {
    state.recorder.lock().as_ref().map(|r| r.status())
}

#[tauri::command]
fn pause_recording(state: State<AppState>) {
    if let Some(r) = state.recorder.lock().as_ref() {
        r.pause();
    }
}

#[tauri::command]
fn resume_recording(state: State<AppState>) {
    if let Some(r) = state.recorder.lock().as_ref() {
        r.resume();
    }
}

#[tauri::command]
fn capture_slide(state: State<AppState>) {
    if let Some(r) = state.recorder.lock().as_ref() {
        r.capture_slide();
    }
}

#[tauri::command]
fn set_microphone_enabled(state: State<AppState>, enabled: bool) {
    if let Some(r) = state.recorder.lock().as_ref() {
        r.set_microphone_enabled(enabled);
    }
}

#[tauri::command]
async fn stop_recording(app: AppHandle) -> CmdResult<String> {
    stop_now(&app).await
}

async fn stop_now(app: &AppHandle) -> CmdResult<String> {
    let state = app.state::<AppState>();
    let Some(rec) = state.recorder.lock().take() else {
        return Err("Nagrywanie nie jest aktywne.".into());
    };
    *state.rec_started.lock() = None;
    *state.auto_stop.lock() = None;
    let _ = app.global_shortcut().unregister_all();
    let outcome = tauri::async_runtime::spawn_blocking(move || rec.stop()).await.map_err(err)?.map_err(err)?;
    *state.keep_awake.lock() = None;
    let path = outcome.lecture_dir.to_string_lossy().into_owned();
    if let Some(t) = outcome.transcription {
        state.services.lock().insert(path.clone(), t);
    }
    allow_dir(app, &outcome.lecture_dir);
    Ok(path)
}

// ---------------------------------------------------------------- scheduled end

#[derive(Serialize)]
struct AutoStopInfo {
    schedule: Option<meeting::AutoStop>,
    shortcut: &'static str,
    can_send_keys: bool,
}

fn auto_stop_info(state: &AppState) -> AutoStopInfo {
    AutoStopInfo { schedule: state.auto_stop.lock().clone(), shortcut: meeting::shortcut_label(), can_send_keys: meeting::can_send_keys() }
}

#[tauri::command]
fn get_auto_stop(state: State<AppState>) -> AutoStopInfo {
    auto_stop_info(&state)
}

/// Change (or clear with `stop_at = None`) the scheduled end of the running recording.
#[tauri::command]
fn set_auto_stop(state: State<AppState>, stop_at: Option<String>, leave_meeting: bool) -> CmdResult<AutoStopInfo> {
    let window = {
        let rec = state.recorder.lock();
        if rec.is_none() {
            return Err("Nagrywanie nie jest aktywne.".into());
        }
        state.settings.lock().last_target.as_ref().map(meeting::MeetingWindow::from_target).unwrap_or_default()
    };
    let new = match stop_at.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(hhmm) => {
            let at = meeting::next_occurrence(hhmm, chrono::Local::now()).map_err(err)?;
            Some(meeting::AutoStop { at: at.to_rfc3339(), at_time: at, leave_meeting, window })
        }
        None => None,
    };
    *state.auto_stop.lock() = new;
    state.settings.lock().auto_leave_meeting = leave_meeting;
    state.save_settings();
    Ok(auto_stop_info(&state))
}

#[tauri::command]
fn open_key_permission_settings() {
    meeting::open_key_permission_settings();
}

#[derive(Clone, Serialize)]
struct AutoStopped {
    path: Option<String>,
    error: Option<String>,
    left_meeting: bool,
    leave_error: Option<String>,
}

/// Background watcher: when the scheduled time is reached, stop and save the
/// recording, then (optionally) leave the Teams meeting.
fn spawn_auto_stop_watcher(app: AppHandle) {
    std::thread::Builder::new()
        .name("lc-auto-stop".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(500));
            let state = app.state::<AppState>();
            let due = {
                let mut a = state.auto_stop.lock();
                match a.as_ref() {
                    Some(s) if chrono::Local::now() >= s.at_time => a.take(),
                    _ => None,
                }
            };
            let Some(job) = due else { continue };
            if state.recorder.lock().is_none() {
                continue;
            }
            log::info!("scheduled end reached ({}), stopping recording", job.at);
            let _ = app.emit("auto-stop-started", ());
            let (path, error) = match tauri::async_runtime::block_on(stop_now(&app)) {
                Ok(p) => (Some(p), None),
                Err(e) => (None, Some(e)),
            };
            let (mut left_meeting, mut leave_error) = (false, None);
            if job.leave_meeting {
                match meeting::leave_meeting(&job.window) {
                    Ok(()) => left_meeting = true,
                    Err(e) => {
                        log::warn!("leave meeting: {e:#}");
                        leave_error = Some(format!("{e:#}"));
                    }
                }
            }
            let _ = app.emit("auto-stopped", AutoStopped { path, error, left_meeting, leave_error });
        })
        .expect("spawn auto-stop watcher");
}

// ---------------------------------------------------------------- library

#[tauri::command]
async fn list_lectures(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Vec<library::LectureSummary>> {
    let roots = state.settings.lock().roots();
    for r in &roots {
        allow_dir(&app, r);
    }
    tauri::async_runtime::spawn_blocking(move || library::scan(&roots)).await.map_err(err)
}

#[tauri::command]
async fn get_lecture(app: AppHandle, state: State<'_, AppState>, path: String) -> CmdResult<library::LectureDetail> {
    allow_dir(&app, Path::new(&path));
    let roots = state.settings.lock().roots();
    tauri::async_runtime::spawn_blocking(move || library::detail(Path::new(&path), &roots)).await.map_err(err)?.map_err(err)
}

#[tauri::command]
fn list_subjects(state: State<AppState>) -> Vec<library::SubjectInfo> {
    library::subjects(&state.settings.lock().roots())
}

#[tauri::command]
fn create_subject(state: State<AppState>, name: String) -> CmdResult<library::SubjectInfo> {
    let name = library::subject_name(&name).map_err(err)?;
    let dir = state.settings.lock().lectures_root.join(&name);
    if dir.join("manifest.json").exists() {
        return Err("Taka nazwa jest zajęta przez folder wykładu.".into());
    }
    std::fs::create_dir_all(&dir).map_err(err)?;
    Ok(library::SubjectInfo { name, path: dir.to_string_lossy().into_owned(), lectures: 0 })
}

fn subject_dirs_named(state: &AppState, name: &str) -> Vec<PathBuf> {
    state.settings.lock().roots().iter().map(|r| r.join(name)).filter(|d| d.is_dir() && !d.join("manifest.json").exists()).collect()
}

fn subject_busy(state: &AppState, dirs: &[PathBuf]) -> bool {
    let in_dirs = |p: &str| dirs.iter().any(|d| Path::new(p).starts_with(d));
    state.services.lock().iter().any(|(p, s)| !s.is_finished() && in_dirs(p))
        || state.jobs.lock().iter().any(|(p, j)| j.state == "running" && in_dirs(p))
        || state.recorder.lock().as_ref().is_some_and(|r| dirs.iter().any(|d| r.lecture_dir().starts_with(d)))
}

#[tauri::command]
fn rename_subject(app: AppHandle, state: State<AppState>, name: String, new_name: String) -> CmdResult<()> {
    let new_name = library::subject_name(&new_name).map_err(err)?;
    if new_name == name {
        return Ok(());
    }
    let dirs = subject_dirs_named(&state, &name);
    if subject_busy(&state, &dirs) {
        return Err("Poczekaj na zakończenie nagrywania/transkrypcji w tym przedmiocie.".into());
    }
    for d in dirs {
        let dest = d.with_file_name(&new_name);
        if dest.exists() {
            // merge into an existing subject of that name
            for e in std::fs::read_dir(&d).map_err(err)?.flatten() {
                if e.path().join("manifest.json").is_file() {
                    library::move_lecture(&e.path(), &dest).map_err(err)?;
                }
            }
            let _ = std::fs::remove_dir(&d);
        } else {
            std::fs::rename(&d, &dest).map_err(err)?;
        }
        allow_dir(&app, &dest);
    }
    if state.settings.lock().last_subject.as_deref() == Some(name.as_str()) {
        state.settings.lock().last_subject = Some(new_name);
        state.save_settings();
    }
    Ok(())
}

/// Removes an empty subject folder (lectures are never deleted here).
#[tauri::command]
fn delete_subject(state: State<AppState>, name: String) -> CmdResult<()> {
    for d in subject_dirs_named(&state, &name) {
        let has_lectures = std::fs::read_dir(&d).map_err(err)?.flatten().any(|e| e.path().join("manifest.json").is_file());
        if has_lectures {
            return Err("Przedmiot zawiera wykłady – najpierw przenieś je lub usuń.".into());
        }
        // only harmless leftovers (e.g. .DS_Store) may remain
        for e in std::fs::read_dir(&d).map_err(err)?.flatten() {
            let p = e.path();
            if p.is_file() && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
                let _ = std::fs::remove_file(p);
            }
        }
        std::fs::remove_dir(&d).map_err(|_| "Folder przedmiotu zawiera inne pliki – usuń je ręcznie.".to_string())?;
    }
    Ok(())
}

/// Move a lecture into a subject (`None` = no subject). Returns the new path.
#[tauri::command]
fn move_lecture(app: AppHandle, state: State<AppState>, path: String, subject: Option<String>) -> CmdResult<String> {
    if busy(&state, &path) {
        return Err("Poczekaj na zakończenie nagrywania/transkrypcji tego wykładu.".into());
    }
    let roots = state.settings.lock().roots();
    let lecture = PathBuf::from(&path);
    // stay in the same library root
    let root = roots.iter().find(|r| lecture.starts_with(r)).cloned().unwrap_or_else(|| state.settings.lock().lectures_root.clone());
    let dest_parent = match subject.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(s) => root.join(library::subject_name(s).map_err(err)?),
        None => root,
    };
    let new = library::move_lecture(&lecture, &dest_parent).map_err(err)?;
    state.services.lock().remove(&path);
    allow_dir(&app, &new);
    Ok(new.to_string_lossy().into_owned())
}

fn busy(state: &AppState, path: &str) -> bool {
    state.services.lock().get(path).is_some_and(|s| !s.is_finished())
        || state.jobs.lock().get(path).is_some_and(|j| j.state == "running")
        || state.recorder.lock().as_ref().is_some_and(|r| r.lecture_dir() == Path::new(path))
}

#[tauri::command]
fn delete_slide(state: State<AppState>, path: String, slide_id: u32) -> CmdResult<()> {
    if busy(&state, &path) {
        return Err("Poczekaj na zakończenie nagrywania/transkrypcji tego wykładu.".into());
    }
    let mut s = LectureSession::open(&path).map_err(err)?;
    s.delete_slide(slide_id).map_err(err)?;
    lc_core::export::write_documents(&s).map_err(err)
}

#[tauri::command]
fn set_occurrence_start(state: State<AppState>, path: String, occurrence_id: String, start_ms: u64) -> CmdResult<()> {
    if busy(&state, &path) {
        return Err("Poczekaj na zakończenie nagrywania/transkrypcji tego wykładu.".into());
    }
    let mut s = LectureSession::open(&path).map_err(err)?;
    s.set_occurrence_start(&occurrence_id, start_ms).map_err(err)?;
    lc_core::export::write_documents(&s).map_err(err)
}

#[tauri::command]
fn rename_lecture(state: State<AppState>, path: String, title: String) -> CmdResult<()> {
    if busy(&state, &path) {
        return Err("Poczekaj na zakończenie nagrywania/transkrypcji tego wykładu.".into());
    }
    let mut s = LectureSession::open(&path).map_err(err)?;
    s.manifest.lecture.title = title.trim().to_string();
    s.save().map_err(err)?;
    lc_core::export::write_documents(&s).map_err(err)
}

#[tauri::command]
fn read_prompt(path: String) -> CmdResult<String> {
    std::fs::read_to_string(Path::new(&path).join("PROMPT.md")).map_err(err)
}

#[tauri::command]
fn regenerate_documents(path: String) -> CmdResult<()> {
    let s = LectureSession::open(&path).map_err(err)?;
    lc_core::export::write_documents(&s).map_err(err)
}

#[tauri::command]
async fn export_zip(path: String, dest: String, include_audio: bool) -> CmdResult<u64> {
    tauri::async_runtime::spawn_blocking(move || lc_core::export::zip::export_zip(Path::new(&path), Path::new(&dest), include_audio))
        .await
        .map_err(err)?
        .map_err(err)
}

// ---------------------------------------------------------------- transcription jobs

#[derive(Serialize)]
struct TranscriptionInfo {
    service: Option<TranscriptionStatusView>,
    job: Option<JobStatus>,
}

#[tauri::command]
fn transcription_status(state: State<AppState>, path: String) -> TranscriptionInfo {
    TranscriptionInfo {
        service: state.services.lock().get(&path).map(|s| s.status()),
        job: state.jobs.lock().get(&path).cloned(),
    }
}

#[tauri::command]
fn retranscribe(state: State<AppState>, path: String, model: String, language: String) -> CmdResult<()> {
    if busy(&state, &path) {
        return Err("Transkrypcja tego wykładu już trwa.".into());
    }
    let tcfg = state.settings.lock().transcription.clone();
    let vad = state.settings.lock().audio.vad.clone();
    let factory = state.factory(&model, &tcfg, &language)?;
    let mut s = LectureSession::open(&path).map_err(err)?;
    s.manifest.transcription.enabled = true;
    s.manifest.transcription.language = language;
    s.save().map_err(err)?;
    let session = Arc::new(Mutex::new(s));
    let cancel = Arc::new(AtomicBool::new(false));
    state.job_cancel.lock().insert(path.clone(), cancel.clone());
    let jobs = state.jobs.clone();
    jobs.lock().insert(path.clone(), JobStatus { state: "running".into(), kind: "retranscribe".into(), ..Default::default() });
    std::thread::Builder::new()
        .name("lc-retranscribe".into())
        .spawn(move || {
            lc_core::pipeline::transcription::lower_thread_priority();
            let j2 = jobs.clone();
            let p2 = path.clone();
            let r = run_retranscribe(&session, factory, vad, &cancel, move |p, rec| {
                if let Some(j) = j2.lock().get_mut(&p2) {
                    j.progress = p.min(1.0);
                    if let Some(r) = rec {
                        j.last_text = Some(r.text.clone());
                    }
                }
            });
            let mut g = jobs.lock();
            let j = g.entry(path).or_default();
            match r {
                Ok(_) => {
                    j.state = "done".into();
                    j.progress = 1.0;
                }
                Err(e) => {
                    j.state = if cancel.load(Ordering::SeqCst) { "cancelled".into() } else { "failed".into() };
                    j.error = Some(err(e));
                    let mut s = session.lock();
                    s.manifest.transcription.status = lc_core::session::manifest::TranscriptionStatus::Failed;
                    let _ = s.save();
                }
            }
        })
        .map_err(err)?;
    Ok(())
}

#[tauri::command]
fn cancel_transcription(state: State<AppState>, path: String) {
    if let Some(c) = state.job_cancel.lock().get(&path) {
        c.store(true, Ordering::SeqCst);
    }
    if let Some(s) = state.services.lock().get(&path) {
        s.cancel();
    }
}

/// Continue queued chunks (after a crash, failure or "transcribe after lecture").
#[tauri::command]
fn resume_transcription(state: State<AppState>, path: String, model: Option<String>) -> CmdResult<()> {
    if busy(&state, &path) {
        return Ok(());
    }
    let mut s = LectureSession::open(&path).map_err(err)?;
    // failed chunks get another chance
    let failed = s.dir.pending_dir().join("failed");
    if let Ok(rd) = std::fs::read_dir(&failed) {
        for e in rd.flatten() {
            let _ = std::fs::rename(e.path(), s.dir.pending_dir().join(e.file_name()));
        }
        s.manifest.transcription.chunks_failed = 0;
        s.manifest.gaps.retain(|g| g.kind != lc_core::session::manifest::GapKind::TranscriptionFailed);
    }
    let model = model.unwrap_or_else(|| s.manifest.transcription.model.clone());
    let language = s.manifest.transcription.language.clone();
    let tcfg = state.settings.lock().transcription.clone();
    let factory = state.factory(&model, &tcfg, &language)?;
    s.manifest.transcription.enabled = true;
    s.manifest.transcription.error = None;
    s.save().map_err(err)?;
    let svc = TranscriptionService::start(Arc::new(Mutex::new(s)), factory, true, None, None);
    svc.finish();
    state.services.lock().insert(path, svc);
    Ok(())
}

#[tauri::command]
fn take_recoveries(state: State<AppState>) -> Vec<RecoveryReport> {
    std::mem::take(&mut *state.recoveries.lock())
}

// ---------------------------------------------------------------- startup

fn recover_on_startup(state: &AppState) {
    let roots = state.settings.lock().roots();
    {
        for p in library::lecture_dirs(&roots) {
            if recovery::needs_recovery(&p) {
                match recovery::recover(&p) {
                    Ok(r) => {
                        log::warn!("recovered {}", r.folder);
                        let pending = r.pending_chunks;
                        state.recoveries.lock().push(r);
                        if pending > 0 {
                            if let Ok(s) = LectureSession::open(&p) {
                                let (model, lang) = (s.manifest.transcription.model.clone(), s.manifest.transcription.language.clone());
                                let tcfg = state.settings.lock().transcription.clone();
                                if let Ok(f) = state.factory(&model, &tcfg, &lang) {
                                    let svc = TranscriptionService::start(Arc::new(Mutex::new(s)), f, true, None, None);
                                    svc.finish();
                                    state.services.lock().insert(p.to_string_lossy().into_owned(), svc);
                                }
                            }
                        }
                    }
                    Err(e) => log::error!("recovery of {} failed: {e:#}", p.display()),
                }
            }
        }
    }
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Pokaż LectureCapture", true, None::<&str>)?;
    let capture = MenuItem::with_id(app, "capture", "Zapisz slajd teraz", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Zatrzymaj i zapisz…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Zakończ", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &capture, &stop, &sep, &quit])?;
    let mut tb = TrayIconBuilder::with_id("main").menu(&menu).tooltip("LectureCapture").show_menu_on_left_click(true);
    if let Some(icon) = app.default_window_icon() {
        tb = tb.icon(icon.clone());
    }
    tb.on_menu_event(|app, ev| match ev.id().as_ref() {
        "show" => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
        "capture" => {
            if let Some(r) = app.state::<AppState>().recorder.lock().as_ref() {
                r.capture_slide();
            }
        }
        "stop" => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
            let _ = app.emit("stop-requested", ());
        }
        "quit" => {
            if app.state::<AppState>().recorder.lock().is_some() {
                let _ = app.emit("close-requested", ());
            } else {
                app.exit(0);
            }
        }
        _ => {}
    })
    .build(app)?;
    // recording indicator next to the tray icon
    let handle = app.handle().clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        let state = handle.state::<AppState>();
        let text = state.rec_started.lock().map(|_| {
            let st = state.recorder.lock().as_ref().map(|r| r.status());
            match st {
                Some(s) if s.state == lc_core::pipeline::RecState::Paused => "⏸ pauza".to_string(),
                Some(s) => format!("● {}", lc_core::util::fmt_ms(s.elapsed_ms)),
                None => String::new(),
            }
        });
        if let Some(tray) = handle.tray_by_id("main") {
            #[cfg(target_os = "macos")]
            let _ = tray.set_title(text.clone().filter(|t| !t.is_empty()));
            let _ = tray.set_tooltip(Some(match &text {
                Some(t) if !t.is_empty() => format!("LectureCapture – nagrywanie {t}"),
                _ => "LectureCapture".to_string(),
            }));
        }
    });
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .level_for("whisper_rs", log::LevelFilter::Warn)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&config_dir)?;
            std::fs::create_dir_all(&data_dir)?;
            let settings_path = config_dir.join("settings.json");
            let settings = Settings::load(&settings_path);
            let _ = std::fs::create_dir_all(&settings.lectures_root);
            let state = AppState {
                settings: Mutex::new(settings),
                settings_path,
                models: ModelManager::new(data_dir.join("models")),
                recorder: Mutex::new(None),
                rec_started: Mutex::new(None),
                keep_awake: Mutex::new(None),
                services: Mutex::new(HashMap::new()),
                jobs: Arc::new(Mutex::new(HashMap::new())),
                job_cancel: Mutex::new(HashMap::new()),
                downloads: Arc::new(Mutex::new(HashMap::new())),
                download_cancel: Mutex::new(HashMap::new()),
                recoveries: Mutex::new(Vec::new()),
                auto_stop: Mutex::new(None),
            };
            recover_on_startup(&state);
            for r in state.settings.lock().roots() {
                allow_dir(app.handle(), &r);
            }
            app.manage(state);
            spawn_auto_stop_watcher(app.handle().clone());
            if let Err(e) = setup_tray(app) {
                log::warn!("tray: {e}");
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.app_handle().state::<AppState>().recorder.lock().is_some() {
                    api.prevent_close();
                    let _ = window.emit("close-requested", ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            system_info,
            disk_free,
            permissions,
            request_screen_permission,
            list_sources,
            list_audio_devices,
            snapshot,
            audio_test,
            models_status,
            download_model,
            download_status,
            cancel_download,
            delete_model,
            start_recording,
            recording_status,
            pause_recording,
            resume_recording,
            capture_slide,
            set_microphone_enabled,
            stop_recording,
            list_lectures,
            get_lecture,
            delete_slide,
            set_occurrence_start,
            rename_lecture,
            read_prompt,
            regenerate_documents,
            export_zip,
            transcription_status,
            retranscribe,
            cancel_transcription,
            resume_transcription,
            take_recoveries,
            list_subjects,
            create_subject,
            rename_subject,
            delete_subject,
            move_lecture,
            get_auto_stop,
            set_auto_stop,
            open_key_permission_settings,
        ])
        .build(tauri::generate_context!())
        .expect("error while building LectureCapture")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, code, .. } = &event {
                if code.is_none() && app.state::<AppState>().recorder.lock().is_some() {
                    api.prevent_exit();
                    let _ = app.emit("close-requested", ());
                }
            }
        });
}
