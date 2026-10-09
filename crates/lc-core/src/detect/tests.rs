use super::*;
use crate::config::{DetectorConfig, RevealMode};
use crate::frame::NormRect;
use crate::synth::{shifted, slide, to_bgra, webcam, with_cursor, with_noise};

const W: u32 = 1280;
const H: u32 = 720;

#[derive(Debug, PartialEq, Clone)]
enum Ev {
    New(SlideId, u64),
    NewBuild(SlideId, SlideId),
    Update(SlideId),
    Revisit(SlideId, u64),
    Already(SlideId),
}

fn simplify(e: DetectorEvent) -> Ev {
    match e {
        DetectorEvent::NewSlide { slide_id, at_ms, build_of: Some(b), .. } => {
            let _ = at_ms;
            Ev::NewBuild(slide_id, b)
        }
        DetectorEvent::NewSlide { slide_id, at_ms, .. } => Ev::New(slide_id, at_ms),
        DetectorEvent::UpdateSlide { slide_id, .. } => Ev::Update(slide_id),
        DetectorEvent::Revisit { slide_id, at_ms } => Ev::Revisit(slide_id, at_ms),
        DetectorEvent::AlreadyCaptured { slide_id } => Ev::Already(slide_id),
    }
}

/// Feeds `frames` at 2 FPS starting at `t0`; returns events and the next time.
fn feed(d: &mut SlideDetector, frames: &[Frame], t0: u64, out: &mut Vec<Ev>) -> u64 {
    let mut t = t0;
    for f in frames {
        if let Some(e) = d.push_frame(f, t) {
            out.push(simplify(e));
        }
        t += 500;
    }
    t
}

fn repeat(f: &Frame, n: usize) -> Vec<Frame> {
    vec![f.clone(); n]
}

#[test]
fn detects_real_slide_change() {
    let mut d = SlideDetector::new(DetectorConfig::default(), None);
    let a = slide(W, H, 1, 4);
    let b = slide(W, H, 2, 5);
    let mut ev = Vec::new();
    let t = feed(&mut d, &repeat(&a, 6), 0, &mut ev);
    feed(&mut d, &repeat(&b, 6), t, &mut ev);
    // slide 2 must be timestamped when it first appeared (t = 3000 ms), not when committed
    assert_eq!(ev, vec![Ev::New(1, 0), Ev::New(2, 3000)]);
}

#[test]
fn works_with_bgra_frames() {
    let mut d = SlideDetector::new(DetectorConfig::default(), None);
    let a = to_bgra(&slide(W, H, 1, 4));
    let b = to_bgra(&slide(W, H, 2, 5));
    let mut ev = Vec::new();
    let t = feed(&mut d, &repeat(&a, 4), 0, &mut ev);
    feed(&mut d, &repeat(&b, 4), t, &mut ev);
    assert_eq!(ev.len(), 2);
}

#[test]
fn ignores_cursor_movement() {
    let mut d = SlideDetector::new(DetectorConfig::default(), None);
    let a = slide(W, H, 3, 5);
    let mut frames = repeat(&a, 4);
    // cursor wanders over the slide, including over text
    for i in 0..40u32 {
        frames.push(with_cursor(&a, 40 + i * 29 % (W - 80), 60 + (i * 53) % (H - 120)));
    }
    let mut ev = Vec::new();
    feed(&mut d, &frames, 0, &mut ev);
    assert_eq!(ev, vec![Ev::New(1, 0)]);
}

#[test]
fn ignores_compression_noise_and_small_shifts() {
    let mut d = SlideDetector::new(DetectorConfig::default(), None);
    let a = slide(W, H, 4, 5);
    let mut frames = repeat(&a, 4);
    for i in 0..20u64 {
        let f = with_noise(&a, 12, i);
        let f = if i % 3 == 0 { shifted(&f, 1, 1) } else { f };
        frames.push(f);
    }
    let mut ev = Vec::new();
    feed(&mut d, &frames, 0, &mut ev);
    assert_eq!(ev, vec![Ev::New(1, 0)]);
}

