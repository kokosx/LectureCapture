//! Energy-based voice activity detection and chunking for transcription.
//!
//! The stream is split into speech chunks at pauses. Continuous speech longer than
//! `max_chunk_ms` is cut at the quietest frame of the last `cut_search_ms`; the next
//! chunk then starts `overlap_ms` *before* the cut so whisper gets acoustic context,
//! and `keep_from` marks the cut so words already transcribed in the previous chunk
//! are dropped during merging (see [`crate::transcript`]).

use super::level::rms_db;
use crate::config::VadConfig;
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct SpeechChunk {
    pub id: u64,
    /// Timeline position (samples @16 kHz) of `samples[0]`.
    pub start: u64,
    /// Words that end before this position belong to the previous chunk.
    pub keep_from: u64,
    pub samples: Vec<f32>,
}

impl SpeechChunk {
    pub fn end(&self) -> u64 {
        self.start + self.samples.len() as u64
    }
}

enum State {
    Silence { onset_frames: u32 },
    Speech { start: u64, keep_from: u64, buf: Vec<f32>, energies: Vec<f32>, silence_frames: u32 },
}

pub struct Segmenter {
    cfg: VadConfig,
    frame_len: usize,
    pos: u64,
    partial: Vec<f32>,
    ring: VecDeque<f32>,
    floor_db: f32,
    state: State,
    next_id: u64,
    pub speech_frames: u64,
    pub total_frames: u64,
}

impl Segmenter {
    pub fn new(cfg: VadConfig, start_pos: u64, first_id: u64) -> Self {
        let frame_len = (cfg.frame_ms as usize * 16).max(16);
        Self {
            cfg,
            frame_len,
            pos: start_pos,
            partial: Vec::with_capacity(frame_len),
            ring: VecDeque::new(),
            floor_db: -70.0,
            state: State::Silence { onset_frames: 0 },
            next_id: first_id,
            speech_frames: 0,
            total_frames: 0,
        }
    }

    /// Timeline position of the next sample to be pushed.
    pub fn position(&self) -> u64 {
        self.pos + self.partial.len() as u64
    }

    pub fn in_speech(&self) -> bool {
        matches!(self.state, State::Speech { .. })
    }

    fn ms_to_frames(&self, ms: u32) -> u32 {
        (ms / self.cfg.frame_ms.max(1)).max(1)
    }

    pub fn push(&mut self, samples: &[f32]) -> Vec<SpeechChunk> {
        let mut out = Vec::new();
        let mut rest = samples;
        while !rest.is_empty() {
            let need = self.frame_len - self.partial.len();
            let take = need.min(rest.len());
            self.partial.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.partial.len() == self.frame_len {
                let frame = std::mem::take(&mut self.partial);
                if let Some(c) = self.process_frame(&frame) {
                    out.push(c);
                }
                self.pos += self.frame_len as u64;
                self.partial = frame;
                self.partial.clear();
            }
        }
        out
    }

    /// `n` samples of digital silence (gap / pause) without materializing them.
    pub fn push_silence(&mut self, n: u64) -> Vec<SpeechChunk> {
        let zeros = vec![0.0f32; self.frame_len];
        let mut out = Vec::new();
        let mut remaining = n;
        // complete the partial frame and let an open speech chunk close naturally
        while remaining > 0 && (!self.partial.is_empty() || self.in_speech()) {
            let k = (self.frame_len - self.partial.len()).min(remaining as usize);
            out.extend(self.push(&zeros[..k]));
            remaining -= k as u64;
        }
        self.pos += remaining;
        self.ring.clear();
        out
    }

    fn process_frame(&mut self, frame: &[f32]) -> Option<SpeechChunk> {
        self.total_frames += 1;
        let e = rms_db(frame);
        if e < self.floor_db {
            self.floor_db = self.floor_db * 0.8 + e * 0.2;
        } else {
            self.floor_db += 0.02 * self.cfg.frame_ms as f32 / 20.0;
        }
        self.floor_db = self.floor_db.max(-90.0);
        let is_speech = e > (self.floor_db + self.cfg.threshold_above_floor_db).max(self.cfg.absolute_threshold_db);
        if is_speech {
            self.speech_frames += 1;
        }
        let frame_pos = self.pos;
        let fl = self.frame_len;
        let preroll = self.cfg.preroll_ms as usize * 16;
        let min_speech_frames = self.ms_to_frames(self.cfg.min_speech_ms);
        let hangover_frames = self.ms_to_frames(self.cfg.hangover_ms);
        let max_len = self.cfg.max_chunk_ms as usize * 16;

        match &mut self.state {
            State::Silence { onset_frames } => {
                self.ring.extend(frame.iter().copied());
                let cap = preroll + (min_speech_frames as usize + 1) * fl;
                while self.ring.len() > cap {
                    self.ring.pop_front();
                }
                *onset_frames = if is_speech { *onset_frames + 1 } else { 0 };
                if *onset_frames >= min_speech_frames {
                    let keep = (preroll + *onset_frames as usize * fl).min(self.ring.len());
                    let skip = self.ring.len() - keep;
                    let buf: Vec<f32> = self.ring.iter().skip(skip).copied().collect();
                    let start = frame_pos + fl as u64 - buf.len() as u64;
                    let energies = buf.chunks(fl).map(rms_db).collect();
                    self.ring.clear();
                    self.state =
                        State::Speech { start, keep_from: start, buf, energies, silence_frames: 0 };
                }
                None
            }
            State::Speech { start, keep_from, buf, energies, silence_frames } => {
                buf.extend_from_slice(frame);
                energies.push(e);
                *silence_frames = if is_speech { 0 } else { *silence_frames + 1 };
                if *silence_frames >= hangover_frames {
                    // keep ~250 ms of the trailing silence
                    let trailing = *silence_frames as usize * fl;
                    let keep_tail = (250 * 16).min(trailing);
                    let new_len = buf.len() - trailing + keep_tail;
                    buf.truncate(new_len);
                    let chunk = SpeechChunk {
                        id: 0,
                        start: *start,
                        keep_from: *keep_from,
                        samples: std::mem::take(buf),
                    };
                    self.state = State::Silence { onset_frames: 0 };
                    return self.finish(chunk);
                }
                if buf.len() >= max_len {
                    let search = (self.cfg.cut_search_ms as usize * 16 / fl).max(1);
                    let nframes = energies.len();
                    let from = nframes.saturating_sub(search);
                    // quietest frame (prefer later ones on ties)
                    let mut k = from;
                    for j in from..nframes {
                        if energies[j] <= energies[k] {
                            k = j;
                        }
                    }
                    let k = k.max(1);
                    let cut_off = k * fl; // offset in buf
                    let overlap = (self.cfg.overlap_ms as usize * 16 / fl * fl).min(cut_off);
                    let new_off = cut_off - overlap;
                    let chunk = SpeechChunk {
                        id: 0,
                        start: *start,
                        keep_from: *keep_from,
                        samples: buf[..cut_off].to_vec(),
                    };
                    let cut_pos = *start + cut_off as u64;
                    *buf = buf[new_off..].to_vec();
                    *energies = energies[new_off / fl..].to_vec();
                    *start += new_off as u64;
                    *keep_from = cut_pos;
                    return self.finish(chunk);
                }
                None
            }
        }
    }

