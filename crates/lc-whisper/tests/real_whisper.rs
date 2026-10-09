//! Real transcription tests with whisper.cpp and real (macOS `say`-generated) speech.
//! They need a downloaded model; without it they are skipped with a message
//! (CI does not download models).

use lc_core::clock::ManualClock;
use lc_core::config::*;
use lc_core::pipeline::*;
use lc_core::session::fsutil::read_jsonl;
use lc_core::session::LectureSession;
use lc_core::synth::slide;
use lc_core::testing::*;
use lc_core::transcript::{finalize_chunk, ChunkMeta, SegmentRecord};
use lc_whisper::{ModelManager, WhisperOptions, WhisperTranscriber};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn model(id: &str) -> Option<PathBuf> {
    let dir = std::env::var_os("LC_MODELS_DIR").map(PathBuf::from).unwrap_or_else(|| {
        dirs_data().join("app.lecturecapture").join("models")
    });
    match ModelManager::new(dir).ready_path(id) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("SKIPPED: model '{id}' not available ({e})");
            None
        }
    }
}

fn dirs_data() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        PathBuf::from(std::env::var("HOME").unwrap()).join("Library/Application Support")
    }
    #[cfg(windows)]
    {
        PathBuf::from(std::env::var("APPDATA").unwrap())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        PathBuf::from(std::env::var("HOME").unwrap()).join(".local/share")
    }
}

fn fixture(name: &str) -> Vec<f32> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    let (pcm, rate) = lc_core::audio::wav::read_wav(&p).unwrap();
    assert_eq!(rate, 16_000);
    pcm
}

fn norm(s: &str) -> String {
    s.to_lowercase()
}

#[test]
fn polish_speech_is_transcribed_with_word_timestamps() {
    let Some(path) = model("base") else { return };
    let mut t = WhisperTranscriber::load(&path, "base", WhisperOptions::default()).unwrap();
    let pcm = fixture("pl_speech.wav");
    let out = t.transcribe(&pcm, &AtomicBool::new(false)).unwrap();
    assert_eq!(out.language.as_deref(), Some("pl"));
    let meta = ChunkMeta { chunk_id: 1, start_ms: 5_000, keep_from_ms: 5_000, end_ms: 5_000 + pcm.len() as u64 / 16 };
    let recs = finalize_chunk(&meta, &out.segments, out.language, "base");
    let text = norm(&recs.iter().map(|r| r.text.clone()).collect::<Vec<_>>().join(" "));
    eprintln!("{text}");
    for w in ["dzień dobry", "drzew", "binarn", "dzieci", "logarytm"] {
        assert!(text.contains(w), "missing '{w}' in: {text}");
    }
    let words: Vec<_> = recs.iter().flat_map(|r| r.words.iter()).collect();
    assert!(words.len() >= 20);
    assert!(words.windows(2).all(|w| w[0].s <= w[1].s), "monotonic word times");
    assert!(words[0].s >= 5_000 && words.last().unwrap().e <= meta.end_ms);
}

#[test]
fn english_is_detected_and_not_translated() {
    let Some(path) = model("base") else { return };
    let opts = WhisperOptions { language: "auto".into(), ..Default::default() };
    let mut t = WhisperTranscriber::load(&path, "base", opts).unwrap();
    let out = t.transcribe(&fixture("en_speech.wav"), &AtomicBool::new(false)).unwrap();
    assert_eq!(out.language.as_deref(), Some("en"));
    let text = norm(&out.segments.iter().map(|s| s.text.clone()).collect::<String>());
    assert!(text.contains("binary search"), "{text}");
}

#[test]
fn full_pipeline_with_real_whisper() {
    let Some(path) = model("base") else { return };
    let speech = fixture("pl_speech.wav");
    // 2 s silence, sentence, 3 s silence, sentence, 2 s silence
    let mut audio = vec![0.0f32; 32_000];
    audio.extend(&speech);
    audio.extend(vec![0.0; 48_000]);
    audio.extend(&speech);
    audio.extend(vec![0.0; 32_000]);
    let total_ms = audio.len() as u64 / 16;

    let tmp = tempfile::tempdir().unwrap();
    let clock = Arc::new(ManualClock::new());
    let (vtap, atap) = (Tap::default(), Tap::default());
    let p2 = path.clone();
    let factory: TranscriberFactory = Box::new(move || {
        Ok(Box::new(WhisperTranscriber::load(&p2, "base", WhisperOptions::default())?) as Box<dyn Transcriber>)
    });
    let cfg = RecorderConfig {
        title: "Struktury danych".into(),
        output_parent: tmp.path().to_path_buf(),
        crop: None,
        detector: DetectorConfig::default(),
        audio: AudioConfig::default(),
        transcription: TranscriptionConfig { model: "base".into(), ..Default::default() },
        output: OutputConfig::default(),
    };
    let rec = Recorder::start(
        cfg,
        Box::new(ScriptedVideo(vtap.clone())),
        vec![Box::new(ScriptedAudio(atap.clone()))],
        Some(factory),
        clock.clone(),
    )
    .unwrap();
    let mut d = Driver { clock, video: vtap, audio: atap, audio48: to_48k(&audio), t_ms: 0 };
    let (a, b) = (slide(1280, 720, 1, 4), slide(1280, 720, 2, 6));
    let switch = 2_000 + speech.len() as u64 / 16 + 1_500;
    d.run_until(switch, |_| Some(a.clone()));
    d.run_until(total_ms, |_| Some(b.clone()));
    let out = rec.stop().unwrap();
    wait_transcription(out.transcription.as_ref().unwrap());

    let s = LectureSession::open(&out.lecture_dir).unwrap();
    assert_eq!(s.manifest.slides.len(), 2);
    let segs: Vec<SegmentRecord> = read_jsonl(&s.dir.segments_path()).unwrap();
    let data = lc_core::export::load_transcript(&s).unwrap();
    let text_of = |i: usize| {
        norm(&data.assignment.slides[i].parts.iter().map(|p| p.text()).collect::<Vec<_>>().join(" "))
    };
    eprintln!("slide 1: {}\nslide 2: {}", text_of(0), text_of(1));
    // each slide gets its own sentence, timestamps aligned with the audio
    assert!(text_of(0).contains("drzew") && text_of(0).contains("logarytm"));
    assert!(text_of(1).contains("drzew") && text_of(1).contains("logarytm"));
    let first = segs.iter().flat_map(|r| r.words.iter()).map(|w| w.s).min().unwrap();
    assert!(first.abs_diff(2_000) < 800, "speech starts at 2 s, got {first}");
    let lecture = std::fs::read_to_string(s.dir.abs("lecture.md")).unwrap();
    assert!(norm(&lecture).contains("binarn"));
    assert_eq!(s.manifest.transcription.status, lc_core::session::manifest::TranscriptionStatus::Completed);
}
