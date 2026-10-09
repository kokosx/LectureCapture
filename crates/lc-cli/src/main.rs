//! `lecturecapture-cli` – headless access to the same pipeline the desktop app uses.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use lc_capture::CaptureTarget;
use lc_core::clock::SystemClock;
use lc_core::config::*;
use lc_core::frame::NormRect;
use lc_core::pipeline::{AudioSource, Recorder, RecorderConfig, TranscriberFactory};
use lc_whisper::{ModelManager, WhisperOptions, WhisperTranscriber};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(name = "lecturecapture-cli", version, about = "LectureCapture – headless recorder and tools")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Screen recording permission status.
    Permissions,
    /// List capturable displays and windows.
    Sources,
    /// List audio devices.
    AudioDevices,
    /// Save one frame of a source as PNG.
    Snapshot {
        #[arg(long)]
        display: Option<u32>,
        #[arg(long)]
        window: Option<u32>,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Whisper model status.
    Models,
    /// Download a Whisper model (tiny, base, small, large-v3-turbo-q5).
    Download { id: String },
    /// Transcribe a WAV file (prints segments as JSON lines).
    TranscribeWav {
        wav: PathBuf,
        #[arg(long, default_value = "base")]
        model: String,
        #[arg(long, default_value = "pl")]
        lang: String,
    },
    /// Record a lecture.
    Record {
        #[arg(long)]
        display: Option<u32>,
        #[arg(long)]
        window: Option<u32>,
        /// Crop as x,y,w,h in 0..1 relative to the source.
        #[arg(long)]
        crop: Option<String>,
        #[arg(long, default_value = "Wyklad testowy")]
        title: String,
        #[arg(long)]
        out: Option<PathBuf>,
        /// Stop after N seconds (default: until Ctrl+C).
        #[arg(long)]
        seconds: Option<u64>,
        #[arg(long, default_value = "base")]
        model: String,
        #[arg(long, default_value = "pl")]
        lang: String,
        #[arg(long)]
        no_audio: bool,
        #[arg(long)]
        no_transcription: bool,
        #[arg(long)]
        mic: bool,
        #[arg(long, default_value_t = 2.0)]
        fps: f32,
    },
    /// Re-run transcription of a lecture folder from audio/recording.ogg.
    Retranscribe {
        dir: PathBuf,
        #[arg(long, default_value = "base")]
        model: String,
        #[arg(long, default_value = "pl")]
        lang: String,
    },
    /// Recover interrupted lectures in a folder.
    Recover { parent: PathBuf },
    /// Regenerate lecture.md, transcript/*.md and PROMPT.md.
    Docs { dir: PathBuf },
}

pub fn app_data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("app.lecturecapture")
}

fn models() -> ModelManager {
    ModelManager::new(std::env::var_os("LC_MODELS_DIR").map(PathBuf::from).unwrap_or_else(|| app_data_dir().join("models")))
}

fn target(display: Option<u32>, window: Option<u32>) -> Result<CaptureTarget> {
    match (display, window) {
        (_, Some(id)) => {
            let list = lc_capture::list_sources()?;
            let w = list.windows.iter().find(|w| w.id == id);
            Ok(CaptureTarget::Window { id, bundle_id: w.map(|w| w.bundle_id.clone()), title: w.map(|w| w.title.clone()) })
        }
        (Some(id), None) => Ok(CaptureTarget::Display { id }),
        (None, None) => {
            let list = lc_capture::list_sources()?;
            let d = list.displays.first().context("no display")?;
            Ok(CaptureTarget::Display { id: d.id })
        }
    }
}

