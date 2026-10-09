//! Slide change detection.
//!
//! Pipeline for every sampled frame (1–2 FPS by default):
//!
//! 1. analyze only the presentation region (crop) → small grayscale image + dHash,
//! 2. compare with the *reference* (the slide currently on screen) using a
//!    shift-tolerant per-pixel diff aggregated into blocks,
//! 3. drop insignificant changes (cursor-sized blobs, compression noise, masked areas),
//! 4. a significant change becomes a *candidate*; it is committed only when it stays
//!    stable for `stable_frames` samples (debounce) or after `max_unstable_ms`,
//! 5. the committed image is matched against all previously saved slides (dHash +
//!    pixel confirmation) → revisit of an existing slide instead of a duplicate,
//! 6. purely additive changes on empty background are classified as *builds*
//!    (bullet points revealed one by one) and handled per [`RevealMode`].
//!
//! Motion filter: consecutive frames are also compared with each other. Blocks that
//! keep changing (the lecturer's webcam, an embedded video) become *dynamic* and are
//! masked until they have been still for `motion_hold_ms`. A frame that is mostly
//! dynamic is live video (camera only) and never produces a slide; a change confined
//! to recently moving areas (the lecturer froze in a new pose) is ignored as well.

pub mod analysis;

use crate::config::{DetectorConfig, RevealMode};
use crate::frame::{Frame, NormRect};
use analysis::{analyze, compare, hamming, Analysis, Diff};
use std::borrow::Cow;

/// Time constant of the per-block motion average.
const MOTION_TAU_MS: f64 = 2_200.0;
/// Motion average above which a block becomes dynamic (≈ 2 s of continuous change).
const MOTION_ON: f32 = 0.55;
/// Motion average of a block that changed in consecutive samples – already treated
/// as live when judging a change, before it is masked (e.g. right after start).
const MOTION_WARM: f32 = 0.3;
/// How long a block counts as "recently live" after it stopped being dynamic.
const LIVE_MEMORY_MS: u64 = 60_000;
/// Blocks this close to a live block count as live too.
const LIVE_RADIUS_BLOCKS: usize = 2;
/// A change that mostly lies in recently live blocks is ignored…
const LIVE_CHANGE_SHARE: f32 = 0.7;
/// …unless it covers this much of the image (a slide replacing the camera view).
const LIVE_OVERRIDE_FRACTION: f32 = 0.5;

pub type SlideId = u32;

#[derive(Debug)]
pub enum DetectorEvent {
    /// A new slide image should be saved.
    NewSlide {
        slide_id: SlideId,
        /// Lecture time when the change was first observed (start of display).
        at_ms: u64,
        /// Cropped frame in native resolution.
        frame: Frame,
        build_of: Option<SlideId>,
        /// Content never became stable; saved after `max_unstable_ms`.
        unstable: bool,
        manual: bool,
        dhash: u64,
    },
    /// Build step in merge mode: replace the image of `slide_id` with a fuller one.
    UpdateSlide { slide_id: SlideId, at_ms: u64, frame: Frame, dhash: u64 },
    /// A previously saved slide is shown again → new timeline occurrence.
    Revisit { slide_id: SlideId, at_ms: u64 },
    /// Manual capture requested but the current slide is already saved unchanged.
    AlreadyCaptured { slide_id: SlideId },
}

struct Candidate {
    first_ms: u64,
    analysis: Analysis,
    frame: Frame,
    stable: u32,
}

pub struct SlideDetector {
    cfg: DetectorConfig,
    crop: Option<NormRect>,
    reference: Option<Analysis>,
    current: Option<SlideId>,
    candidate: Option<Candidate>,
    gallery: Vec<(SlideId, Analysis)>,
    last_commit_ms: Option<u64>,
    last: Option<(Frame, Analysis)>,
    next_id: SlideId,
    /// When the reference was set (lower bound for change timestamps).
    ref_ms: u64,
    motion: Motion,
    pub stats: DetectorStats,
}

