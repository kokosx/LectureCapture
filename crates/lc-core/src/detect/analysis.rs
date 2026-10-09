//! Cheap frame features used by the slide detector.
//!
//! A captured region (native resolution, e.g. 2560×1600) is box-averaged down to a
//! small grayscale image (default 320 px wide). All comparisons work on that image,
//! so the per-frame cost is a single pass over the source pixels plus a few hundred
//! thousand integer operations – negligible at 1–2 FPS.

use crate::config::DetectorConfig;
use crate::frame::{Frame, NormRect};

#[derive(Clone, Debug)]
pub struct Analysis {
    pub w: usize,
    pub h: usize,
    pub luma: Vec<u8>,
    /// `true` = pixel excluded from comparison (user mask).
    pub mask: Vec<bool>,
    pub mean: f32,
    pub std: f32,
    /// 64-bit difference hash (gradient sign of a 9×8 thumbnail).
    pub dhash: u64,
    /// Source (cropped) size in native pixels.
    pub src_w: u32,
    pub src_h: u32,
}

impl Analysis {
    /// Dominant (background) luma value of the unmasked image.
    pub fn background_luma(&self) -> u8 {
        let mut hist = [0u32; 64];
        for (i, &v) in self.luma.iter().enumerate() {
            if !self.mask[i] {
                hist[(v >> 2) as usize] += 1;
            }
        }
        let (bin, _) = hist.iter().enumerate().max_by_key(|(_, &c)| c).unwrap();
        (bin as u8) * 4 + 2
    }

    pub fn is_blank(&self) -> bool {
        self.std < 3.0
    }

    /// Standard deviation of luma inside one block – used to tell "empty" background
    /// areas from areas with content (for build/reveal detection).
    pub fn block_std(&self, bx: usize, by: usize, bs: usize) -> f32 {
        let (x0, y0) = (bx * bs, by * bs);
        let (x1, y1) = ((x0 + bs).min(self.w), (y0 + bs).min(self.h));
        let mut n = 0f32;
        let mut s = 0f32;
        let mut s2 = 0f32;
        for y in y0..y1 {
            for x in x0..x1 {
                let v = self.luma[y * self.w + x] as f32;
                s += v;
                s2 += v * v;
                n += 1.0;
            }
        }
        if n == 0.0 {
            return 0.0;
        }
        let m = s / n;
        (s2 / n - m * m).max(0.0).sqrt()
    }
}

