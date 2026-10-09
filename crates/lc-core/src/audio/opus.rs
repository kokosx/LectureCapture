//! Streaming Ogg/Opus (RFC 7845) writer and reader, 16 kHz mono speech.
//!
//! The writer closes an Ogg page every second and fsyncs every few seconds, so after
//! a crash the file is valid up to the last complete page (at most a few seconds lost).

use anyhow::{anyhow, bail, Context, Result};
use audiopus_sys as ffi;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;

const RATE: i32 = 16_000;
const FRAME: usize = 320; // 20 ms @ 16 kHz
const GRANULE_PER_FRAME: u64 = 960; // 20 ms @ 48 kHz (Ogg Opus granules are 48 kHz)
const PACKETS_PER_PAGE: u32 = 50;

pub struct OggOpusWriter {
    writer: ogg::PacketWriter<'static, BufWriter<File>>,
    enc: *mut ffi::OpusEncoder,
    serial: u32,
    pre_skip: u64,
    pending: Vec<f32>,
    samples_in: u64,
    frames_out: u64,
    packets_in_page: u32,
    pages_since_sync: u32,
    out_buf: Vec<u8>,
}

// The encoder pointer is only used from the owning thread.
unsafe impl Send for OggOpusWriter {}

impl OggOpusWriter {
    pub fn create(path: &Path, bitrate: u32) -> Result<Self> {
        let file = File::create(path).with_context(|| format!("create {}", path.display()))?;
        let mut err = 0i32;
        let enc = unsafe { ffi::opus_encoder_create(RATE, 1, ffi::OPUS_APPLICATION_VOIP, &mut err) };
        if enc.is_null() || err != ffi::OPUS_OK {
            bail!("opus_encoder_create failed: {err}");
        }
        let mut lookahead: i32 = 0;
        unsafe {
            ffi::opus_encoder_ctl(enc, ffi::OPUS_SET_BITRATE_REQUEST, bitrate as i32);
            ffi::opus_encoder_ctl(enc, ffi::OPUS_SET_SIGNAL_REQUEST, ffi::OPUS_SIGNAL_VOICE);
            ffi::opus_encoder_ctl(enc, ffi::OPUS_GET_LOOKAHEAD_REQUEST, &mut lookahead as *mut i32);
        }
        let pre_skip = (lookahead.max(0) as u64) * 3;
        let serial = (std::process::id() ^ 0x4c43_5054) | 1;
        let mut writer = ogg::PacketWriter::new(BufWriter::new(file));

        let mut head = Vec::with_capacity(19);
        head.extend_from_slice(b"OpusHead");
        head.push(1); // version
        head.push(1); // channels
        head.extend_from_slice(&(pre_skip as u16).to_le_bytes());
        head.extend_from_slice(&(RATE as u32).to_le_bytes()); // original input rate
        head.extend_from_slice(&0i16.to_le_bytes()); // output gain
        head.push(0); // mapping family
        writer.write_packet(head, serial, ogg::PacketWriteEndInfo::EndPage, 0)?;

        let vendor = format!("LectureCapture {}", crate::APP_VERSION);
        let mut tags = Vec::new();
        tags.extend_from_slice(b"OpusTags");
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor.as_bytes());
        tags.extend_from_slice(&0u32.to_le_bytes());
        writer.write_packet(tags, serial, ogg::PacketWriteEndInfo::EndPage, 0)?;
        writer.inner_mut().flush()?;

        Ok(Self {
            writer,
            enc,
            serial,
            pre_skip,
            pending: Vec::with_capacity(FRAME * 4),
            samples_in: 0,
            frames_out: 0,
            packets_in_page: 0,
            pages_since_sync: 0,
            out_buf: vec![0u8; 4000],
        })
    }

    /// Duration written so far in samples (16 kHz).
    pub fn samples_written(&self) -> u64 {
        self.samples_in
    }

    pub fn write(&mut self, samples: &[f32]) -> Result<()> {
        self.samples_in += samples.len() as u64;
        self.pending.extend_from_slice(samples);
        let mut off = 0;
        while self.pending.len() - off >= FRAME {
            let frame: Vec<f32> = self.pending[off..off + FRAME].to_vec();
            self.encode_frame(&frame, false)?;
            off += FRAME;
        }
        self.pending.drain(..off);
        Ok(())
    }

    /// Writes `n` samples of silence without allocating them all.
    pub fn write_silence(&mut self, n: u64) -> Result<()> {
        let zeros = [0f32; FRAME];
        let mut left = n;
        while left > 0 {
            let k = left.min(FRAME as u64) as usize;
            self.write(&zeros[..k])?;
            left -= k as u64;
        }
        Ok(())
    }

    fn encode_frame(&mut self, frame: &[f32], last: bool) -> Result<()> {
        let n = unsafe {
            ffi::opus_encode_float(
                self.enc,
                frame.as_ptr(),
                FRAME as i32,
                self.out_buf.as_mut_ptr(),
                self.out_buf.len() as i32,
            )
        };
        if n < 0 {
            bail!("opus_encode_float failed: {n}");
        }
        self.frames_out += 1;
        self.packets_in_page += 1;
        let granule = if last {
            self.pre_skip + self.samples_in * 3
        } else {
            self.pre_skip + self.frames_out * GRANULE_PER_FRAME
        };
        let info = if last {
            ogg::PacketWriteEndInfo::EndStream
        } else if self.packets_in_page >= PACKETS_PER_PAGE {
            ogg::PacketWriteEndInfo::EndPage
        } else {
            ogg::PacketWriteEndInfo::NormalPacket
        };
        let packet = self.out_buf[..n as usize].to_vec();
        self.writer.write_packet(packet, self.serial, info, granule)?;
        if !matches!(info, ogg::PacketWriteEndInfo::NormalPacket) {
            self.packets_in_page = 0;
            self.pages_since_sync += 1;
            self.writer.inner_mut().flush()?;
            if self.pages_since_sync >= 5 {
                self.pages_since_sync = 0;
                let _ = self.writer.inner_mut().get_ref().sync_data();
            }
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        let mut frame = std::mem::take(&mut self.pending);
        frame.resize(FRAME, 0.0);
        self.encode_frame(&frame, true)?;
        let inner = self.writer.inner_mut();
        inner.flush()?;
        inner.get_ref().sync_all()?;
        Ok(())
    }
}

