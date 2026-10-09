//! Output generation: lossless slide images, Markdown documents, PROMPT.md, ZIP.

pub mod markdown;
pub mod prompt;
pub mod zip;

use crate::config::PngCompression;
use crate::frame::Frame;
use crate::session::fsutil::{atomic_write, read_jsonl};
use crate::session::LectureSession;
use crate::transcript::{assign, merge, Assignment, SegmentRecord};
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::path::Path;

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Encode a frame as lossless RGB PNG (native resolution, no filtering of content).
pub fn encode_png(frame: &Frame, compression: PngCompression) -> Result<Vec<u8>> {
    let rgb = frame.to_rgb8();
    let mut out = Vec::with_capacity(rgb.len() / 4);
    {
        let mut enc = png::Encoder::new(&mut out, frame.width, frame.height);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(match compression {
            PngCompression::Fast => png::Compression::Fast,
            PngCompression::Balanced => png::Compression::Balanced,
            PngCompression::Best => png::Compression::High,
        });
        let mut w = enc.write_header()?;
        w.write_image_data(&rgb)?;
        w.finish()?;
    }
    Ok(out)
}

/// Lossless WebP (optional archive format).
pub fn encode_webp_lossless(frame: &Frame) -> Result<Vec<u8>> {
    use image::ImageEncoder;
    let rgb = frame.to_rgb8();
    let mut out = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut out).write_image(
        &rgb,
        frame.width,
        frame.height,
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(out)
}

/// Write the PNG atomically; returns (sha256, bytes).
pub fn write_slide_png(frame: &Frame, path: &Path, compression: PngCompression) -> Result<(String, u64)> {
    let bytes = encode_png(frame, compression)?;
    atomic_write(path, &bytes)?;
    Ok((sha256_hex(&bytes), bytes.len() as u64))
}

pub fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[1..4] != b"PNG" {
        return None;
    }
    Some((
        u32::from_be_bytes(bytes[16..20].try_into().ok()?),
        u32::from_be_bytes(bytes[20..24].try_into().ok()?),
    ))
}

/// Everything the document renderers need, loaded from a lecture folder.
pub struct LectureData {
    pub records: Vec<SegmentRecord>,
    pub assignment: Assignment,
}

pub fn load_transcript(session: &LectureSession) -> Result<LectureData> {
    let raw: Vec<SegmentRecord> = read_jsonl(&session.dir.segments_path())?;
    let records = merge(&raw);
    let assignment = assign(&records, &session.manifest.closed_timeline());
    Ok(LectureData { records, assignment })
}

/// (Re)generate lecture.md, transcript/full.md, transcript/by-slide.md and PROMPT.md.
pub fn write_documents(session: &LectureSession) -> Result<()> {
    let data = load_transcript(session)?;
    let m = &session.manifest;
    let d = &session.dir;
    atomic_write(&d.abs("transcript/full.md"), markdown::full_md(m, &data).as_bytes())?;
    atomic_write(&d.abs("transcript/by-slide.md"), markdown::by_slide_md(m, &data).as_bytes())?;
    atomic_write(&d.abs("lecture.md"), markdown::lecture_md(m, &data).as_bytes())?;
    atomic_write(&d.abs("PROMPT.md"), prompt::prompt_md(m, &data).as_bytes())?;
    Ok(())
}
