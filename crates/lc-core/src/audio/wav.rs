//! Minimal 16-bit PCM mono WAV writer/reader for the on-disk transcription queue.

use anyhow::{bail, Context, Result};
use std::io::{Read, Write};
use std::path::Path;

pub fn encode_wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = samples.len() as u32 * 2;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

pub fn write_wav(path: &Path, samples: &[f32], sample_rate: u32) -> Result<()> {
    crate::session::fsutil::atomic_write(path, &encode_wav(samples, sample_rate))
}

/// Reads a PCM16 / float32 mono-or-stereo WAV and returns mono f32 + sample rate.
pub fn read_wav(path: &Path) -> Result<(Vec<f32>, u32)> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .with_context(|| format!("open {}", path.display()))?
        .read_to_end(&mut bytes)?;
    decode_wav(&bytes)
}

pub fn decode_wav(bytes: &[u8]) -> Result<(Vec<f32>, u32)> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bail!("not a WAV file");
    }
    let mut i = 12;
    let (mut fmt, mut channels, mut rate, mut bits) = (0u16, 1u16, 16_000u32, 16u16);
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let len = u32::from_le_bytes(bytes[i + 4..i + 8].try_into().unwrap()) as usize;
        let body = &bytes[i + 8..(i + 8 + len).min(bytes.len())];
        if id == b"fmt " {
            fmt = u16::from_le_bytes([body[0], body[1]]);
            channels = u16::from_le_bytes([body[2], body[3]]);
            rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            let ch = channels.max(1) as usize;
            let mono: Vec<f32> = match (fmt, bits) {
                (1, 16) => body
                    .chunks_exact(2 * ch)
                    .map(|f| {
                        f.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).sum::<f32>()
                            / ch as f32
                    })
                    .collect(),
                (3, 32) => body
                    .chunks_exact(4 * ch)
                    .map(|f| {
                        f.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).sum::<f32>() / ch as f32
                    })
                    .collect(),
                _ => bail!("unsupported WAV format {fmt}/{bits}"),
            };
            return Ok((mono, rate));
        }
        i += 8 + len + (len & 1);
    }
    bail!("WAV without data chunk")
}

/// Streaming writer used by tests / CLI to dump raw audio.
pub struct WavStreamWriter {
    file: std::fs::File,
    samples: u32,
    rate: u32,
}

impl WavStreamWriter {
    pub fn create(path: &Path, rate: u32) -> Result<Self> {
        let mut file = std::fs::File::create(path)?;
        file.write_all(&encode_wav(&[], rate))?;
        Ok(Self { file, samples: 0, rate })
    }
    pub fn write(&mut self, samples: &[f32]) -> Result<()> {
        let mut buf = Vec::with_capacity(samples.len() * 2);
        for s in samples {
            buf.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
        }
        self.file.write_all(&buf)?;
        self.samples += samples.len() as u32;
        Ok(())
    }
    pub fn finish(mut self) -> Result<()> {
        use std::io::{Seek, SeekFrom};
        let header = encode_wav(&[], self.rate);
        let data_len = self.samples * 2;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&header[..4])?;
        self.file.write_all(&(36 + data_len).to_le_bytes())?;
        self.file.seek(SeekFrom::Start(40))?;
        self.file.write_all(&data_len.to_le_bytes())?;
        self.file.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let s: Vec<f32> = (0..1000).map(|i| ((i as f32) / 100.0).sin() * 0.5).collect();
        let (d, rate) = decode_wav(&encode_wav(&s, 16_000)).unwrap();
        assert_eq!(rate, 16_000);
        assert_eq!(d.len(), s.len());
        assert!(s.iter().zip(&d).all(|(a, b)| (a - b).abs() < 1e-3));
    }
}