#[test]
fn ignores_transient_change_shorter_than_stability_window() {
    let mut d = SlideDetector::new(DetectorConfig::default(), None);
    let a = slide(W, H, 5, 4);
    let b = slide(W, H, 6, 4);
    let mut frames = repeat(&a, 4);
    frames.push(b.clone()); // flashes for one sample only
    frames.extend(repeat(&a, 6));
    let mut ev = Vec::new();
    feed(&mut d, &frames, 0, &mut ev);
    assert_eq!(ev, vec![Ev::New(1, 0)]);
}

#[test]
fn incremental_reveal_merge_mode_updates_same_slide() {
    let mut d = SlideDetector::new(DetectorConfig::default(), None);
    let mut ev = Vec::new();
    let mut t = 0;
    for lines in [2, 3, 4] {
        t = feed(&mut d, &repeat(&slide(W, H, 7, lines), 5), t, &mut ev);
    }
    assert_eq!(ev, vec![Ev::New(1, 0), Ev::Update(1), Ev::Update(1)]);
}

#[test]
fn incremental_reveal_separate_mode_creates_linked_slides() {
    let cfg = DetectorConfig { reveal_mode: RevealMode::Separate, ..Default::default() };
    let mut d = SlideDetector::new(cfg, None);
    let mut ev = Vec::new();
    let mut t = 0;
    for lines in [2, 3, 4] {
        t = feed(&mut d, &repeat(&slide(W, H, 7, lines), 5), t, &mut ev);
    }
    assert_eq!(ev, vec![Ev::New(1, 0), Ev::NewBuild(2, 1), Ev::NewBuild(3, 2)]);
}

#[test]
fn deduplicates_identical_slides_and_records_revisit() {
    let mut d = SlideDetector::new(DetectorConfig::default(), None);
    let a = slide(W, H, 8, 4);
    let b = slide(W, H, 9, 6);
    let mut ev = Vec::new();
    let mut t = feed(&mut d, &repeat(&a, 5), 0, &mut ev);
    t = feed(&mut d, &repeat(&b, 5), t, &mut ev);
    // back to A with a bit of noise – must reuse slide 1, not save a duplicate
    let a_noisy: Vec<Frame> = (0..5).map(|i| with_noise(&a, 8, i)).collect();
    let back = t;
    t = feed(&mut d, &a_noisy, t, &mut ev);
    feed(&mut d, &repeat(&b, 5), t, &mut ev);
    assert_eq!(ev, vec![Ev::New(1, 0), Ev::New(2, 2500), Ev::Revisit(1, back), Ev::Revisit(2, t)]);
}

#[test]
fn never_stable_content_is_saved_after_timeout() {
    let cfg = DetectorConfig { max_unstable_ms: 5_000, motion_filter: false, ..Default::default() };
    let mut d = SlideDetector::new(cfg, None);
    let mut ev = Vec::new();
    let mut unstable_flag = false;
    for i in 0..20u64 {
        // a different slide every sample (e.g. a video playing)
        if let Some(e) = d.push_frame(&slide(W, H, 100 + i, 3 + (i % 4) as u32), i * 500) {
            if let DetectorEvent::NewSlide { unstable, .. } = &e {
                unstable_flag |= *unstable;
            }
            ev.push(simplify(e));
        }
    }
    assert!(!ev.is_empty(), "unstable content should eventually be saved");
    assert!(unstable_flag);
    assert!(ev.len() <= 3, "must not save every frame: {ev:?}");
}

#[test]
fn idle_ticks_count_towards_stability() {
    let mut d = SlideDetector::new(DetectorConfig::default(), None);
    let a = slide(W, H, 10, 4);
    assert!(d.push_frame(&a, 0).is_none());
    assert!(d.push_idle(500).is_none());
    let e = d.push_idle(1000).map(simplify);
    assert_eq!(e, Some(Ev::New(1, 0)));
}

