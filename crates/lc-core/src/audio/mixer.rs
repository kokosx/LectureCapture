//! Clock-aligned mixer for independent audio sources (system loopback + optional
//! microphone).
//!
//! Each source delivers 16 kHz mono blocks at its own pace. A block is *placed* on the
//! lecture timeline right after the previous block of the same source as long as that
//! stays within `resync` of the wall clock; otherwise (gap after sleep, device switch,
//! long stall, clock drift) the source is re-anchored to the wall clock. Sources are
//! summed into an accumulation buffer and emitted `latency` behind real time, so the
//! output sample index always corresponds to lecture time (`index / 16 = ms`).
//! Regions where no source delivered anything are emitted as [`Block::Silence`]
//! without allocating memory (hours of sleep cost nothing).

use super::level::LevelMeter;
use super::SAMPLES_PER_MS;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    System,
    Microphone,
}

#[derive(Debug, PartialEq)]
pub enum Block {
    Samples(Vec<f32>),
    Silence(u64),
}

impl Block {
    pub fn len(&self) -> u64 {
        match self {
            Block::Samples(s) => s.len() as u64,
            Block::Silence(n) => *n,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceStats {
    pub kind: SourceKind,
    pub enabled: bool,
    pub received_samples: u64,
    pub last_push_ms: Option<u64>,
    pub level_db: f32,
    pub resyncs: u32,
}

struct Source {
    kind: SourceKind,
    gain: f32,
    enabled: bool,
    next_pos: Option<u64>,
    last_push_ms: Option<u64>,
    received: u64,
    resyncs: u32,
    meter: LevelMeter,
}

pub struct Mixer {
    sources: Vec<Source>,
    /// Samples handed out so far (== lecture-time position of the next output sample).
    written: u64,
    /// Timeline position of `acc[0]`; positions in `written..acc_start` are silence.
    acc_start: u64,
    acc: VecDeque<f32>,
    latency: u64,
    resync: u64,
    max_gap_fill: u64,
    flushed: VecDeque<Block>,
}

impl Mixer {
    pub fn new(latency_ms: u64) -> Self {
        Self {
            sources: Vec::new(),
            written: 0,
            acc_start: 0,
            acc: VecDeque::new(),
            latency: latency_ms * SAMPLES_PER_MS,
            resync: 500 * SAMPLES_PER_MS,
            max_gap_fill: 5_000 * SAMPLES_PER_MS,
            flushed: VecDeque::new(),
        }
    }

    pub fn add_source(&mut self, kind: SourceKind, gain: f32) {
        if self.sources.iter().any(|s| s.kind == kind) {
            return;
        }
        self.sources.push(Source {
            kind,
            gain,
            enabled: true,
            next_pos: None,
            last_push_ms: None,
            received: 0,
            resyncs: 0,
            meter: LevelMeter::new(-60.0),
        });
    }

    pub fn set_enabled(&mut self, kind: SourceKind, enabled: bool) {
        if let Some(s) = self.sources.iter_mut().find(|s| s.kind == kind) {
            s.enabled = enabled;
            s.next_pos = None;
        }
    }

    /// Source restarted (device change, stream restart) – re-anchor on next push.
    pub fn reset_source(&mut self, kind: SourceKind) {
        if let Some(s) = self.sources.iter_mut().find(|s| s.kind == kind) {
            s.next_pos = None;
        }
    }

    pub fn written(&self) -> u64 {
        self.written
    }

    pub fn stats(&self) -> Vec<SourceStats> {
        self.sources
            .iter()
            .map(|s| SourceStats {
                kind: s.kind,
                enabled: s.enabled,
                received_samples: s.received,
                last_push_ms: s.last_push_ms,
                level_db: s.meter.level_db,
                resyncs: s.resyncs,
            })
            .collect()
    }

    pub fn push(&mut self, kind: SourceKind, samples: &[f32], now_ms: u64) {
        let Some(si) = self.sources.iter().position(|s| s.kind == kind) else { return };
        let len = samples.len() as u64;
        {
            let s = &mut self.sources[si];
            s.received += len;
            s.last_push_ms = Some(now_ms);
            s.meter.process(samples, now_ms);
            if !s.enabled || len == 0 {
                return;
            }
        }
        let expected_end = now_ms * SAMPLES_PER_MS;
        let pos = {
            let s = &mut self.sources[si];
            let anchored = expected_end.saturating_sub(len);
            let pos = match s.next_pos {
                None => anchored,
                Some(p) => {
                    let end = p + len;
                    if end + self.resync < expected_end || end > expected_end + self.resync {
                        s.resyncs += 1;
                        anchored
                    } else {
                        p
                    }
                }
            };
            s.next_pos = Some(pos + len);
            pos
        };
        let gain = self.sources[si].gain;
        self.place(pos, samples, gain);
    }

    fn place(&mut self, pos: u64, samples: &[f32], gain: f32) {
        let len = samples.len() as u64;
        if pos + len <= self.written {
            return; // arrived too late, already emitted
        }
        let acc_end = self.acc_start + self.acc.len() as u64;
        if self.acc.is_empty() {
            self.acc_start = self.written.max(pos);
        } else if pos > acc_end + self.max_gap_fill {
            // Long gap (e.g. machine slept): emit what we have now and leave the gap
            // as implicit silence instead of allocating it.
            let pending: Vec<f32> = self.acc.drain(..).collect();
            let silence_before = self.acc_start - self.written;
            if silence_before > 0 {
                self.flushed.push_back(Block::Silence(silence_before));
            }
            self.written = acc_end;
            self.flushed.push_back(Block::Samples(clamp(pending)));
            self.acc_start = pos;
        }
        // grow accumulation buffer to cover [.., pos+len)
        let start = pos.max(self.written);
        if start < self.acc_start {
            // placement before current acc start (within implicit silence): prepend zeros
            let extra = (self.acc_start - start) as usize;
            for _ in 0..extra {
                self.acc.push_front(0.0);
            }
            self.acc_start = start;
        }
        let needed_end = pos + len;
        let cur_end = self.acc_start + self.acc.len() as u64;
        if needed_end > cur_end {
            self.acc.resize(self.acc.len() + (needed_end - cur_end) as usize, 0.0);
        }
        let skip = (start - pos) as usize;
        for (i, v) in samples.iter().enumerate().skip(skip) {
            let idx = (pos + i as u64 - self.acc_start) as usize;
            self.acc[idx] += v * gain;
        }
    }

    /// Emit everything up to `now - latency` (at most `max` samples per call).
    pub fn pull(&mut self, now_ms: u64, max: u64) -> Vec<Block> {
        let target = (now_ms * SAMPLES_PER_MS).saturating_sub(self.latency);
        self.pull_until(target, max)
    }

    /// Emit everything that was placed (used on stop).
    pub fn drain_all(&mut self) -> Vec<Block> {
        let end = (self.acc_start + self.acc.len() as u64).max(self.written);
        self.pull_until(end, u64::MAX)
    }

    fn pull_until(&mut self, target: u64, max: u64) -> Vec<Block> {
        let mut out: Vec<Block> = self.flushed.drain(..).collect();
        let target = target.min(self.written.saturating_add(max));
        while self.written < target {
            if self.written < self.acc_start {
                let n = self.acc_start.min(target) - self.written;
                out.push(Block::Silence(n));
                self.written += n;
                continue;
            }
            let avail = self.acc.len() as u64;
            if avail == 0 {
                let n = target - self.written;
                out.push(Block::Silence(n));
                self.written += n;
                self.acc_start = self.written;
                break;
            }
            let n = (target - self.written).min(avail) as usize;
            let block: Vec<f32> = self.acc.drain(..n).collect();
            self.written += n as u64;
            self.acc_start = self.written;
            out.push(Block::Samples(clamp(block)));
        }
        out
    }
}

fn clamp(mut v: Vec<f32>) -> Vec<f32> {
    for s in &mut v {
        *s = s.clamp(-1.0, 1.0);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn total(blocks: &[Block]) -> u64 {
        blocks.iter().map(|b| b.len()).sum()
    }

    fn flatten(blocks: Vec<Block>) -> Vec<f32> {
        let mut v = Vec::new();
        for b in blocks {
            match b {
                Block::Samples(s) => v.extend(s),
                Block::Silence(n) => v.extend(std::iter::repeat_n(0.0, n as usize)),
            }
        }
        v
    }

    #[test]
    fn output_tracks_wall_clock_with_jittery_input() {
        let mut m = Mixer::new(500);
        m.add_source(SourceKind::System, 1.0);
        let mut out = Vec::new();
        // 10 ms blocks delivered in irregular bursts
        let mut t = 300u64; // capture starts 300 ms after lecture start
        for i in 0..1000u64 {
            let burst = if i % 7 == 0 { 3 } else { 1 };
            for _ in 0..burst {
                m.push(SourceKind::System, &vec![0.1; 160], t);
            }
            t += 10 * burst;
            if i % 10 == 0 {
                out.extend(m.pull(t, u64::MAX));
            }
        }
        let emitted = total(&out);
        assert_eq!(emitted, m.written());
        // emitted ≈ now - latency
        assert!((emitted as i64 - ((t - 500) * 16) as i64).abs() < 16 * 120, "{emitted}");
        // no zero holes inside the continuous signal
        let flat = flatten(out);
        let first = flat.iter().position(|&v| v > 0.0).unwrap();
        // the first audio is anchored near 300 ms (capture start), not at 0
        assert!((first as i64 - 290 * 16).abs() < 16 * 20, "first audio at {first}");
        assert!(flat[first..].iter().all(|&v| v > 0.0), "gap inside continuous audio");
    }

    #[test]
    fn mixes_two_sources_additively() {
        let mut m = Mixer::new(200);
        m.add_source(SourceKind::System, 1.0);
        m.add_source(SourceKind::Microphone, 0.5);
        for i in 0..100u64 {
            m.push(SourceKind::System, &vec![0.2; 160], 10 + i * 10);
            m.push(SourceKind::Microphone, &vec![0.2; 160], 10 + i * 10);
        }
        let flat = flatten(m.drain_all());
        let mid = flat[flat.len() / 2];
        assert!((mid - 0.3).abs() < 1e-6, "{mid}");
    }

    #[test]
    fn disabled_microphone_is_not_mixed() {
        let mut m = Mixer::new(200);
        m.add_source(SourceKind::System, 1.0);
        m.add_source(SourceKind::Microphone, 1.0);
        m.set_enabled(SourceKind::Microphone, false);
        for i in 0..50u64 {
            m.push(SourceKind::System, &vec![0.1; 160], 10 + i * 10);
            m.push(SourceKind::Microphone, &vec![0.5; 160], 10 + i * 10);
        }
        let flat = flatten(m.drain_all());
        assert!(flat.iter().all(|&v| v <= 0.1 + 1e-6));
        assert!(m.stats()[1].received_samples > 0, "still metered while muted");
    }

    #[test]
    fn long_gap_is_silence_without_allocation() {
        let mut m = Mixer::new(500);
        m.add_source(SourceKind::System, 1.0);
        for i in 0..100u64 {
            m.push(SourceKind::System, &vec![0.1; 160], 10 + i * 10);
        }
        // machine sleeps for one hour, then audio continues
        let wake = 1_000 + 3_600_000;
        m.push(SourceKind::System, &vec![0.1; 160], wake);
        assert!(m.acc.len() < 16_000 * 10, "gap must not be allocated");
        let mut blocks = Vec::new();
        let mut t = wake;
        while m.written() < (wake - 500) * 16 {
            blocks.extend(m.pull(t, 16_000 * 60));
            t += 1;
        }
        let silence: u64 = blocks.iter().filter_map(|b| if let Block::Silence(n) = b { Some(*n) } else { None }).sum();
        assert!(silence > 3_590_000 * 16);
        // alignment preserved: the post-sleep audio lands at its wall-clock position
        let tail = m.drain_all();
        assert!(tail.iter().any(|b| matches!(b, Block::Samples(_))));
        assert_eq!(m.written(), wake * 16);
    }
}
