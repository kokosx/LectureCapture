//! End-to-end tests of the recording pipeline with synthetic sources:
//! slide changes, revisits, builds, pause, speech across slide changes, stop during
//! speech, missing audio signal, transcription errors, crash recovery, document output.

mod common;

use common::*;
use lc_core::clock::ManualClock;
use lc_core::config::*;
use lc_core::export::load_transcript;
use lc_core::frame::Frame;
use lc_core::pipeline::*;
use lc_core::session::fsutil::read_jsonl;
use lc_core::session::manifest::*;
use lc_core::session::{recovery, LectureSession};
use lc_core::synth::{slide, speech_like, with_cursor, with_noise};
use lc_core::transcript::SegmentRecord;
use std::sync::Arc;

const W: u32 = 1280;
const H: u32 = 720;

fn config(parent: &std::path::Path) -> RecorderConfig {
    RecorderConfig {
        title: "Algorytmy i struktury danych".into(),
        output_parent: parent.to_path_buf(),
        crop: None,
        detector: DetectorConfig::default(),
        audio: AudioConfig::default(),
        transcription: TranscriptionConfig { model: "energy-test".into(), ..Default::default() },
        output: OutputConfig::default(),
    }
}

/// Speech plan of the main scenario (ms, speech?) → 75 s.
fn speech_plan() -> Vec<(u32, bool)> {
    vec![
        (1000, false),
        (7000, true),   // 1–8 s
        (1000, false),
        (14000, true),  // 9–23 s, spans the slide change at 20 s
        (2000, false),
        (20000, true),  // 25–45 s, spans the change at 40 s
        (3000, false),
        (10000, true),  // 48–58 s, spans the change at 50 s
        (2000, false),
        (5000, true),   // 60–65 s: recording paused – must not be transcribed
        (1000, false),
        (9000, true),   // 66–75 s: still talking when Stop is pressed
    ]
}

fn start(
    dir: &std::path::Path,
    cfg: RecorderConfig,
    factory: Option<TranscriberFactory>,
    speech: &[f32],
) -> (Recorder, Driver) {
    let clock = Arc::new(ManualClock::new());
    let vtap = Tap::default();
    let atap = Tap::default();
    let _ = dir;
    let rec = Recorder::start(
        cfg,
        Box::new(ScriptedVideo(vtap.clone())),
        vec![Box::new(ScriptedAudio(atap.clone()))],
        factory,
        clock.clone(),
    )
    .expect("recorder starts");
    let driver = Driver { clock, video: vtap, audio: atap, audio48: to_48k(speech), t_ms: 0 };
    (rec, driver)
}