/// Per-block motion state (block grid of the analysis image).
#[derive(Default)]
struct Motion {
    prev: Option<Analysis>,
    t_ms: Option<u64>,
    bx: usize,
    by: usize,
    ema: Vec<f32>,
    last_motion: Vec<Option<u64>>,
    dynamic: Vec<bool>,
    last_dynamic: Vec<Option<u64>>,
}

impl Motion {
    fn reset(&mut self) {
        *self = Motion::default();
    }

    fn ensure_grid(&mut self, a: &Analysis, bs: usize) {
        let (bx, by) = (a.w.div_ceil(bs), a.h.div_ceil(bs));
        if (bx, by) != (self.bx, self.by) || self.ema.len() != bx * by {
            *self = Motion {
                bx,
                by,
                ema: vec![0.0; bx * by],
                last_motion: vec![None; bx * by],
                dynamic: vec![false; bx * by],
                last_dynamic: vec![None; bx * by],
                ..Default::default()
            };
        }
    }

    /// One sample: `changed` = blocks that differ from the previous frame.
    fn step(&mut self, changed: Option<&[bool]>, t_ms: u64, hold_ms: u64) {
        let dt = self.t_ms.map(|p| t_ms.saturating_sub(p)).unwrap_or(0) as f64;
        self.t_ms = Some(t_ms);
        let alpha = (1.0 - (-dt / MOTION_TAU_MS).exp()) as f32;
        for i in 0..self.ema.len() {
            let ch = changed.is_some_and(|c| c.get(i).copied().unwrap_or(false));
            if ch {
                self.last_motion[i] = Some(t_ms);
            }
            self.ema[i] = self.ema[i] * (1.0 - alpha) + if ch { alpha } else { 0.0 };
            let held = self.dynamic[i] && self.last_motion[i].is_some_and(|m| t_ms.saturating_sub(m) < hold_ms);
            self.dynamic[i] = self.ema[i] >= MOTION_ON || held;
            if self.dynamic[i] {
                self.last_dynamic[i] = Some(t_ms);
            }
        }
    }

    fn recently_live(&self, i: usize, t_ms: u64) -> bool {
        self.ema.get(i).is_some_and(|&e| e >= MOTION_WARM)
            || self.last_dynamic.get(i).copied().flatten().is_some_and(|d| t_ms.saturating_sub(d) <= LIVE_MEMORY_MS)
    }

    /// Recently live, or next to such a block (a person moving into a new area).
    fn near_live(&self, i: usize, t_ms: u64) -> bool {
        let r = LIVE_RADIUS_BLOCKS as isize;
        let (x, y) = ((i % self.bx) as isize, (i / self.bx) as isize);
        for dy in -r..=r {
            for dx in -r..=r {
                let (nx, ny) = (x + dx, y + dy);
                if nx >= 0 && ny >= 0 && (nx as usize) < self.bx && (ny as usize) < self.by
                    && self.recently_live(ny as usize * self.bx + nx as usize, t_ms)
                {
                    return true;
                }
            }
        }
        false
    }
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct DetectorStats {
    pub frames: u64,
    pub idle_ticks: u64,
    pub blank_frames: u64,
    pub ignored_small_changes: u64,
    pub candidates: u64,
    pub last_change_fraction: f32,
    /// Frames treated as live video (camera / video only).
    pub live_frames: u64,
    /// Changes ignored because they were confined to moving areas.
    pub live_changes_ignored: u64,
    /// Fraction of the image currently masked as moving.
    pub dynamic_fraction: f32,
}

impl SlideDetector {
    pub fn new(cfg: DetectorConfig, crop: Option<NormRect>) -> Self {
        Self {
            cfg,
            crop: crop.filter(|c| !c.is_full()),
            reference: None,
            current: None,
            candidate: None,
            gallery: Vec::new(),
            last_commit_ms: None,
            last: None,
            next_id: 1,
            ref_ms: 0,
            motion: Motion::default(),
            stats: DetectorStats::default(),
        }
    }

    pub fn config(&self) -> &DetectorConfig {
        &self.cfg
    }

