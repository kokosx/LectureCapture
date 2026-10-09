//! Crash recovery: finalize lectures whose recorder died (crash, forced quit, power
//! loss, lost permissions) so that every correctly written file stays usable and the
//! unfinished transcription can be resumed.

use super::layout::LectureDir;
use super::manifest::*;
use super::store::LectureSession;
use anyhow::Result;
use serde::Serialize;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct RecoveryReport {
    pub folder: String,
    pub end_ms: u64,
    pub missing_slides_removed: Vec<u32>,
    pub orphan_slides_added: Vec<u32>,
    pub pending_chunks: usize,
}

/// A lecture needs recovery if it is still marked as recording but no live process
/// holds its lock.
pub fn needs_recovery(root: &Path) -> bool {
    let dir = LectureDir::open(root);
    let Ok(text) = std::fs::read_to_string(dir.manifest_path()) else { return false };
    let Ok(m) = serde_json::from_str::<Manifest>(&text) else { return false };
    if m.lecture.status != LectureStatus::Recording {
        return false;
    }
    match std::fs::read_to_string(dir.lock_path()).ok().and_then(|s| s.trim().parse::<u32>().ok()) {
        Some(pid) if pid == std::process::id() => false,
        Some(pid) => !crate::util::pid_alive(pid),
        None => true,
    }
}

/// Fast duration of an Ogg/Opus file from the granule position of its last page.
pub fn ogg_tail_duration_ms(path: &Path) -> Option<u64> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut head = vec![0u8; 512.min(len as usize)];
    f.read_exact(&mut head).ok()?;
    let pos = head.windows(8).position(|w| w == b"OpusHead")?;
    let pre_skip = u16::from_le_bytes([*head.get(pos + 10)?, *head.get(pos + 11)?]) as u64;
    let tail_len = len.min(256 * 1024);
    f.seek(SeekFrom::Start(len - tail_len)).ok()?;
    let mut tail = vec![0u8; tail_len as usize];
    f.read_exact(&mut tail).ok()?;
    let mut i = tail.len().saturating_sub(27);
    loop {
        if &tail[i..i + 4] == b"OggS" && tail[i + 4] == 0 {
            let g = u64::from_le_bytes(tail[i + 6..i + 14].try_into().ok()?);
            if g != u64::MAX && g > 0 {
                return Some(g.saturating_sub(pre_skip) / 48);
            }
        }
        if i == 0 {
            return None;
        }
        i -= 1;
    }
}

pub fn recover(root: &Path) -> Result<RecoveryReport> {
    let mut s = LectureSession::open(root)?;
    let dir = s.dir.clone();

    // 1. how far did we get?
    let mut end_ms = s.manifest.lecture.last_alive_ms;
    if let Some(e) = s.manifest.events.last() {
        end_ms = end_ms.max(e.t_ms);
    }
    if let Some(o) = s.manifest.timeline.last() {
        end_ms = end_ms.max(o.start_ms);
    }
    let audio_ms = ogg_tail_duration_ms(&dir.recording_path());
    if let Some(a) = audio_ms {
        end_ms = end_ms.max(a);
        s.manifest.audio.duration_ms = Some(a);
    }
    // segments already transcribed tell us audio existed at least until then
    let segs: Vec<serde_json::Value> = super::fsutil::read_jsonl(&dir.segments_path()).unwrap_or_default();
    if let Some(max_end) = segs.iter().filter_map(|v| v.get("end_ms").and_then(|x| x.as_u64())).max() {
        end_ms = end_ms.max(max_end);
    }

    // 2. slides listed but missing on disk (crash before PNG rename) are dropped
    let missing: Vec<u32> =
        s.manifest.slides.iter().filter(|sl| !dir.abs(&sl.file).exists()).map(|sl| sl.id).collect();
    for id in &missing {
        s.manifest.slides.retain(|sl| sl.id != *id);
        s.manifest.timeline.retain(|o| o.slide_id != *id);
    }

    // 3. PNGs written but never registered (crash between PNG and manifest write)
    let mut orphans = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir.slides_dir()) {
        let mut files: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        files.sort();
        for p in files {
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else { continue };
            if p.extension().and_then(|e| e.to_str()) != Some("png") {
                continue;
            }
            let Ok(id) = stem.parse::<u32>() else { continue };
            if s.manifest.slide(id).is_some() {
                continue;
            }
            let bytes = std::fs::read(&p).unwrap_or_default();
            let (w, h) = crate::export::png_dimensions(&bytes).unwrap_or((0, 0));
            let captured_ms = s.manifest.lecture.last_alive_ms;
            s.manifest.slides.push(Slide {
                id,
                file: LectureDir::slide_rel(id),
                archive_file: None,
                sha256: crate::export::sha256_hex(&bytes),
                dhash: String::new(),
                width: w,
                height: h,
                bytes: bytes.len() as u64,
                captured_at: s.at(captured_ms),
                captured_ms,
                trigger: SlideTrigger::Recovered,
                build_of: None,
                updates: 0,
                occurrences: vec![],
                display_start_ms: None,
                display_end_ms: None,
            });
            orphans.push(id);
        }
        s.manifest.slides.sort_by_key(|sl| sl.id);
    }

    // 4. close everything at the last known moment
    let last_alive = s.manifest.lecture.last_alive_ms;
    s.manifest.gaps.push(Gap {
        kind: GapKind::Crash,
        start_ms: end_ms,
        end_ms: Some(end_ms),
        detail: Some(format!(
            "Nagrywanie przerwane nieoczekiwanie; ostatni potwierdzony zapis po {} od startu.",
            crate::util::fmt_ms(last_alive.max(end_ms))
        )),
    });
    s.manifest.lecture.notes.push(
        "Wykład odzyskany po nieoczekiwanym zakończeniu programu. Materiały zapisane do momentu awarii są kompletne."
            .into(),
    );
    if !orphans.is_empty() {
        s.manifest.lecture.notes.push(format!(
            "Slajdy {:?} zostały odzyskane z dysku bez informacji o czasie wyświetlania.",
            orphans
        ));
    }
    let pending = std::fs::read_dir(dir.pending_dir())
        .map(|rd| rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "wav")).count())
        .unwrap_or(0);
    if s.manifest.transcription.enabled && pending > 0 {
        s.manifest.transcription.status = TranscriptionStatus::Pending;
    }
    s.finish(end_ms, LectureStatus::Recovered)?;
    Ok(RecoveryReport {
        folder: dir.folder_name(),
        end_ms,
        missing_slides_removed: missing,
        orphan_slides_added: orphans,
        pending_chunks: pending,
    })
}
