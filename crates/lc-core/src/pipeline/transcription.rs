//! Transcription worker fed from the on-disk queue `.lc/pending/` (crash-safe and
//! independent from capture), plus full re-transcription from `audio/recording.ogg`.

use super::traits::{Transcriber, TranscriberFactory};
use crate::audio::vad::{Segmenter, SpeechChunk};
use crate::audio::wav::{read_wav, write_wav};
use crate::clock::Clock;
use crate::config::{AudioRetention, VadConfig};
use crate::session::fsutil::{append_line, atomic_write};
use crate::session::manifest::{GapKind, Gap, TranscriptionStatus};
use crate::session::{LectureDir, LectureSession};
use crate::transcript::{finalize_chunk, ChunkMeta, SegmentRecord};
use anyhow::{Context, Result};
use crossbeam_channel::{bounded, Receiver, Sender};
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize, Default)]
pub struct TranscriptionStatusView {
    /// `disabled`, `waiting`, `loading`, `running`, `idle`, `done`, `failed`, `cancelled`
    pub state: String,
    pub model: String,
    pub queue_len: usize,
    pub done: u64,
    pub failed: u64,
    /// How far transcription lags behind live audio.
    pub lag_ms: u64,
    pub error: Option<String>,
    /// 0..1 for re-transcription from the recording.
    pub progress: Option<f32>,
    /// Audio seconds processed per wall second (higher = faster than real time).
    pub speed: f32,
    pub last_text: Option<String>,
}

pub type SegmentCallback = Arc<dyn Fn(&SegmentRecord) + Send + Sync>;

/// Persist a speech chunk into the transcription queue.
pub fn enqueue_chunk(dir: &LectureDir, chunk: &SpeechChunk) -> Result<()> {
    let pending = dir.pending_dir();
    std::fs::create_dir_all(&pending)?;
    let meta = ChunkMeta {
        chunk_id: chunk.id,
        start_ms: chunk.start / 16,
        keep_from_ms: chunk.keep_from / 16,
        end_ms: chunk.end() / 16,
    };
    // WAV first, then the metadata that makes the chunk visible to the worker
    write_wav(&pending.join(format!("{:06}.wav", chunk.id)), &chunk.samples, 16_000)?;
    atomic_write(&pending.join(format!("{:06}.json", chunk.id)), &serde_json::to_vec(&meta)?)?;
    Ok(())
}

fn list_pending(dir: &Path) -> Vec<(ChunkMeta, PathBuf, PathBuf)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "json") {
            let wav = p.with_extension("wav");
            if let Ok(text) = std::fs::read_to_string(&p) {
                if let Ok(meta) = serde_json::from_str::<ChunkMeta>(&text) {
                    if wav.exists() {
                        out.push((meta, p, wav));
                    }
                }
            }
        }
    }
    out.sort_by_key(|(m, _, _)| m.chunk_id);
    out
}

fn transcribe_chunk(
    tr: &mut Box<dyn Transcriber>,
    meta: &ChunkMeta,
    pcm: &[f32],
    cancel: &AtomicBool,
) -> Result<Vec<SegmentRecord>> {
    let mut audio = pcm.to_vec();
    if audio.len() < 16_000 + 1_600 {
        audio.resize(16_000 + 1_600, 0.0); // whisper needs at least ~1 s
    }
    let out = tr.transcribe(&audio, cancel)?;
    Ok(finalize_chunk(meta, &out.segments, out.language, &tr.model_name()))
}