    pub fn current_slide(&self) -> Option<SlideId> {
        self.current
    }

    pub fn set_crop(&mut self, crop: Option<NormRect>) {
        self.crop = crop.filter(|c| !c.is_full());
        self.reference = None;
        self.candidate = None;
        self.motion.reset();
    }

    /// Continue numbering after existing slides (e.g. resuming a lecture).
    pub fn seed_next_id(&mut self, next: SlideId) {
        self.next_id = self.next_id.max(next);
    }

    /// Feed a newly sampled frame.
    pub fn push_frame(&mut self, frame: &Frame, t_ms: u64) -> Option<DetectorEvent> {
        self.stats.frames += 1;
        let a = analyze(frame, self.crop.as_ref(), &self.cfg);
        self.last = Some((frame.clone(), a.clone()));
        if self.cfg.motion_filter {
            self.update_motion(&a, t_ms);
        }
        if self.cfg.ignore_blank && a.is_blank() {
            self.stats.blank_frames += 1;
            self.candidate = None;
            return None;
        }
        if self.cfg.motion_filter && self.stats.dynamic_fraction >= self.cfg.live_video_fraction {
            // camera / video only: nothing to save
            self.stats.live_frames += 1;
            self.candidate = None;
            return None;
        }
        let am = self.masked(&a);
        let mut change_ms = t_ms;
        match &self.reference {
            Some(r) => {
                let d = compare(r, &am, &self.cfg);
                self.stats.last_change_fraction = d.changed_fraction;
                if !d.is_significant(&self.cfg) {
                    if d.changed_blocks > 0 {
                        self.stats.ignored_small_changes += 1;
                    }
                    self.candidate = None;
                    return None;
                }
                if self.cfg.motion_filter {
                    let changed: Vec<usize> = (0..d.changed.len()).filter(|&i| d.changed[i]).collect();
                    if self.is_live_change(&changed, d.changed_fraction, t_ms) {
                        self.stats.live_changes_ignored += 1;
                        self.candidate = None;
                        return None;
                    }
                    change_ms = self.settled_at(&changed, t_ms);
                }
            }
            None if self.cfg.motion_filter => {
                let all: Vec<usize> = (0..self.motion.dynamic.len()).collect();
                if self.is_live_change(&all, 0.0, t_ms) {
                    self.stats.live_changes_ignored += 1;
                    self.candidate = None;
                    return None;
                }
            }
            None => {}
        }
        match &mut self.candidate {
            Some(c) if !compare(&c.analysis, &am, &self.cfg).is_significant(&self.cfg) => {
                c.stable += 1;
                c.analysis = a;
                c.frame = frame.clone();
            }
            other => {
                let first_ms = other.as_ref().map(|c| c.first_ms).unwrap_or(change_ms);
                if other.is_none() {
                    self.stats.candidates += 1;
                }
                *other = Some(Candidate { first_ms, analysis: a, frame: frame.clone(), stable: 1 });
            }
        }
        self.try_commit(t_ms)
    }

    fn update_motion(&mut self, a: &Analysis, t_ms: u64) {
        let bs = self.cfg.block_size.max(2) as usize;
        self.motion.ensure_grid(a, bs);
        let changed = match &self.motion.prev {
            Some(p) if p.w == a.w && p.h == a.h => Some(compare(p, a, &self.cfg).changed),
            _ => None,
        };
        self.motion.step(changed.as_deref(), t_ms, self.cfg.motion_hold_ms);
        self.motion.prev = Some(a.clone());
        self.update_dynamic_fraction(a);
    }

    fn update_dynamic_fraction(&mut self, a: &Analysis) {
        let bs = self.cfg.block_size.max(2) as usize;
        let (mut total, mut dynamic) = (0usize, 0usize);
        for (i, &dy) in self.motion.dynamic.iter().enumerate() {
            let (bx, by) = (i % self.motion.bx, i / self.motion.bx);
            // blocks fully covered by a user mask do not count
            let (x, y) = ((bx * bs + bs / 2).min(a.w - 1), (by * bs + bs / 2).min(a.h - 1));
            if a.mask[y * a.w + x] {
                continue;
            }
            total += 1;
            dynamic += dy as usize;
        }
        self.stats.dynamic_fraction = if total > 0 { dynamic as f32 / total as f32 } else { 0.0 };
    }