#[test]
fn full_pipeline_scenario() {
    let tmp = tempfile::tempdir().unwrap();
    let (speech, spans) = speech_like(&speech_plan(), 42);
    let (factory, calls) = energy_factory(false);
    let (rec, mut d) = start(tmp.path(), config(tmp.path()), Some(factory), &speech);

    let a = slide(W, H, 1, 4);
    let b = slide(W, H, 2, 6);
    let c2 = slide(W, H, 7, 2);
    let c3 = slide(W, H, 7, 3);
    let c4 = slide(W, H, 7, 4);

    // 0–20 s slide A with a wandering cursor and compression noise
    d.run_until(20_000, |t| {
        let f = with_noise(&a, 6, t);
        Some(if (t / 500) % 3 == 0 { with_cursor(&f, 100 + (t / 50) as u32 % 900, 300) } else { f })
    });
    // 20–40 s slide B
    d.run_until(40_000, |_| Some(b.clone()));
    // 40–50 s back to slide A (revisit)
    d.run_until(50_000, |t| Some(with_noise(&a, 6, t)));
    // 50–60 s slide C revealed in three steps
    d.run_until(60_000, |t| Some(if t < 52_000 { c2.clone() } else if t < 55_000 { c3.clone() } else { c4.clone() }));
    // 60–65 s paused (the lecturer keeps talking)
    rec.pause();
    d.run_until(65_000, |_| Some(c4.clone()));
    rec.resume();
    // 65–75 s slide C again; manual capture of an already saved slide
    d.run_until(70_000, |_| Some(c4.clone()));
    rec.capture_slide();
    d.run_until(75_000, |_| Some(c4.clone()));

    let st = rec.status();
    assert_eq!(st.slides, 3, "warnings: {:?}", st.warnings);
    assert!(st.audio.level_db > -60.0 || st.audio.recorded_ms > 70_000);

    let out = rec.stop().expect("stop");
    let svc = out.transcription.expect("transcription service");
    wait_transcription(&svc);
    assert!(calls.load(std::sync::atomic::Ordering::SeqCst) >= 5);

    let s = LectureSession::open(&out.lecture_dir).unwrap();
    let m = &s.manifest;
    // --- manifest / timeline -------------------------------------------------------
    assert_eq!(m.schema, SCHEMA);
    assert_eq!(m.lecture.status, LectureStatus::Completed);
    assert_eq!(m.lecture.duration_ms, Some(75_000));
    assert_eq!(m.slides.len(), 3);
    let seq: Vec<u32> = m.timeline.iter().map(|o| o.slide_id).collect();
    assert_eq!(seq, vec![1, 2, 1, 3, 3], "timeline {:?}", m.timeline);
    let starts: Vec<u64> = m.timeline.iter().map(|o| o.start_ms).collect();
    for (got, want) in starts.iter().zip([0u64, 20_000, 40_000, 50_000, 65_000]) {
        assert!(got.abs_diff(want) <= 600, "occurrence start {got} vs {want}");
    }
    // pause closes the occurrence of slide C at 60 s
    assert!(m.timeline[3].end_ms.unwrap().abs_diff(60_000) <= 100);
    assert!(m.gaps.iter().any(|g| g.kind == GapKind::Paused && g.start_ms.abs_diff(60_000) <= 100 && g.end_ms.unwrap().abs_diff(65_000) <= 100));
    let c = m.slide(3).unwrap();
    assert_eq!(c.updates, 2, "two build steps merged into slide 3");
    assert_eq!(c.occurrences.len(), 2);
    assert_eq!(m.slide(1).unwrap().occurrences.len(), 2, "revisit reuses slide 1");

    // --- slide files ---------------------------------------------------------------------
    for sl in &m.slides {
        let bytes = std::fs::read(s.dir.abs(&sl.file)).unwrap();
        assert_eq!(lc_core::export::sha256_hex(&bytes), sl.sha256);
        assert_eq!(lc_core::export::png_dimensions(&bytes), Some((W, H)), "native resolution");
        assert!(!sl.file.contains('\\') && !sl.file.starts_with('/'));
    }
    assert_eq!(std::fs::read_dir(s.dir.slides_dir()).unwrap().count(), 3, "no duplicate images");
    // slide 3 image is the fully revealed version
    let png = image_luma_rows(&s.dir.abs(&c.file));
    let expected = c4.to_rgb8();
    assert_eq!(png.len(), expected.len());
    assert!(png == expected, "PNG must be bit-exact (lossless)");

    // --- audio ---------------------------------------------------------------------------
    let audio_ms = recovery::ogg_tail_duration_ms(&s.dir.recording_path()).unwrap();
    assert!(audio_ms.abs_diff(75_000) <= 1_500, "audio duration {audio_ms}");

    // --- transcript ----------------------------------------------------------------------
    let segs: Vec<SegmentRecord> = read_jsonl(&s.dir.segments_path()).unwrap();
    assert!(!segs.is_empty());
    let words: Vec<_> = segs.iter().flat_map(|r| r.words.iter()).collect();
    for w in &words {
        let mid = w.mid();
        assert!(
            spans.iter().any(|(a, b)| mid + 600 >= *a && mid <= b + 600),
            "word at {mid} ms outside speech"
        );
        assert!(!(60_400..64_600).contains(&mid), "speech during pause must not be transcribed ({mid})");
    }
    // speech right before Stop (66–75 s) was flushed and transcribed
    assert!(words.iter().any(|w| w.mid() > 72_000), "last utterance lost on stop");
    assert!(m.transcription.status == TranscriptionStatus::Completed);
    assert_eq!(m.transcription.chunks_done, m.transcription.chunks_total);
    assert!(std::fs::read_dir(s.dir.pending_dir()).unwrap().count() == 0, "queue drained");

    // --- slide ↔ speech sync -------------------------------------------------------------
    let data = load_transcript(&s).unwrap();
    assert_eq!(data.assignment.word_count(), data.records.iter().map(|r| r.words.len()).sum::<usize>());
    for sp in &data.assignment.slides {
        for p in &sp.parts {
            for w in &p.words {
                assert!(w.mid() >= sp.start_ms && w.mid() < sp.end_ms, "word {} not inside its slide interval", w.mid());
            }
        }
    }
    let crossing = data.assignment.slides.iter().flat_map(|s| s.parts.iter()).any(|p| p.continues_to_next);
    assert!(crossing, "an utterance spanning a slide change must be split and marked");

    // --- documents ----------------------------------------------------------------------
    let lecture = std::fs::read_to_string(s.dir.abs("lecture.md")).unwrap();
    assert!(lecture.contains("# Algorytmy i struktury danych"));
    assert!(lecture.contains("![Slajd 1](slides/001.png)"));
    assert!(lecture.contains("ponownie"));
    assert!(lecture.contains("pauza"));
    assert!(lecture.contains("| Liczba slajdów | 3 (wyświetleń na osi czasu: 5) |"));
    let prompt = std::fs::read_to_string(s.dir.abs("PROMPT.md")).unwrap();
    for needle in ["manifest.json", "WSZYSTKIE slajdy (3)", "transcript/full.md", "notes.md", "summary.md", "exam-questions.md", "flashcards.md", "Nie wymyślaj"] {
        assert!(prompt.contains(needle), "PROMPT.md lacks {needle}");
    }
    let by_slide = std::fs::read_to_string(s.dir.abs("transcript/by-slide.md")).unwrap();
    assert!(by_slide.contains("](../slides/002.png)"));
    let full = std::fs::read_to_string(s.dir.abs("transcript/full.md")).unwrap();
    assert!(full.contains("**[00:01]**") || full.contains("**[00:00]**"));
    assert!(!lecture.contains(&tmp.path().to_string_lossy().to_string()), "no absolute paths in documents");
    assert!(!s.dir.lock_path().exists());
    // manifest is valid JSON for other tools
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(s.dir.manifest_path()).unwrap()).unwrap();
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["timeline"][1]["slide_id"], 2);
}