pub fn lower_thread_priority() {
    #[cfg(target_os = "macos")]
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

struct Shared {
    session: Arc<Mutex<LectureSession>>,
    status: Mutex<TranscriptionStatusView>,
    wake_tx: Sender<()>,
    wake_rx: Receiver<()>,
    processing: AtomicBool,
    finish_when_empty: AtomicBool,
    cancel: AtomicBool,
    finished: AtomicBool,
    clock: Option<Arc<dyn Clock>>,
    on_segment: Option<SegmentCallback>,
}

/// Queue worker. Created when recording starts (or to resume pending chunks).
pub struct TranscriptionService {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl TranscriptionService {
    pub fn start(
        session: Arc<Mutex<LectureSession>>,
        factory: TranscriberFactory,
        process_now: bool,
        clock: Option<Arc<dyn Clock>>,
        on_segment: Option<SegmentCallback>,
    ) -> Arc<Self> {
        let (wake_tx, wake_rx) = bounded(64);
        let model = session.lock().manifest.transcription.model.clone();
        let shared = Arc::new(Shared {
            session,
            status: Mutex::new(TranscriptionStatusView {
                state: if process_now { "idle".into() } else { "waiting".into() },
                model,
                ..Default::default()
            }),
            wake_tx,
            wake_rx,
            processing: AtomicBool::new(process_now),
            finish_when_empty: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            clock,
            on_segment,
        });
        let s2 = shared.clone();
        let handle = std::thread::Builder::new()
            .name("lc-transcribe".into())
            .spawn(move || {
                lower_thread_priority();
                worker(&s2, factory);
                s2.finished.store(true, Ordering::SeqCst);
            })
            .expect("spawn transcription thread");
        Arc::new(Self { shared, thread: Mutex::new(Some(handle)) })
    }

    pub fn notify(&self) {
        let _ = self.shared.wake_tx.try_send(());
    }

    /// Start processing (for "transcribe after the lecture" mode).
    pub fn begin_processing(&self) {
        self.shared.processing.store(true, Ordering::SeqCst);
        self.notify();
    }

    /// Process the remaining queue, then finalize and exit.
    pub fn finish(&self) {
        self.shared.finish_when_empty.store(true, Ordering::SeqCst);
        self.begin_processing();
    }

    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::SeqCst);
        self.notify();
    }

    pub fn status(&self) -> TranscriptionStatusView {
        self.shared.status.lock().clone()
    }

    pub fn is_finished(&self) -> bool {
        self.shared.finished.load(Ordering::SeqCst)
    }

    pub fn join(&self) {
        if let Some(h) = self.thread.lock().take() {
            let _ = h.join();
        }
    }
}

fn set_state(sh: &Shared, state: &str) {
    sh.status.lock().state = state.into();
}