/// Analyze the region `crop` of `frame`.
pub fn analyze(frame: &Frame, crop: Option<&NormRect>, cfg: &DetectorConfig) -> Analysis {
    let (cx0, cy0, cx1, cy1) =
        crop.copied().unwrap_or(NormRect::FULL).to_pixels(frame.width, frame.height);
    let (cw, ch) = ((cx1 - cx0) as usize, (cy1 - cy0) as usize);
    let aw = (cfg.analysis_width as usize).clamp(8, cw.max(8)).min(cw.max(1));
    let ah = ((ch * aw) as f64 / cw as f64).round().max(1.0) as usize;

    // Precompute source column ranges for every output column.
    let col_ranges: Vec<(usize, usize)> = (0..aw)
        .map(|ox| {
            let a = cx0 as usize + ox * cw / aw;
            let b = (cx0 as usize + (ox + 1) * cw / aw).max(a + 1);
            (a, b)
        })
        .collect();

    let mut luma = vec![0u8; aw * ah];
    let (ri, gi, bi) = match frame.format {
        crate::frame::PixelFormat::Bgra8 => (2, 1, 0),
        crate::frame::PixelFormat::Rgba8 => (0, 1, 2),
    };
    let mut acc = vec![0u32; aw];
    let mut cnt = vec![0u32; aw];
    for oy in 0..ah {
        let y_a = cy0 as usize + oy * ch / ah;
        let y_b = (cy0 as usize + (oy + 1) * ch / ah).max(y_a + 1);
        acc.iter_mut().for_each(|v| *v = 0);
        cnt.iter_mut().for_each(|v| *v = 0);
        // Sub-sample rows/cols with a step so huge sources stay cheap; still averages
        // many pixels per output cell which suppresses noise.
        let row_step = ((y_b - y_a) / 8).max(1);
        let mut y = y_a;
        while y < y_b {
            let row = &frame.data[y * frame.stride..];
            for (ox, &(a, b)) in col_ranges.iter().enumerate() {
                let col_step = ((b - a) / 8).max(1);
                let mut x = a;
                while x < b {
                    let p = &row[x * 4..x * 4 + 4];
                    acc[ox] += (p[ri] as u32 * 77 + p[gi] as u32 * 150 + p[bi] as u32 * 29) >> 8;
                    cnt[ox] += 1;
                    x += col_step;
                }
            }
            y += row_step;
        }
        for ox in 0..aw {
            luma[oy * aw + ox] = (acc[ox] / cnt[ox].max(1)) as u8;
        }
    }

    let mut mask = vec![false; aw * ah];
    for m in &cfg.masks {
        let (x0, y0, x1, y1) = m.to_pixels(aw as u32, ah as u32);
        for y in y0..y1 {
            for x in x0..x1 {
                mask[y as usize * aw + x as usize] = true;
            }
        }
    }

    let (mut s, mut s2, mut n) = (0f64, 0f64, 0f64);
    for (i, &v) in luma.iter().enumerate() {
        if !mask[i] {
            let v = v as f64;
            s += v;
            s2 += v * v;
            n += 1.0;
        }
    }
    let mean = if n > 0.0 { s / n } else { 0.0 };
    let std = if n > 0.0 { (s2 / n - mean * mean).max(0.0).sqrt() } else { 0.0 };

    let dhash = dhash(&luma, aw, ah);
    Analysis {
        w: aw,
        h: ah,
        luma,
        mask,
        mean: mean as f32,
        std: std as f32,
        dhash,
        src_w: cx1 - cx0,
        src_h: cy1 - cy0,
    }
}

fn resize_box(luma: &[u8], w: usize, h: usize, tw: usize, th: usize) -> Vec<u8> {
    let mut out = vec![0u8; tw * th];
    for ty in 0..th {
        let (ya, yb) = (ty * h / th, ((ty + 1) * h / th).max(ty * h / th + 1));
        for tx in 0..tw {
            let (xa, xb) = (tx * w / tw, ((tx + 1) * w / tw).max(tx * w / tw + 1));
            let mut s = 0u32;
            let mut n = 0u32;
            for y in ya..yb.min(h) {
                for x in xa..xb.min(w) {
                    s += luma[y * w + x] as u32;
                    n += 1;
                }
            }
            out[ty * tw + tx] = (s / n.max(1)) as u8;
        }
    }
    out
}

pub fn dhash(luma: &[u8], w: usize, h: usize) -> u64 {
    let t = resize_box(luma, w, h, 9, 8);
    let mut bits = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            if t[y * 9 + x] < t[y * 9 + x + 1] {
                bits |= 1 << (y * 8 + x);
            }
        }
    }
    bits
}

pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Result of comparing two analysis images.
#[derive(Clone, Debug, Default)]
pub struct Diff {
    pub changed_fraction: f32,
    pub blocks_x: usize,
    pub blocks_y: usize,
    pub changed: Vec<bool>,
    pub changed_blocks: usize,
    /// Per-pixel change map (analysis resolution).
    pub changed_px: Vec<bool>,
    /// Connected components of changed blocks: (count, bbox_w, bbox_h, block indices).
    pub components: Vec<Component>,
    /// Images had different geometry (source resized) – treat as a full change.
    pub geometry_changed: bool,
}

#[derive(Clone, Debug)]
pub struct Component {
    pub blocks: Vec<usize>,
    pub bbox_w: usize,
    pub bbox_h: usize,
}

impl Diff {
    /// Whether the change is meaningful content change (not cursor / noise).
    pub fn is_significant(&self, cfg: &DetectorConfig) -> bool {
        if self.geometry_changed {
            return true;
        }
        if self.changed_fraction >= 0.25 {
            return true;
        }
        let max = cfg.cursor_max_blocks as usize;
        let mut small = 0;
        for c in &self.components {
            if c.bbox_w > max || c.bbox_h > max {
                return true;
            }
            small += 1;
        }
        small > cfg.max_small_components as usize
    }
}