/// Decode a PNG to RGB rows for bit-exact comparison.
fn image_luma_rows(p: &std::path::Path) -> Vec<u8> {
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(p).unwrap()));
    let mut r = dec.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    buf
}

#[test]
fn missing_audio_signal_is_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let silence = vec![0.0f32; 16_000 * 45];
    let (rec, mut d) = start(tmp.path(), config(tmp.path()), None, &silence);
    let a = slide(W, H, 3, 3);
    d.run_until(45_000, |_| Some(a.clone()));
    let st = rec.status();
    assert!(st.audio.no_signal, "no-signal flag");
    assert!(st.warnings.iter().any(|w| w.message.contains("Brak sygnału audio")));
    let out = rec.stop().unwrap();
    let s = LectureSession::open(&out.lecture_dir).unwrap();
    let g = s.manifest.gaps.iter().find(|g| g.kind == GapKind::NoAudioSignal).expect("gap recorded");
    assert!(g.start_ms <= 1_000);
    let lecture = std::fs::read_to_string(s.dir.abs("lecture.md")).unwrap();
    assert!(lecture.contains("brak sygnału audio"));
    assert_eq!(s.manifest.transcription.status, TranscriptionStatus::Disabled);
}

#[test]
fn transcription_errors_are_isolated() {
    let tmp = tempfile::tempdir().unwrap();
    let (speech, _) = speech_like(&[(500, false), (4000, true), (1500, false), (4000, true), (1000, false)], 7);
    let (factory, _) = energy_factory(true);
    let (rec, mut d) = start(tmp.path(), config(tmp.path()), Some(factory), &speech);
    let a = slide(W, H, 4, 3);
    d.run_until(11_000, |_| Some(a.clone()));
    let out = rec.stop().unwrap();
    wait_transcription(out.transcription.as_ref().unwrap());
    let s = LectureSession::open(&out.lecture_dir).unwrap();
    // capture is unaffected
    assert_eq!(s.manifest.slides.len(), 1);
    assert!(s.manifest.transcription.chunks_failed >= 2);
    assert_eq!(s.manifest.transcription.status, TranscriptionStatus::Partial);
    assert!(s.manifest.gaps.iter().any(|g| g.kind == GapKind::TranscriptionFailed));
    // failed chunks are kept for a later retry, audio is kept
    assert!(s.dir.pending_dir().join("failed").read_dir().unwrap().count() >= 4);
    assert!(s.dir.recording_path().exists());
    let lecture = std::fs::read_to_string(s.dir.abs("lecture.md")).unwrap();
    assert!(lecture.contains("nieudana transkrypcja fragmentu"));
}

