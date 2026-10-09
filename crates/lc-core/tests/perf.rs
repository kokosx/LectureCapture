//! Performance guard: per-frame analysis cost at MacBook Air native resolution and
//! PNG encoding cost. Run with `cargo test -p lc-core --release --test perf -- --nocapture`.

use lc_core::config::{DetectorConfig, PngCompression};
use lc_core::detect::SlideDetector;
use lc_core::synth::{slide, to_bgra, with_noise};
use std::time::Instant;

#[test]
fn detection_cost_per_frame() {
    // 2560x1664 = MacBook Air 13" M4 native panel
    let a = to_bgra(&slide(2560, 1664, 1, 5));
    let b = to_bgra(&slide(2560, 1664, 2, 6));
    let frames: Vec<_> = (0..20).map(|i| if i < 10 { with_noise(&a, 6, i) } else { with_noise(&b, 6, i) }).collect();
    let mut d = SlideDetector::new(DetectorConfig::default(), None);
    let t = Instant::now();
    for (i, f) in frames.iter().enumerate() {
        d.push_frame(f, i as u64 * 500);
    }
    let per = t.elapsed().as_secs_f64() * 1000.0 / frames.len() as f64;
    eprintln!("detection: {per:.1} ms/frame at 2560x1664 → {:.2}% of one core at 2 FPS", per * 2.0 / 10.0);
    // generous bound so debug builds pass; release is ~10x faster
    assert!(per < if cfg!(debug_assertions) { 400.0 } else { 40.0 }, "{per} ms/frame");
}

#[test]
fn png_encoding_cost() {
    let a = slide(2560, 1664, 3, 6);
    let t = Instant::now();
    let png = lc_core::export::encode_png(&a, PngCompression::Balanced).unwrap();
    eprintln!("png: {:.0} ms, {} KB for 2560x1664", t.elapsed().as_secs_f64() * 1000.0, png.len() / 1024);
}