fn whisper_factory(model: &str, lang: &str) -> Result<TranscriberFactory> {
    let path = models().ready_path(model)?;
    let (model, lang) = (model.to_string(), lang.to_string());
    Ok(Box::new(move || {
        let t = WhisperTranscriber::load(&path, &model, WhisperOptions { language: lang, ..Default::default() })?;
        Ok(Box::new(t) as Box<dyn lc_core::pipeline::Transcriber>)
    }))
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,whisper_rs=warn")).init();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Permissions => println!("{}", serde_json::to_string_pretty(&lc_capture::permissions())?),
        Cmd::Sources => {
            let s = lc_capture::list_sources()?;
            for d in &s.displays {
                println!("display {:>4}  {}  scale {}", d.id, d.name, d.scale);
            }
            for w in &s.windows {
                println!(
                    "window  {:>6}  {:<28} {:<40} {}x{}{}",
                    w.id,
                    w.app_name,
                    w.title.chars().take(40).collect::<String>(),
                    w.width,
                    w.height,
                    if w.on_screen { "" } else { "  (off-screen)" }
                );
            }
        }
        Cmd::AudioDevices => println!("{}", serde_json::to_string_pretty(&lc_capture::list_audio_devices())?),
        Cmd::Snapshot { display, window, out } => {
            let t = target(display, window)?;
            let f = lc_capture::snapshot(&t)?;
            lc_core::export::write_slide_png(&f, &out, PngCompression::Balanced)?;
            println!("{}x{} → {}", f.width, f.height, out.display());
        }
        Cmd::Models => {
            let mm = models();
            println!("models dir: {}", mm.dir().display());
            for s in mm.status() {
                println!("{:<20} installed={:<5} verified={:<5} {}", s.info.id, s.installed, s.verified, s.info.label);
            }
        }
        Cmd::Download { id } => {
            let mm = models();
            let cancel = AtomicBool::new(false);
            let mut last = Instant::now();
            let p = mm.download(&id, &cancel, |d, t| {
                if last.elapsed() > Duration::from_secs(2) {
                    last = Instant::now();
                    eprintln!("  {:.1}% ({} / {} MB)", d as f64 * 100.0 / t as f64, d / 1_000_000, t / 1_000_000);
                }
            })?;
            println!("verified and stored: {}", p.display());
        }
        Cmd::TranscribeWav { wav, model, lang } => {
            let (pcm, rate) = lc_core::audio::wav::read_wav(&wav)?;
            let pcm = if rate != 16_000 {
                let mut r = lc_core::audio::resample::Resampler::new(rate, 16_000);
                let mut out = Vec::new();
                r.process(&pcm, &mut out);
                out
            } else {
                pcm
            };
            let mut t = whisper_factory(&model, &lang)?()?;
            let start = Instant::now();
            let out = t.transcribe(&pcm, &AtomicBool::new(false))?;
            let meta = lc_core::transcript::ChunkMeta { chunk_id: 1, start_ms: 0, keep_from_ms: 0, end_ms: pcm.len() as u64 / 16 };
            for r in lc_core::transcript::finalize_chunk(&meta, &out.segments, out.language.clone(), &model) {
                println!("{}", serde_json::to_string(&r)?);
            }
            eprintln!(
                "language={:?} audio={:.1}s time={:.2}s ({:.1}× realtime)",
                out.language,
                pcm.len() as f32 / 16_000.0,
                start.elapsed().as_secs_f32(),
                pcm.len() as f32 / 16_000.0 / start.elapsed().as_secs_f32()
            );
        }
        Cmd::Record { display, window, crop, title, out, seconds, model, lang, no_audio, no_transcription, mic, fps } => {
            let t = target(display, window)?;
            let crop = crop
                .map(|c| -> Result<NormRect> {
                    let v: Vec<f32> = c.split(',').map(|x| x.trim().parse()).collect::<Result<_, _>>()?;
                    if v.len() != 4 {
                        bail!("--crop x,y,w,h");
                    }
                    Ok(NormRect { x: v[0], y: v[1], w: v[2], h: v[3] })
                })
                .transpose()?;
            let audio_cfg = AudioConfig { capture_microphone: mic, ..Default::default() };
            let mut sources: Vec<Box<dyn AudioSource>> = Vec::new();
            if !no_audio {
                sources.push(lc_capture::system_audio_source(&audio_cfg)?);
                if mic {
                    sources.push(lc_capture::microphone_source(None));
                }
            }
            let factory = if no_transcription { None } else { Some(whisper_factory(&model, &lang)?) };
            let cfg = RecorderConfig {
                title,
                output_parent: out.unwrap_or_else(|| dirs::document_dir().unwrap_or_default().join("LectureCapture")),
                crop,
                detector: DetectorConfig { sample_fps: fps, ..Default::default() },
                audio: audio_cfg,
                transcription: TranscriptionConfig { model: model.clone(), language: lang, ..Default::default() },
                output: OutputConfig::default(),
            };
            let rec = Recorder::start(cfg, lc_capture::video_source(t)?, sources, factory, Arc::new(SystemClock::new()))?;
            eprintln!("recording into {} (Ctrl+C to stop)", rec.lecture_dir().display());
            let stop = Arc::new(AtomicBool::new(false));
            let s2 = stop.clone();
            ctrlc::set_handler(move || s2.store(true, Ordering::SeqCst))?;
            let started = Instant::now();
            let mut last = Instant::now();
            while !stop.load(Ordering::SeqCst) && seconds.is_none_or(|s| started.elapsed().as_secs() < s) {
                std::thread::sleep(Duration::from_millis(200));
                if last.elapsed() > Duration::from_secs(5) {
                    last = Instant::now();
                    let st = rec.status();
                    eprintln!(
                        "[{:>5}s] slides={} occ={} video={}({} frames) audio={:.0}dB rec={}s silent={}s | whisper={} q={} done={} speed={:.1}x | {}",
                        st.elapsed_ms / 1000,
                        st.slides,
                        st.occurrences,
                        st.video.state,
                        st.video.frames,
                        st.audio.level_db,
                        st.audio.recorded_ms / 1000,
                        st.audio.silent_for_ms / 1000,
                        st.transcription.state,
                        st.transcription.queue_len,
                        st.transcription.done,
                        st.transcription.speed,
                        st.recent_segments.last().map(|s| s.text.as_str()).unwrap_or("")
                    );
                    for w in st.warnings.iter().filter(|w| w.t_ms + 5_000 >= st.elapsed_ms) {
                        eprintln!("    {}: {}", w.level, w.message);
                    }
                }
            }
            let outcome = rec.stop()?;
            eprintln!("stopped; finishing transcription…");
            if let Some(t) = &outcome.transcription {
                while !t.is_finished() {
                    std::thread::sleep(Duration::from_millis(500));
                }
                eprintln!("transcription: {:?}", t.status());
            }
            println!("{}", outcome.lecture_dir.display());
        }
        Cmd::Retranscribe { dir, model, lang } => {
            let s = lc_core::session::LectureSession::open(&dir)?;
            let session = Arc::new(parking_lot::Mutex::new(s));
            let cancel = AtomicBool::new(false);
            let mut last = Instant::now();
            let n = lc_core::pipeline::transcription::retranscribe(
                &session,
                whisper_factory(&model, &lang)?,
                VadConfig::default(),
                &cancel,
                |p, r| {
                    if let Some(r) = r {
                        eprintln!("[{}] {}", lc_core::util::fmt_ms(r.start_ms), r.text);
                    } else if last.elapsed() > Duration::from_secs(3) {
                        last = Instant::now();
                        eprintln!("  {:.0}%", p * 100.0);
                    }
                },
            )?;
            println!("{n} chunks transcribed");
        }
        Cmd::Recover { parent } => {
            for e in std::fs::read_dir(&parent)?.flatten() {
                if lc_core::session::recovery::needs_recovery(&e.path()) {
                    let r = lc_core::session::recovery::recover(&e.path())?;
                    println!("{}", serde_json::to_string_pretty(&r)?);
                }
            }
        }
        Cmd::Docs { dir } => {
            let s = lc_core::session::LectureSession::open(&dir)?;
            lc_core::export::write_documents(&s)?;
            println!("documents regenerated");
        }
    }
    Ok(())
}
