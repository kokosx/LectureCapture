//! Deterministic synthetic test material: slide-like frames, cursor, compression
//! noise, shifts and speech-like audio. Used by unit and integration tests and by
//! the CLI self-test – never by the real capture path.

use crate::frame::{Frame, PixelFormat};

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
}

/// Slide with a title bar and `lines` bullet lines made of word-like dark blocks.
/// Different `seed`s produce visibly different slides.
pub fn slide(width: u32, height: u32, seed: u64, lines: u32) -> Frame {
    let mut f = Frame::solid(width, height, [250, 250, 248]);
    let mut rng = Lcg(seed.wrapping_mul(7919) + 17);
    let u = (height / 36).max(4); // text height unit
    // title
    let title_words = 2 + rng.next() % 3;
    let mut x = width / 12;
    for _ in 0..title_words {
        let w = u * (3 + rng.next() % 5);
        f.fill_rect(x, height / 12, w, u * 2, [30, 40, 90]);
        x += w + u;
    }
    // bullets
    for l in 0..lines {
        let y = height / 4 + l * u * 3;
        f.fill_rect(width / 12, y + u / 4, u / 2 + 2, u / 2 + 2, [200, 60, 40]);
        let mut x = width / 12 + u * 2;
        let words = 3 + rng.next() % 6;
        for _ in 0..words {
            let w = u * (1 + rng.next() % 4);
            if x + w > width - width / 12 {
                break;
            }
            f.fill_rect(x, y, w, u, [20, 20, 20]);
            x += w + u / 2 + 2;
        }
    }
    // a "diagram" box depending on seed
    if seed % 2 == 0 {
        let bx = width / 2 + (rng.next() % (width / 8));
        f.fill_rect(bx, height * 2 / 3, width / 5, height / 6, [70, 130, 200]);
    }
    f
}

/// Draws a small arrow-like cursor (about 12×18 px at 1280×720 scale).
pub fn with_cursor(frame: &Frame, x: u32, y: u32) -> Frame {
    let mut f = frame.clone();
    let s = (frame.width / 110).max(4);
    for i in 0..(s * 3 / 2) {
        f.fill_rect(x, y + i, (i * 2 / 3).max(1), 1, [0, 0, 0]);
    }
    f.fill_rect(x + 1, y + s, s / 3 + 1, s / 2, [0, 0, 0]);
    f
}

/// Adds ±`amp` pseudo-random noise (video compression artefacts).
pub fn with_noise(frame: &Frame, amp: u8, seed: u64) -> Frame {
    let mut rng = Lcg(seed + 99);
    let mut data = (*frame.data).clone();
    for px in data.chunks_exact_mut(4) {
        let n = (rng.next() % (2 * amp as u32 + 1)) as i16 - amp as i16;
        for c in &mut px[..3] {
            *c = (*c as i16 + n).clamp(0, 255) as u8;
        }
    }
    Frame { data: std::sync::Arc::new(data), ..frame.clone() }
}

/// Shifts the content by (dx, dy) pixels, filling with the edge colour.
pub fn shifted(frame: &Frame, dx: i32, dy: i32) -> Frame {
    let (w, h) = (frame.width as i32, frame.height as i32);
    let mut out = vec![0u8; frame.data.len()];
    for y in 0..h {
        for x in 0..w {
            let sx = (x - dx).clamp(0, w - 1);
            let sy = (y - dy).clamp(0, h - 1);
            let si = sy as usize * frame.stride + sx as usize * 4;
            let di = y as usize * frame.stride + x as usize * 4;
            out[di..di + 4].copy_from_slice(&frame.data[si..si + 4]);
        }
    }
    Frame { data: std::sync::Arc::new(out), ..frame.clone() }
}

pub fn to_bgra(frame: &Frame) -> Frame {
    if frame.format == PixelFormat::Bgra8 {
        return frame.clone();
    }
    let mut data = (*frame.data).clone();
    for px in data.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    Frame { data: std::sync::Arc::new(data), format: PixelFormat::Bgra8, ..frame.clone() }
}

/// Speech-like synthetic audio at 16 kHz: harmonic "voiced" bursts with syllable
/// amplitude modulation, separated by pauses. Returns (samples, speech intervals ms).
pub fn speech_like(pattern_ms: &[(u32, bool)], seed: u64) -> (Vec<f32>, Vec<(u64, u64)>) {
    let mut rng = Lcg(seed);
    let mut out = Vec::new();
    let mut spans = Vec::new();
    let mut t_ms = 0u64;
    for &(dur, speech) in pattern_ms {
        let n = dur as usize * 16;
        let f0 = 110.0 + (rng.next() % 80) as f32;
        let start = out.len();
        for i in 0..n {
            let t = (start + i) as f32 / 16000.0;
            let noise = ((rng.next() % 2001) as f32 / 1000.0 - 1.0) * 0.002;
            let v = if speech {
                let syl = (0.5 + 0.5 * (2.0 * std::f32::consts::PI * 4.0 * t).sin()).powf(0.7);
                let mut s = 0.0;
                for k in 1..6 {
                    s += (2.0 * std::f32::consts::PI * f0 * k as f32 * t).sin() / k as f32;
                }
                0.25 * syl * s + noise
            } else {
                noise
            };
            out.push(v);
        }
        if speech {
            spans.push((t_ms, t_ms + dur as u64));
        }
        t_ms += dur as u64;
    }
    (out, spans)
}