    fn finish(&mut self, mut chunk: SpeechChunk) -> Option<SpeechChunk> {
        let min = self.cfg.min_chunk_ms as usize * 16;
        // continuation chunks are always kept (they carry speech after a forced cut)
        if chunk.samples.len() < min && chunk.keep_from == chunk.start {
            return None;
        }
        chunk.id = self.next_id;
        self.next_id += 1;
        Some(chunk)
    }

    /// Close the current chunk (Stop / Pause).
    pub fn flush(&mut self) -> Option<SpeechChunk> {
        let partial = std::mem::take(&mut self.partial);
        self.pos += partial.len() as u64;
        let state = std::mem::replace(&mut self.state, State::Silence { onset_frames: 0 });
        self.ring.clear();
        match state {
            State::Speech { start, keep_from, mut buf, .. } => {
                buf.extend_from_slice(&partial);
                self.finish(SpeechChunk { id: 0, start, keep_from, samples: buf })
            }
            State::Silence { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::speech_like;

    fn run(samples: &[f32], cfg: VadConfig) -> Vec<SpeechChunk> {
        let mut s = Segmenter::new(cfg, 0, 1);
        let mut out = Vec::new();
        for c in samples.chunks(1234) {
            out.extend(s.push(c));
        }
        out.extend(s.flush());
        out
    }

    #[test]
    fn splits_at_pauses_with_preroll() {
        let (audio, spans) =
            speech_like(&[(1000, false), (3000, true), (1500, false), (2000, true), (1000, false)], 1);
        let chunks = run(&audio, VadConfig::default());
        assert_eq!(chunks.len(), 2, "{:?}", chunks.iter().map(|c| (c.start / 16, c.end() / 16)).collect::<Vec<_>>());
        for (c, (s, e)) in chunks.iter().zip(spans) {
            let (cs, ce) = (c.start / 16, c.end() / 16);
            assert!(cs <= s && s - cs <= 400, "chunk start {cs} vs speech {s}");
            assert!(ce >= e && ce - e <= 400, "chunk end {ce} vs speech {e}");
        }
    }

    #[test]
    fn silence_produces_no_chunks() {
        let (audio, _) = speech_like(&[(10_000, false)], 2);
        assert!(run(&audio, VadConfig::default()).is_empty());
    }

    #[test]
    fn long_speech_is_split_with_overlap_and_full_coverage() {
        let (audio, _) = speech_like(&[(500, false), (70_000, true), (1000, false)], 3);
        let cfg = VadConfig::default();
        let chunks = run(&audio, cfg.clone());
        assert!(chunks.len() >= 3);
        for c in &chunks {
            assert!(c.samples.len() <= cfg.max_chunk_ms as usize * 16 + 320);
        }
        for w in chunks.windows(2) {
            // next chunk starts before the cut (overlap) and keeps from exactly the cut
            assert_eq!(w[0].end(), w[1].keep_from, "no gap and no double coverage");
            assert!(w[1].start < w[1].keep_from);
            assert!(w[1].keep_from - w[1].start <= cfg.overlap_ms as u64 * 16);
        }
        // ids are sequential
        assert!(chunks.windows(2).all(|w| w[1].id == w[0].id + 1));
    }

    #[test]
    fn push_silence_closes_open_chunk_and_advances_position() {
        let (audio, _) = speech_like(&[(300, false), (2000, true)], 4);
        let mut s = Segmenter::new(VadConfig::default(), 0, 1);
        assert!(s.push(&audio).is_empty());
        let chunks = s.push_silence(16_000 * 3600);
        assert_eq!(chunks.len(), 1);
        assert_eq!(s.position(), audio.len() as u64 + 16_000 * 3600);
    }
}
