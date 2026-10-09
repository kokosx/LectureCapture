//! Raw captured frames and normalized regions.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PixelFormat {
    Bgra8,
    Rgba8,
}

/// A captured frame in native resolution. `data` is shared so frames can be passed
/// between threads without copying the pixels.
#[derive(Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Bytes per row (may be larger than `width * 4`).
    pub stride: usize,
    pub format: PixelFormat,
    pub data: Arc<Vec<u8>>,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Frame({}x{} {:?})", self.width, self.height, self.format)
    }
}

/// Rectangle in normalized coordinates (0..1) relative to the source frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NormRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl NormRect {
    pub const FULL: NormRect = NormRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };

    pub fn is_full(&self) -> bool {
        self.x <= 0.0 && self.y <= 0.0 && self.x + self.w >= 1.0 && self.y + self.h >= 1.0
    }

    /// Pixel rectangle `(x0, y0, x1, y1)` (exclusive end), clamped, at least 1x1.
    pub fn to_pixels(&self, width: u32, height: u32) -> (u32, u32, u32, u32) {
        let clamp = |v: f32| v.clamp(0.0, 1.0);
        let x0 = (clamp(self.x) * width as f32).floor() as u32;
        let y0 = (clamp(self.y) * height as f32).floor() as u32;
        let x1 = (clamp(self.x + self.w) * width as f32).round() as u32;
        let y1 = (clamp(self.y + self.h) * height as f32).round() as u32;
        let x0 = x0.min(width.saturating_sub(1));
        let y0 = y0.min(height.saturating_sub(1));
        (x0, y0, x1.max(x0 + 1).min(width), y1.max(y0 + 1).min(height))
    }
}

impl Frame {
    pub fn new(width: u32, height: u32, format: PixelFormat, data: Vec<u8>) -> Self {
        let stride = width as usize * 4;
        assert!(data.len() >= stride * height as usize, "frame buffer too small");
        Self { width, height, stride, format, data: Arc::new(data) }
    }

    /// Solid-colour RGBA frame (handy for tests).
    pub fn solid(width: u32, height: u32, rgb: [u8; 3]) -> Self {
        let mut data = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..(width * height) {
            data.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
        Self::new(width, height, PixelFormat::Rgba8, data)
    }

    #[inline]
    pub fn rgb_at(&self, x: u32, y: u32) -> [u8; 3] {
        let i = y as usize * self.stride + x as usize * 4;
        let p = &self.data[i..i + 4];
        match self.format {
            PixelFormat::Bgra8 => [p[2], p[1], p[0]],
            PixelFormat::Rgba8 => [p[0], p[1], p[2]],
        }
    }

    /// Copy of the cropped region (native resolution, no scaling).
    pub fn crop(&self, rect: Option<&NormRect>) -> Frame {
        let Some(rect) = rect.filter(|r| !r.is_full()) else {
            return self.clone();
        };
        let (x0, y0, x1, y1) = rect.to_pixels(self.width, self.height);
        let w = x1 - x0;
        let h = y1 - y0;
        let mut out = Vec::with_capacity(w as usize * h as usize * 4);
        for y in y0..y1 {
            let start = y as usize * self.stride + x0 as usize * 4;
            out.extend_from_slice(&self.data[start..start + w as usize * 4]);
        }
        Frame { width: w, height: h, stride: w as usize * 4, format: self.format, data: Arc::new(out) }
    }

    /// Tightly packed RGB8 pixels (alpha dropped – captured screens are opaque).
    pub fn to_rgb8(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.width as usize * self.height as usize * 3);
        for y in 0..self.height as usize {
            let row = &self.data[y * self.stride..y * self.stride + self.width as usize * 4];
            match self.format {
                PixelFormat::Rgba8 => {
                    for p in row.chunks_exact(4) {
                        out.extend_from_slice(&p[..3]);
                    }
                }
                PixelFormat::Bgra8 => {
                    for p in row.chunks_exact(4) {
                        out.extend_from_slice(&[p[2], p[1], p[0]]);
                    }
                }
            }
        }
        out
    }

    /// Mutable RGBA access for synthetic test frames.
    pub fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, rgb: [u8; 3]) {
        let format = self.format;
        let stride = self.stride;
        let (width, height) = (self.width, self.height);
        let data = Arc::make_mut(&mut self.data);
        for yy in y.min(height)..(y + h).min(height) {
            for xx in x.min(width)..(x + w).min(width) {
                let i = yy as usize * stride + xx as usize * 4;
                let px = match format {
                    PixelFormat::Rgba8 => [rgb[0], rgb[1], rgb[2], 255],
                    PixelFormat::Bgra8 => [rgb[2], rgb[1], rgb[0], 255],
                };
                data[i..i + 4].copy_from_slice(&px);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_keeps_native_resolution() {
        let mut f = Frame::solid(100, 50, [255, 255, 255]);
        f.fill_rect(50, 25, 10, 10, [255, 0, 0]);
        let c = f.crop(Some(&NormRect { x: 0.5, y: 0.5, w: 0.5, h: 0.5 }));
        assert_eq!((c.width, c.height), (50, 25));
        assert_eq!(c.rgb_at(0, 0), [255, 0, 0]);
        assert_eq!(c.rgb_at(10, 10), [255, 255, 255]);
    }

    #[test]
    fn bgra_to_rgb() {
        let f = Frame::new(1, 1, PixelFormat::Bgra8, vec![1, 2, 3, 255]);
        assert_eq!(f.to_rgb8(), vec![3, 2, 1]);
    }
}