    /// `a` with currently dynamic blocks masked out.
    fn masked<'a>(&self, a: &'a Analysis) -> Cow<'a, Analysis> {
        if !self.cfg.motion_filter || !self.motion.dynamic.iter().any(|&d| d) {
            return Cow::Borrowed(a);
        }
        let bs = self.cfg.block_size.max(2) as usize;
        let mut m = a.clone();
        for y in 0..a.h {
            for x in 0..a.w {
                let bi = (y / bs) * self.motion.bx + x / bs;
                if self.motion.dynamic.get(bi).copied().unwrap_or(false) {
                    m.mask[y * a.w + x] = true;
                }
            }
        }
        Cow::Owned(m)
    }

    /// The change lies (almost) entirely in areas that were moving recently and does
    /// not replace most of the image → the camera, not a new slide.
    fn is_live_change(&self, changed_blocks: &[usize], changed_fraction: f32, t_ms: u64) -> bool {
        if changed_blocks.is_empty() || changed_fraction >= LIVE_OVERRIDE_FRACTION {
            return false;
        }
        let live = changed_blocks.iter().filter(|&&i| self.motion.near_live(i, t_ms)).count();
        live as f32 >= LIVE_CHANGE_SHARE * changed_blocks.len() as f32
    }

    /// When the changed area stopped moving – the moment the new content appeared,
    /// even if it was masked as moving until now.
    fn settled_at(&self, changed_blocks: &[usize], t_ms: u64) -> u64 {
        changed_blocks
            .iter()
            .filter_map(|&i| self.motion.last_motion.get(i).copied().flatten())
            .max()
            .map_or(t_ms, |m| m.clamp(self.ref_ms, t_ms))
    }

    /// No new frame arrived for one sampling period (screen unchanged).
    pub fn push_idle(&mut self, t_ms: u64) -> Option<DetectorEvent> {
        self.stats.idle_ticks += 1;
        if self.cfg.motion_filter && !self.motion.ema.is_empty() {
            self.motion.step(None, t_ms, self.cfg.motion_hold_ms);
            if let Some((_, a)) = &self.last {
                let a = a.clone();
                self.update_dynamic_fraction(&a);
            }
        }
        if let Some(c) = &mut self.candidate {
            c.stable += 1;
        }
        self.try_commit(t_ms)
    }

    fn try_commit(&mut self, t_ms: u64) -> Option<DetectorEvent> {
        let c = self.candidate.as_ref()?;
        let unstable = t_ms.saturating_sub(c.first_ms) >= self.cfg.max_unstable_ms;
        if c.stable < self.cfg.stable_frames.max(1) && !unstable {
            return None;
        }
        if let Some(lc) = self.last_commit_ms {
            if t_ms.saturating_sub(lc) < self.cfg.min_change_interval_ms {
                return None;
            }
        }
        let c = self.candidate.take()?;
        self.last_commit_ms = Some(t_ms);
        let unstable = unstable && c.stable < self.cfg.stable_frames.max(1);
        self.commit(c.analysis, c.frame, c.first_ms, unstable, false)
    }

    fn find_in_gallery(&self, a: &Analysis) -> Option<SlideId> {
        let mut best: Option<(u32, SlideId)> = None;
        for (id, g) in &self.gallery {
            let hd = hamming(g.dhash, a.dhash);
            if hd > self.cfg.dedupe_hash_distance {
                continue;
            }
            if compare(g, &self.masked(a), &self.cfg).is_significant(&self.cfg) {
                continue;
            }
            if best.is_none_or(|(b, _)| hd < b) {
                best = Some((hd, *id));
            }
        }
        best.map(|(_, id)| id)
    }

    fn commit(
        &mut self,
        a: Analysis,
        frame: Frame,
        at_ms: u64,
        unstable: bool,
        manual: bool,
    ) -> Option<DetectorEvent> {
        // Back to a slide we already have?
        if let Some(id) = self.find_in_gallery(&a) {
            self.reference = Some(a);
            self.ref_ms = at_ms;
            if self.current == Some(id) {
                // still the same slide (e.g. a transient overlay disappeared)
                return manual.then_some(DetectorEvent::AlreadyCaptured { slide_id: id });
            }
            self.current = Some(id);
            return Some(DetectorEvent::Revisit { slide_id: id, at_ms });
        }

        let cropped = frame.crop(self.crop.as_ref());
        let build_of = match (&self.reference, self.current) {
            (Some(r), Some(cur)) if !manual && {
                let am = self.masked(&a);
                is_build(r, &am, &compare(r, &am, &self.cfg), &self.cfg)
            } =>
            {
                Some(cur)
            }
            _ => None,
        };
        self.ref_ms = at_ms;
        if let (Some(cur), RevealMode::Merge) = (build_of, self.cfg.reveal_mode) {
            if let Some(entry) = self.gallery.iter_mut().find(|(id, _)| *id == cur) {
                entry.1 = a.clone();
            }
            let dhash = a.dhash;
            self.reference = Some(a);
            return Some(DetectorEvent::UpdateSlide { slide_id: cur, at_ms, frame: cropped, dhash });
        }

        let id = self.next_id;
        self.next_id += 1;
        let dhash = a.dhash;
        self.gallery.push((id, a.clone()));
        self.reference = Some(a);
        self.current = Some(id);
        Some(DetectorEvent::NewSlide { slide_id: id, at_ms, frame: cropped, build_of, unstable, manual, dhash })
    }

    /// Save the current screen immediately (button / hotkey).
    pub fn manual_capture(&mut self, t_ms: u64) -> Option<DetectorEvent> {
        let (frame, a) = self.last.clone()?;
        self.candidate = None;
        if let (Some(r), Some(cur)) = (&self.reference, self.current) {
            if !compare(r, &a, &self.cfg).is_significant(&self.cfg) {
                return Some(DetectorEvent::AlreadyCaptured { slide_id: cur });
            }
        }
        self.last_commit_ms = Some(t_ms);
        self.commit(a, frame, t_ms, false, true)
    }

    /// After pause/resume or source change: forget what is on screen but keep the
    /// gallery so returning slides are still deduplicated.
    pub fn reset_reference(&mut self) {
        self.reference = None;
        self.current = None;
        self.candidate = None;
        self.motion.prev = None;
    }

    /// Most recent raw frame (for previews).
    pub fn last_frame(&self) -> Option<&Frame> {
        self.last.as_ref().map(|(f, _)| f)
    }
}

/// Purely additive change on previously empty background = build step (e.g. the
/// next bullet point appears while everything already shown stays in place).
fn is_build(reference: &Analysis, new: &Analysis, d: &Diff, _cfg: &DetectorConfig) -> bool {
    if d.geometry_changed || d.changed_blocks == 0 || d.changed_fraction > 0.3 {
        return false;
    }
    let bg = reference.background_luma() as i16;
    let is_bg = |v: u8| (v as i16 - bg).abs() <= 12;
    let (mut changed, mut changed_on_bg, mut content, mut content_changed) = (0usize, 0usize, 0usize, 0usize);
    for (i, &ch) in d.changed_px.iter().enumerate() {
        if reference.mask[i] {
            continue;
        }
        let ref_bg = is_bg(reference.luma[i]);
        if !ref_bg {
            content += 1;
            if ch {
                content_changed += 1;
            }
        }
        if ch {
            changed += 1;
            if ref_bg && !is_bg(new.luma[i]) {
                changed_on_bg += 1;
            }
        }
    }
    content > 0
        && changed > 0
        && changed_on_bg as f32 / changed as f32 >= 0.8
        && (content_changed as f32 / content as f32) <= 0.08
}

#[cfg(test)]
mod tests;
