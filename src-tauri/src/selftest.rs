//! Headless self-test of the real capture stack, run with the app's own permissions:
//! `open -a LectureCapture.app --args --selftest <out-dir> [seconds] [model]`.
//! Writes `<out-dir>/selftest-report.json`.

use lc_core::clock::SystemClock;
use lc_core::config::*;
use lc_core::pipeline::{Recorder, RecorderConfig};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub fn run(args: &[String]) {
    let out = PathBuf::from(args.first().cloned().unwrap_or_else(|| "/tmp/lc-selftest".into()));
    let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
    let model = args.get(2).cloned().unwrap_or_else(|| "base".into());
    let _ = std::fs::create_dir_all(&out);
    let mut report = serde_json::Map::new();
    let started = Instant::now();
    let r: anyhow::Result<()> = (|| {
        report.insert("permissions".into(), serde_json::to_value(lc_capture::permissions())?);
        let sources = lc_capture::list_sources()?;
        report.insert("displays".into(), serde_json::to_value(&sources.displays)?);
        report.insert("windows".into(), json!(sources.windows.iter().map(|w| format!("{} — {}", w.app_name, w.title)).collect::<Vec<_>>()));
        let d = sources.displays.first().ok_or_else(|| anyhow::anyhow!("no display"))?;
        let target = lc_capture::CaptureTarget::Display { id: d.id };
        let snap = lc_capture::snapshot(&target)?;
        lc_core::export::write_slide_png(&snap, &out.join("snapshot.png"), PngCompression::Fast)?;
        report.insert("snapshot".into(), json!({"width": snap.width, "height": snap.height}));

        let data_dir = dirs::data_dir().unwrap_or_default().join("app.lecturecapture").join("models");
        let path = lc_whisper::ModelManager::new(data_dir).ready_path(&model)?;
        let m2 = model.clone();
        let factory: lc_core::pipeline::TranscriberFactory = Box::new(move || {
            Ok(Box::new(lc_whisper::WhisperTranscriber::load(&path, &m2, lc_whisper::WhisperOptions::default())?)
                as Box<dyn lc_core::pipeline::Transcriber>)
        });
        let audio_cfg = AudioConfig::default();
        let cfg = RecorderConfig {
            title: "Selftest".into(),
            output_parent: out.clone(),
            crop: None,
            detector: DetectorConfig::default(),
            audio: audio_cfg.clone(),
            transcription: TranscriptionConfig { model: model.clone(), ..Default::default() },
            output: OutputConfig::default(),
        };
        let rec = Recorder::start(
            cfg,
            lc_capture::video_source(target)?,
            vec![lc_capture::system_audio_source(&audio_cfg)?],
            Some(factory),
            Arc::new(SystemClock::new()),
        )?;
        let mut samples = Vec::new();
        while started.elapsed() < Duration::from_secs(secs) {
            std::thread::sleep(Duration::from_secs(2));
            let st = rec.status();
            samples.push(json!({
                "t": st.elapsed_ms, "slides": st.slides, "video": st.video.state, "frames": st.video.frames,
                "level_db": st.audio.level_db, "recorded_ms": st.audio.recorded_ms, "whisper": st.transcription.state,
                "queue": st.transcription.queue_len, "done": st.transcription.done,
            }));
        }
        let st = rec.status();
        report.insert("final_status".into(), serde_json::to_value(&st)?);
        report.insert("samples".into(), json!(samples));
        let outcome = rec.stop()?;
        if let Some(t) = &outcome.transcription {
            let wait = Instant::now();
            while !t.is_finished() && wait.elapsed() < Duration::from_secs(120) {
                std::thread::sleep(Duration::from_millis(300));
            }
            report.insert("transcription".into(), serde_json::to_value(t.status())?);
        }
        report.insert("lecture_dir".into(), json!(outcome.lecture_dir));
        Ok(())
    })();
    if let Err(e) = r {
        report.insert("error".into(), json!(format!("{e:#}")));
    }
    report.insert("elapsed_s".into(), json!(started.elapsed().as_secs_f32()));
    let _ = std::fs::write(out.join("selftest-report.json"), serde_json::to_vec_pretty(&report).unwrap_or_default());
}