/// Shift-tolerant comparison: a pixel only counts as changed if no pixel within
/// `shift_tolerance` in the other image is close to it. This absorbs sub-pixel
/// shifts, scaling jitter and compression ringing around text edges.
pub fn compare(a: &Analysis, b: &Analysis, cfg: &DetectorConfig) -> Diff {
    if a.w != b.w || a.h != b.h {
        return Diff { changed_fraction: 1.0, geometry_changed: true, ..Default::default() };
    }
    let (w, h) = (a.w, a.h);
    let r = cfg.shift_tolerance as isize;
    let thr = cfg.pixel_threshold as i16;
    let bs = cfg.block_size.max(2) as usize;
    let (bx_n, by_n) = (w.div_ceil(bs), h.div_ceil(bs));
    let mut block_changed = vec![0u32; bx_n * by_n];
    let mut block_total = vec![0u32; bx_n * by_n];
    let mut changed_px = 0usize;
    let mut total_px = 0usize;
    let mut px_map = vec![false; w * h];

    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if a.mask[i] || b.mask[i] {
                continue;
            }
            total_px += 1;
            let bi = (y / bs) * bx_n + x / bs;
            block_total[bi] += 1;
            let va = a.luma[i] as i16;
            let direct = (va - b.luma[i] as i16).abs();
            if direct <= thr {
                continue;
            }
            // symmetric tolerance: a vs neighbourhood of b AND b vs neighbourhood of a
            let near = |src: &Analysis, v: i16| -> bool {
                for dy in -r..=r {
                    let yy = y as isize + dy;
                    if yy < 0 || yy >= h as isize {
                        continue;
                    }
                    for dx in -r..=r {
                        let xx = x as isize + dx;
                        if xx < 0 || xx >= w as isize {
                            continue;
                        }
                        if (v - src.luma[yy as usize * w + xx as usize] as i16).abs() <= thr {
                            return true;
                        }
                    }
                }
                false
            };
            if near(b, va) && near(a, b.luma[i] as i16) {
                continue;
            }
            changed_px += 1;
            px_map[i] = true;
            block_changed[bi] += 1;
        }
    }

    let changed: Vec<bool> = block_changed
        .iter()
        .zip(&block_total)
        .map(|(&c, &t)| t > 0 && c >= 3 && c as f32 / t as f32 >= cfg.block_change_ratio)
        .collect();
    let changed_blocks = changed.iter().filter(|&&c| c).count();
    let components = components(&changed, bx_n, by_n);
    Diff {
        changed_fraction: if total_px > 0 { changed_px as f32 / total_px as f32 } else { 0.0 },
        blocks_x: bx_n,
        blocks_y: by_n,
        changed,
        changed_blocks,
        changed_px: px_map,
        components,
        geometry_changed: false,
    }
}

fn components(changed: &[bool], bw: usize, bh: usize) -> Vec<Component> {
    let mut seen = vec![false; changed.len()];
    let mut out = Vec::new();
    for start in 0..changed.len() {
        if !changed[start] || seen[start] {
            continue;
        }
        let mut stack = vec![start];
        seen[start] = true;
        let mut blocks = Vec::new();
        let (mut minx, mut miny, mut maxx, mut maxy) = (usize::MAX, usize::MAX, 0, 0);
        while let Some(i) = stack.pop() {
            blocks.push(i);
            let (x, y) = (i % bw, i / bw);
            minx = minx.min(x);
            maxx = maxx.max(x);
            miny = miny.min(y);
            maxy = maxy.max(y);
            for dy in -1isize..=1 {
                for dx in -1isize..=1 {
                    let (nx, ny) = (x as isize + dx, y as isize + dy);
                    if nx < 0 || ny < 0 || nx >= bw as isize || ny >= bh as isize {
                        continue;
                    }
                    let j = ny as usize * bw + nx as usize;
                    if changed[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        out.push(Component { blocks, bbox_w: maxx - minx + 1, bbox_h: maxy - miny + 1 });
    }
    out
}