fn worker(sh: &Shared, factory: TranscriberFactory) {
    let dir = sh.session.lock().dir.clone();
    let pending_dir = dir.pending_dir();
    let mut factory = Some(factory);
    let mut tr: Option<Box<dyn Transcriber>> = None;
    let mut attempts: HashMap<u64, u32> = HashMap::new();
    let mut load_failed = false;

    loop {
        if sh.cancel.load(Ordering::SeqCst) {
            set_state(sh, "cancelled");
            return;
        }
        if !sh.processing.load(Ordering::SeqCst) {
            let _ = sh.wake_rx.recv_timeout(Duration::from_millis(500));
            continue;
        }
        let pending = list_pending(&pending_dir);
        {
            let mut st = sh.status.lock();
            st.queue_len = pending.len();
            st.lag_ms = match (&sh.clock, pending.first()) {
                (Some(c), Some((m, _, _))) => c.now_ms().saturating_sub(m.end_ms),
                _ => 0,
            };
        }
        let Some((meta, json, wav)) = pending.into_iter().next() else {
            if sh.finish_when_empty.load(Ordering::SeqCst) {
                break;
            }
            set_state(sh, "idle");
            let _ = sh.wake_rx.recv_timeout(Duration::from_millis(1000));
            continue;
        };
        if tr.is_none() {
            set_state(sh, "loading");
            match factory.take().map(|f| f()) {
                Some(Ok(t)) => {
                    sh.status.lock().model = t.model_name();
                    let mut s = sh.session.lock();
                    s.manifest.transcription.model = t.model_name();
                    s.manifest.transcription.engine = t.engine_name();
                    s.manifest.transcription.status = TranscriptionStatus::Running;
                    let _ = s.save();
                    drop(s);
                    tr = Some(t);
                }
                Some(Err(e)) => {
                    let msg = format!("Nie udało się załadować modelu: {e:#}");
                    log::error!("{msg}");
                    let mut st = sh.status.lock();
                    st.state = "failed".into();
                    st.error = Some(msg.clone());
                    drop(st);
                    let mut s = sh.session.lock();
                    s.manifest.transcription.status = TranscriptionStatus::Failed;
                    s.manifest.transcription.error = Some(msg);
                    let _ = s.save();
                    load_failed = true;
                    break;
                }
                None => break,
            }
        }
        let t = tr.as_mut().unwrap();
        set_state(sh, "running");
        let started = Instant::now();
        let result = read_wav(&wav).and_then(|(pcm, _)| transcribe_chunk(t, &meta, &pcm, &sh.cancel));
        match result {
            Ok(records) => {
                let segs = dir.segments_path();
                let mut ok = true;
                for r in &records {
                    match serde_json::to_string(r).map_err(anyhow::Error::from).and_then(|l| append_line(&segs, &l)) {
                        Ok(()) => {}
                        Err(e) => {
                            ok = false;
                            log::error!("cannot write segments.jsonl: {e:#}");
                        }
                    }
                }
                if !ok {
                    let mut st = sh.status.lock();
                    st.error = Some("Nie można zapisać transkrypcji do folderu wykładu".into());
                    drop(st);
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
                let _ = std::fs::remove_file(&json);
                let _ = std::fs::remove_file(&wav);
                let audio_s = (meta.end_ms - meta.start_ms) as f32 / 1000.0;
                let wall = started.elapsed().as_secs_f32().max(0.001);
                {
                    let mut st = sh.status.lock();
                    st.done += 1;
                    st.speed = audio_s / wall;
                    st.error = None;
                    if let Some(r) = records.last() {
                        st.last_text = Some(r.text.clone());
                    }
                }
                let mut s = sh.session.lock();
                s.manifest.transcription.chunks_done += 1;
                for r in &records {
                    if let Some(l) = &r.lang {
                        if !s.manifest.transcription.detected_languages.contains(l) {
                            s.manifest.transcription.detected_languages.push(l.clone());
                        }
                    }
                }
                let _ = s.save();
                drop(s);
                if let Some(cb) = &sh.on_segment {
                    for r in &records {
                        cb(r);
                    }
                }
            }
            Err(e) => {
                if sh.cancel.load(Ordering::SeqCst) {
                    continue;
                }
                let n = attempts.entry(meta.chunk_id).or_insert(0);
                *n += 1;
                log::warn!("chunk {} failed (attempt {}): {e:#}", meta.chunk_id, n);
                if *n >= 2 {
                    let failed_dir = pending_dir.join("failed");
                    let _ = std::fs::create_dir_all(&failed_dir);
                    let _ = std::fs::rename(&wav, failed_dir.join(wav.file_name().unwrap()));
                    let _ = std::fs::rename(&json, failed_dir.join(json.file_name().unwrap()));
                    sh.status.lock().failed += 1;
                    sh.status.lock().error = Some(format!("{e:#}"));
                    let mut s = sh.session.lock();
                    s.manifest.transcription.chunks_failed += 1;
                    s.manifest.gaps.push(Gap {
                        kind: GapKind::TranscriptionFailed,
                        start_ms: meta.start_ms,
                        end_ms: Some(meta.end_ms),
                        detail: Some(format!("{e:#}")),
                    });
                    let _ = s.save();
                }
            }
        }
    }
    if !load_failed {
        finalize(sh);
    }
}

/// Mark transcription complete, regenerate documents and apply audio retention.
fn finalize(sh: &Shared) {
    let mut s = sh.session.lock();
    let remaining = list_pending(&s.dir.pending_dir()).len();
    let failed = s.manifest.transcription.chunks_failed;
    s.manifest.transcription.status = if remaining > 0 {
        TranscriptionStatus::Pending
    } else if failed > 0 {
        TranscriptionStatus::Partial
    } else {
        TranscriptionStatus::Completed
    };
    s.manifest.transcription.completed_at = Some(chrono::Local::now().fixed_offset());
    if s.manifest.transcription.status == TranscriptionStatus::Completed
        && s.manifest.audio.retention == AudioRetention::DeleteAfterTranscription
        && s.manifest.lecture.status != crate::session::manifest::LectureStatus::Recording
    {
        let p = s.dir.recording_path();
        if std::fs::remove_file(&p).is_ok() {
            s.manifest.audio.file = None;
            let t = s.manifest.end_ms();
            s.event(t, "audio_deleted_after_transcription", serde_json::Value::Null);
        }
    }
    let _ = s.save();
    if s.manifest.lecture.status != crate::session::manifest::LectureStatus::Recording {
        if let Err(e) = crate::export::write_documents(&s) {
            log::error!("write documents: {e:#}");
        }
    }
    let st = s.manifest.transcription.status;
    drop(s);
    let mut v = sh.status.lock();
    v.state = if st == TranscriptionStatus::Completed || st == TranscriptionStatus::Partial { "done".into() } else { "failed".into() };
    v.queue_len = remaining;
}

/// Re-run the whole transcription from `audio/recording.ogg` (e.g. with a better
/// model or after a failure). Replaces `segments.jsonl` atomically at the end.
pub fn retranscribe(
    session: &Arc<Mutex<LectureSession>>,
    factory: TranscriberFactory,
    vad: VadConfig,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32, Option<&SegmentRecord>),
) -> Result<usize> {
    let (dir, rec_rel) = {
        let s = session.lock();
        (s.dir.clone(), s.manifest.audio.file.clone())
    };
    let rec_rel = rec_rel.context("Nagranie audio nie jest dostępne (usunięte po transkrypcji)")?;
    let ogg = dir.abs(&rec_rel);
    let total_ms = crate::session::recovery::ogg_tail_duration_ms(&ogg).unwrap_or(0).max(1);
    let mut tr = factory()?;
    {
        let mut s = session.lock();
        s.manifest.transcription.status = TranscriptionStatus::Running;
        s.manifest.transcription.model = tr.model_name();
        s.manifest.transcription.engine = tr.engine_name();
        s.manifest.transcription.error = None;
        s.save()?;
    }
    let tmp = dir.abs("transcript/.segments.jsonl.new");
    let _ = std::fs::remove_file(&tmp);
    let mut seg = Segmenter::new(vad, 0, 1);
    let mut count = 0usize;
    let mut failed = 0u64;
    let mut decoded = 0u64;
    let mut handle = |chunk: Option<SpeechChunk>, tr: &mut Box<dyn Transcriber>, decoded: u64| -> Result<()> {
        let Some(chunk) = chunk else {
            progress((decoded / 16) as f32 / total_ms as f32, None);
            return Ok(());
        };
        let meta = ChunkMeta {
            chunk_id: chunk.id,
            start_ms: chunk.start / 16,
            keep_from_ms: chunk.keep_from / 16,
            end_ms: chunk.end() / 16,
        };
        match transcribe_chunk(tr, &meta, &chunk.samples, cancel) {
            Ok(records) => {
                for r in &records {
                    append_line(&tmp, &serde_json::to_string(r)?)?;
                    progress((decoded / 16) as f32 / total_ms as f32, Some(r));
                }
                count += 1;
            }
            Err(e) => {
                if cancel.load(Ordering::SeqCst) {
                    anyhow::bail!("anulowano");
                }
                log::warn!("chunk {} failed: {e:#}", meta.chunk_id);
                failed += 1;
            }
        }
        Ok(())
    };
    crate::audio::opus::decode_ogg_opus(&ogg, |block| {
        if cancel.load(Ordering::SeqCst) {
            anyhow::bail!("anulowano");
        }
        decoded += block.len() as u64;
        for c in seg.push(block) {
            handle(Some(c), &mut tr, decoded)?;
        }
        handle(None, &mut tr, decoded)
    })?;
    if let Some(c) = seg.flush() {
        handle(Some(c), &mut tr, decoded)?;
    }
    drop(handle);
    if !tmp.exists() {
        std::fs::write(&tmp, b"")?;
    }
    std::fs::rename(&tmp, dir.segments_path())?;
    // the full re-run supersedes any queued live chunks
    let _ = std::fs::remove_dir_all(dir.pending_dir());
    let _ = std::fs::create_dir_all(dir.pending_dir());
    let mut s = session.lock();
    s.manifest.gaps.retain(|g| g.kind != GapKind::TranscriptionFailed);
    s.manifest.transcription.chunks_total = (count as u64) + failed;
    s.manifest.transcription.chunks_done = count as u64;
    s.manifest.transcription.chunks_failed = failed;
    s.manifest.transcription.status =
        if failed > 0 { TranscriptionStatus::Partial } else { TranscriptionStatus::Completed };
    s.manifest.transcription.completed_at = Some(chrono::Local::now().fixed_offset());
    let t = s.manifest.end_ms();
    s.event(t, "retranscribed", serde_json::json!({ "chunks": count, "failed": failed }));
    s.save()?;
    crate::export::write_documents(&s)?;
    Ok(count)
}