#[test]
fn blank_frames_are_ignored() {
    let mut d = SlideDetector::new(DetectorConfig::default(), None);
    let a = slide(W, H, 11, 4);
    let black = Frame::solid(W, H, [0, 0, 0]);
    let mut ev = Vec::new();
    let mut t = feed(&mut d, &repeat(&a, 4), 0, &mut ev);
    t = feed(&mut d, &repeat(&black, 6), t, &mut ev);
    feed(&mut d, &repeat(&a, 4), t, &mut ev);
    assert_eq!(ev, vec![Ev::New(1, 0)]);
}

#[test]
fn manual_capture() {
    let mut d = SlideDetector::new(DetectorConfig::default(), None);
    let a = slide(W, H, 12, 4);
    let mut ev = Vec::new();
    feed(&mut d, &repeat(&a, 4), 0, &mut ev);
    assert_eq!(d.manual_capture(3000).map(simplify), Some(Ev::Already(1)));
    // something new on screen, not yet stable → manual capture saves it right away
    d.push_frame(&slide(W, H, 13, 6), 3500);
    assert_eq!(d.manual_capture(3600).map(simplify), Some(Ev::New(2, 3600)));
}

#[test]
fn masked_region_is_ignored() {
    // Teams overlay / clock in the bottom-right corner
    let cfg = DetectorConfig {
        masks: vec![NormRect { x: 0.7, y: 0.8, w: 0.3, h: 0.2 }],
        ..Default::default()
    };
    let mut d = SlideDetector::new(cfg, None);
    let a = slide(W, H, 14, 4);
    let mut frames = repeat(&a, 4);
    for i in 0..6 {
        let mut f = a.clone();
        f.fill_rect(W * 3 / 4, H * 17 / 20, 200, 60, [(i * 40) as u8, 90, 200]);
        frames.push(f);
    }
    let mut ev = Vec::new();
    feed(&mut d, &frames, 0, &mut ev);
    assert_eq!(ev, vec![Ev::New(1, 0)]);
}

#[test]
fn crop_limits_analysis_to_presentation_area() {
    // participant tiles change on the right side, presentation on the left stays
    let crop = NormRect { x: 0.0, y: 0.0, w: 0.7, h: 1.0 };
    let mut d = SlideDetector::new(DetectorConfig::default(), Some(crop));
    let a = slide(W, H, 15, 4);
    let mut ev = Vec::new();
    let mut frames = Vec::new();
    for i in 0..10u32 {
        let mut f = a.clone();
        f.fill_rect(W * 3 / 4, 0, W / 4, H, [(i * 20) as u8, 120, 60]);
        frames.push(f);
    }
    feed(&mut d, &frames, 0, &mut ev);
    assert_eq!(ev.len(), 1);
}

#[test]
fn saved_frame_has_native_cropped_resolution() {
    let crop = NormRect { x: 0.25, y: 0.0, w: 0.5, h: 1.0 };
    let mut d = SlideDetector::new(DetectorConfig::default(), Some(crop));
    let a = slide(1920, 1080, 16, 4);
    let mut saved = None;
    for i in 0..5 {
        if let Some(DetectorEvent::NewSlide { frame, .. }) = d.push_frame(&a, i * 500) {
            saved = Some(frame);
        }
    }
    let f = saved.expect("slide saved");
    assert_eq!((f.width, f.height), (960, 1080));
}