#[test]
fn model_load_failure_keeps_queue_for_later() {
    let tmp = tempfile::tempdir().unwrap();
    let (speech, _) = speech_like(&[(500, false), (3000, true), (1500, false)], 9);
    let factory: TranscriberFactory = Box::new(|| anyhow::bail!("model file missing"));
    let (rec, mut d) = start(tmp.path(), config(tmp.path()), Some(factory), &speech);
    let a = slide(W, H, 5, 3);
    d.run_until(5_000, |_| Some(a.clone()));
    let out = rec.stop().unwrap();
    wait_transcription(out.transcription.as_ref().unwrap());
    let s = LectureSession::open(&out.lecture_dir).unwrap();
    assert_eq!(s.manifest.transcription.status, TranscriptionStatus::Failed);
    assert!(s.manifest.transcription.error.as_deref().unwrap().contains("model file missing"));
    assert!(s.dir.pending_dir().read_dir().unwrap().count() >= 2, "chunk kept on disk");
    // later: model available → resume the queue
    let session = Arc::new(parking_lot::Mutex::new(s));
    let (factory, _) = energy_factory(false);
    let svc = TranscriptionService::start(session.clone(), factory, true, None, None);
    svc.finish();
    wait_transcription(&svc);
    let s = session.lock();
    assert_eq!(s.manifest.transcription.status, TranscriptionStatus::Completed);
    let segs: Vec<SegmentRecord> = read_jsonl(&s.dir.segments_path()).unwrap();
    assert!(!segs.is_empty());
}

#[test]
fn crash_recovery_and_transcription_resume() {
    let tmp = tempfile::tempdir().unwrap();
    let (speech, _) = speech_like(&[(1000, false), (6000, true), (1000, false), (6000, true), (6000, false)], 11);
    // "live" transcription disabled: everything is still queued when we crash
    let mut cfg = config(tmp.path());
    cfg.transcription.live = false;
    let (factory, _) = energy_factory(false);
    let (rec, mut d) = start(tmp.path(), cfg, Some(factory), &speech);
    let a = slide(W, H, 21, 3);
    let b = slide(W, H, 22, 5);
    d.run_until(10_000, |_| Some(a.clone()));
    d.run_until(20_000, |_| Some(b.clone()));
    let dir = rec.simulate_crash();

    // a PNG that was written but never registered (crash between the two writes)
    let orphan = Frame::solid(640, 360, [10, 200, 30]);
    lc_core::export::write_slide_png(&orphan, &dir.join("slides").join("003.png"), PngCompression::Fast).unwrap();
    // the lock belongs to a process that no longer exists
    std::fs::write(dir.join(".lc").join("recording.lock"), "999999").unwrap();
    assert!(recovery::needs_recovery(&dir));
    let report = recovery::recover(&dir).unwrap();
    assert_eq!(report.orphan_slides_added, vec![3]);
    assert!(report.pending_chunks >= 1);
    assert!(report.end_ms >= 15_000, "recovered end {}", report.end_ms);
    assert!(!recovery::needs_recovery(&dir));

    let s = LectureSession::open(&dir).unwrap();
    assert_eq!(s.manifest.lecture.status, LectureStatus::Recovered);
    assert_eq!(s.manifest.slides.len(), 3);
    assert_eq!(s.manifest.timeline.iter().map(|o| o.slide_id).collect::<Vec<_>>(), vec![1, 2]);
    assert!(s.manifest.timeline.iter().all(|o| o.end_ms.is_some()));
    assert!(s.manifest.gaps.iter().any(|g| g.kind == GapKind::Crash));
    // the audio written before the crash is readable
    let n = lc_core::audio::opus::decode_ogg_opus(&s.dir.recording_path(), |_| Ok(())).unwrap();
    assert!(n >= 16_000 * 14, "decoded {} samples", n);

    // resume the unfinished transcription
    let session = Arc::new(parking_lot::Mutex::new(s));
    let (factory, _) = energy_factory(false);
    let svc = TranscriptionService::start(session.clone(), factory, true, None, None);
    svc.finish();
    wait_transcription(&svc);
    let s = session.lock();
    let segs: Vec<SegmentRecord> = read_jsonl(&s.dir.segments_path()).unwrap();
    assert!(segs.len() >= 2);
    assert!(s.dir.abs("lecture.md").exists() && s.dir.abs("PROMPT.md").exists());
    let lecture = std::fs::read_to_string(s.dir.abs("lecture.md")).unwrap();
    assert!(lecture.contains("odzyskany"));
}