impl Drop for OggOpusWriter {
    fn drop(&mut self) {
        unsafe { ffi::opus_encoder_destroy(self.enc) };
    }
}

/// Decodes an Ogg/Opus file to 16 kHz mono, streaming blocks to `on_block`.
/// Tolerates a truncated tail (crash). Returns total samples delivered.
pub fn decode_ogg_opus(path: &Path, mut on_block: impl FnMut(&[f32]) -> Result<()>) -> Result<u64> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut reader = ogg::PacketReader::new(BufReader::new(file));
    let mut err = 0i32;
    let dec = unsafe { ffi::opus_decoder_create(RATE, 1, &mut err) };
    if dec.is_null() || err != ffi::OPUS_OK {
        bail!("opus_decoder_create failed: {err}");
    }
    struct Guard(*mut ffi::OpusDecoder);
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe { ffi::opus_decoder_destroy(self.0) };
        }
    }
    let _guard = Guard(dec);

    let mut out = vec![0f32; 5760];
    let mut header_seen = false;
    let mut skip: u64 = 0;
    let mut pre_skip48: u64 = 0;
    let mut delivered: u64 = 0;
    let mut packet_index = 0u64;
    loop {
        let packet = match reader.read_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(e) => {
                log::warn!("ogg read stopped (truncated file?): {e}");
                break;
            }
        };
        packet_index += 1;
        if !header_seen {
            if packet.data.len() < 19 || &packet.data[..8] != b"OpusHead" {
                return Err(anyhow!("not an Ogg Opus stream"));
            }
            pre_skip48 = u16::from_le_bytes([packet.data[10], packet.data[11]]) as u64;
            skip = pre_skip48 / 3;
            header_seen = true;
            continue;
        }
        if packet_index == 2 && packet.data.starts_with(b"OpusTags") {
            continue;
        }
        let n = unsafe {
            ffi::opus_decode_float(dec, packet.data.as_ptr(), packet.data.len() as i32, out.as_mut_ptr(), out.len() as i32, 0)
        };
        if n < 0 {
            log::warn!("opus decode error {n}, skipping packet");
            continue;
        }
        let mut block = &out[..n as usize];
        if skip > 0 {
            let s = skip.min(block.len() as u64) as usize;
            block = &block[s..];
            skip -= s as u64;
        }
        // last packet: trim encoder padding using the granule position
        if packet.last_in_stream() {
            let total = packet.absgp_page().saturating_sub(pre_skip48) / 3;
            let allowed = total.saturating_sub(delivered) as usize;
            block = &block[..block.len().min(allowed)];
        }
        if !block.is_empty() {
            on_block(block)?;
            delivered += block.len() as u64;
        }
    }
    Ok(delivered)
}

/// Duration of an Ogg/Opus file in ms (decodes the stream; used for recovery).
pub fn ogg_opus_duration_ms(path: &Path) -> Result<u64> {
    let n = decode_ogg_opus(path, |_| Ok(()))?;
    Ok(n / 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_preserves_length_and_signal() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.ogg");
        let mut w = OggOpusWriter::create(&p, 32_000).unwrap();
        let tone: Vec<f32> =
            (0..16_000 * 3 + 123).map(|i| (2.0 * std::f32::consts::PI * 300.0 * i as f32 / 16_000.0).sin() * 0.4).collect();
        for c in tone.chunks(777) {
            w.write(c).unwrap();
        }
        w.write_silence(16_000).unwrap();
        w.finish().unwrap();
        let mut decoded = Vec::new();
        let n = decode_ogg_opus(&p, |b| {
            decoded.extend_from_slice(b);
            Ok(())
        })
        .unwrap();
        assert_eq!(n as usize, tone.len() + 16_000);
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        assert!((rms(&decoded[4000..40_000]) - 0.283).abs() < 0.03);
        assert!(rms(&decoded[tone.len() + 2000..]) < 0.01);
    }

    #[test]
    fn truncated_file_is_still_readable() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("b.ogg");
        let mut w = OggOpusWriter::create(&p, 24_000).unwrap();
        w.write(&vec![0.1f32; 16_000 * 5]).unwrap();
        drop(w); // simulated crash: no finish()
        let len = std::fs::metadata(&p).unwrap().len();
        // chop off part of the last page as a crash mid-write would
        let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
        f.set_len(len - 7).unwrap();
        let n = decode_ogg_opus(&p, |_| Ok(())).unwrap();
        assert!(n >= 16_000 * 3, "decoded {n}");
    }
}