/// Lecturer on camera: sways, gestures and sometimes holds still for a few seconds.
fn camera_frames(seconds: u64, seed: u64) -> Vec<Frame> {
    let mut x = 0i32;
    let mut y = 0i32;
    let mut state = seed * 7 + 3;
    let mut out = Vec::new();
    for i in 0..seconds * 2 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let r = (state >> 33) as i32;
        // every ~8 s the lecturer holds still for ~2.5 s
        let still = i % 16 >= 11;
        if !still {
            x = (x + r % 61 - 30).clamp(-200, 200);
            y = (y + (r >> 8) % 21 - 10).clamp(-40, 40);
        }
        let arm = if still { 0 } else { ((r >> 4) % 120) as u32 };
        out.push(with_noise(&webcam(W / 2, H / 2, x / 2, y / 2, arm / 2), 6, i));
    }
    out
}

#[test]
fn moving_lecturer_on_camera_is_not_saved_repeatedly() {
    let mut d = SlideDetector::new(DetectorConfig::default(), None);
    let mut ev = Vec::new();
    feed(&mut d, &camera_frames(120, 1), 0, &mut ev);
    // at most the very first view of the camera may be kept, never a stream of shots
    assert!(ev.len() <= 1, "camera produced {} slides: {ev:?}", ev.len());

    // without the filter the same footage produces many "slides" (the reported bug)
    let mut old = SlideDetector::new(DetectorConfig { motion_filter: false, ..Default::default() }, None);
    let mut ev_old = Vec::new();
    feed(&mut old, &camera_frames(120, 1), 0, &mut ev_old);
    assert!(ev_old.len() >= 5, "reference behaviour changed: {ev_old:?}");
}

#[test]
fn slide_after_camera_is_detected_with_correct_time() {
    let mut d = SlideDetector::new(DetectorConfig::default(), None);
    let mut ev = Vec::new();
    let t = feed(&mut d, &camera_frames(40, 2), 0, &mut ev);
    let before = ev.len();
    let a = slide(W, H, 21, 5);
    feed(&mut d, &repeat(&a, 20), t, &mut ev);
    let after: Vec<_> = ev[before..].to_vec();
    assert_eq!(after.len(), 1, "{ev:?}");
    match after[0] {
        Ev::New(_, at) => assert_eq!(at, t, "slide must be timestamped when it appeared"),
        ref other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn camera_overlay_on_slide_does_not_hide_slide_changes() {
    // Teams shows the presenter's video in a corner of the shared slide
    let overlay = |base: &Frame, i: u64| {
        let cam = webcam(240, 135, ((i * 37) % 40) as i32 - 20, ((i * 13) % 14) as i32 - 7, ((i * 29) % 40) as u32);
        let mut f = base.clone();
        for y in 0..135u32 {
            for x in 0..240u32 {
                f.fill_rect(W - 250 + x, H - 145 + y, 1, 1, cam.rgb_at(x, y));
            }
        }
        f
    };
    let mut d = SlideDetector::new(DetectorConfig::default(), None);
    let mut ev = Vec::new();
    let mut t = 0;
    let mut i = 0;
    for (seed, lines) in [(31, 4), (32, 6), (33, 3)] {
        let s = slide(W, H, seed, lines);
        let frames: Vec<Frame> = (0..30).map(|_| { i += 1; overlay(&s, i) }).collect();
        t = feed(&mut d, &frames, t, &mut ev);
    }
    let new: Vec<_> = ev.iter().filter(|e| matches!(e, Ev::New(..))).collect();
    assert_eq!(new.len(), 3, "{ev:?}");
}

#[test]
fn motion_filter_keeps_normal_slide_timing() {
    let mut d = SlideDetector::new(DetectorConfig::default(), None);
    let mut ev = Vec::new();
    let mut t = 0;
    let mut starts = Vec::new();
    for seed in 40..46 {
        starts.push(t);
        t = feed(&mut d, &repeat(&with_noise(&slide(W, H, seed, 3 + (seed % 3) as u32), 6, seed), 8), t, &mut ev);
    }
    let times: Vec<u64> = ev.iter().filter_map(|e| if let Ev::New(_, at) = e { Some(*at) } else { None }).collect();
    assert_eq!(times, starts);
}