#[test]
fn retranscription_from_recording() {
    let tmp = tempfile::tempdir().unwrap();
    let (speech, _) = speech_like(&[(1000, false), (5000, true), (1000, false), (5000, true), (1000, false)], 13);
    let mut cfg = config(tmp.path());
    cfg.transcription.enabled = false;
    let (rec, mut d) = start(tmp.path(), cfg, None, &speech);
    let a = slide(W, H, 31, 3);
    d.run_until(13_000, |_| Some(a.clone()));
    let out = rec.stop().unwrap();
    let mut s = LectureSession::open(&out.lecture_dir).unwrap();
    s.manifest.transcription.enabled = true;
    let session = Arc::new(parking_lot::Mutex::new(s));
    let (factory, _) = energy_factory(false);
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut last = 0.0;
    let n = lc_core::pipeline::transcription::retranscribe(&session, factory, VadConfig::default(), &cancel, |p, _| last = p)
        .unwrap();
    assert!(n >= 2);
    assert!(last > 0.9);
    let s = session.lock();
    assert_eq!(s.manifest.transcription.status, TranscriptionStatus::Completed);
    let segs: Vec<SegmentRecord> = read_jsonl(&s.dir.segments_path()).unwrap();
    // words are positioned on the lecture timeline (speech starts at 1 s)
    let first = segs.iter().flat_map(|r| r.words.iter()).map(|w| w.s).min().unwrap();
    assert!(first.abs_diff(1_000) < 700, "first word at {first}");
}

#[test]
fn post_recording_edits() {
    let tmp = tempfile::tempdir().unwrap();
    let (speech, _) = speech_like(&[(1000, false), (20000, true), (1000, false)], 17);
    let (factory, _) = energy_factory(false);
    let (rec, mut d) = start(tmp.path(), config(tmp.path()), Some(factory), &speech);
    let (a, b, c) = (slide(W, H, 41, 3), slide(W, H, 42, 5), slide(W, H, 43, 2));
    d.run_until(8_000, |_| Some(a.clone()));
    d.run_until(10_000, |_| Some(b.clone())); // a wrongly captured flash
    d.run_until(22_000, |_| Some(c.clone()));
    let out = rec.stop().unwrap();
    wait_transcription(out.transcription.as_ref().unwrap());
    let mut s = LectureSession::open(&out.lecture_dir).unwrap();
    assert_eq!(s.manifest.slides.len(), 3);
    let words_before = load_transcript(&s).unwrap().assignment.word_count();

    s.delete_slide(2).unwrap();
    assert!(!s.dir.abs("slides/002.png").exists());
    assert_eq!(s.manifest.timeline.iter().map(|o| o.slide_id).collect::<Vec<_>>(), vec![1, 3]);
    // the deleted slide's time went to slide 1, no speech lost
    let data = load_transcript(&s).unwrap();
    assert_eq!(data.assignment.word_count(), words_before);
    assert!(s.manifest.timeline[0].end_ms.unwrap().abs_diff(10_000) <= 600);

    let occ = s.manifest.timeline[1].id.clone();
    s.set_occurrence_start(&occ, 12_000).unwrap();
    assert_eq!(s.manifest.timeline[0].end_ms, Some(12_000));
    assert!(s.set_occurrence_start(&occ, 999_999).is_err());
    lc_core::export::write_documents(&s).unwrap();

    let zip_path = tmp.path().join("export.zip");
    let n = lc_core::export::zip::export_zip(&s.dir.root, &zip_path, true).unwrap();
    assert!(n >= 8);
    let z = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
    let names: Vec<String> = z.file_names().map(|s| s.to_string()).collect();
    assert!(names.iter().any(|n| n.ends_with("/PROMPT.md")));
    assert!(names.iter().any(|n| n.ends_with("/slides/001.png")));
    assert!(!names.iter().any(|n| n.contains("/.lc/")));
}
