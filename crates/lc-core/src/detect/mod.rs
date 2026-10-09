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

pub mod analysis;

use crate::config::{DetectorConfig, RevealMode};
use crate::frame::{Frame, NormRect};
use analysis::{analyze, compare, hamming, Analysis, Diff};

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
    pub stats: DetectorStats,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct DetectorStats {
    pub frames: u64,
    pub idle_ticks: u64,
    pub blank_frames: u64,
    pub ignored_small_changes: u64,
    pub candidates: u64,
    pub last_change_fraction: f32,
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
        if self.cfg.ignore_blank && a.is_blank() {
            self.stats.blank_frames += 1;
            self.candidate = None;
            return None;
        }
        if let Some(r) = &self.reference {
            let d = compare(r, &a, &self.cfg);
            self.stats.last_change_fraction = d.changed_fraction;
            if !d.is_significant(&self.cfg) {
                if d.changed_blocks > 0 {
                    self.stats.ignored_small_changes += 1;
                }
                self.candidate = None;
                return None;
            }
        }
        match &mut self.candidate {
            Some(c) if !compare(&c.analysis, &a, &self.cfg).is_significant(&self.cfg) => {
                c.stable += 1;
                c.analysis = a;
                c.frame = frame.clone();
            }
            other => {
                let first_ms = other.as_ref().map(|c| c.first_ms).unwrap_or(t_ms);
                if other.is_none() {
                    self.stats.candidates += 1;
                }
                *other = Some(Candidate { first_ms, analysis: a, frame: frame.clone(), stable: 1 });
            }
        }
        self.try_commit(t_ms)
    }

    /// No new frame arrived for one sampling period (screen unchanged).
    pub fn push_idle(&mut self, t_ms: u64) -> Option<DetectorEvent> {
        self.stats.idle_ticks += 1;
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
            if compare(g, a, &self.cfg).is_significant(&self.cfg) {
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
            if self.current == Some(id) {
                // still the same slide (e.g. a transient overlay disappeared)
                return manual.then_some(DetectorEvent::AlreadyCaptured { slide_id: id });
            }
            self.current = Some(id);
            return Some(DetectorEvent::Revisit { slide_id: id, at_ms });
        }

        let cropped = frame.crop(self.crop.as_ref());
        let build_of = match (&self.reference, self.current) {
            (Some(r), Some(cur)) if !manual && is_build(r, &a, &compare(r, &a, &self.cfg), &self.cfg) => {
                Some(cur)
            }
            _ => None,
        };
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
