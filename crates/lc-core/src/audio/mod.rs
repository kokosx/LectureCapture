//! Audio processing. Everything downstream of the capture backends works on
//! 16 kHz mono `f32` – the format whisper.cpp expects – so the recording, VAD
//! segmentation and transcription share one timeline (1 sample = 1/16 ms).

pub mod level;
pub mod mixer;
pub mod opus;
pub mod resample;
pub mod vad;
pub mod wav;

pub const SAMPLE_RATE: u32 = 16_000;
pub const SAMPLES_PER_MS: u64 = 16;

#[inline]
pub fn samples_to_ms(samples: u64) -> u64 {
    samples / SAMPLES_PER_MS
}

#[inline]
pub fn ms_to_samples(ms: u64) -> u64 {
    ms * SAMPLES_PER_MS
}

/// Downmix interleaved multi-channel audio to mono.
pub fn downmix_interleaved(data: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return data.to_vec();
    }
    data.chunks_exact(channels).map(|c| c.iter().sum::<f32>() / channels as f32).collect()
}

/// Downmix planar (one slice per channel) audio to mono.
pub fn downmix_planar(planes: &[&[f32]]) -> Vec<f32> {
    match planes.len() {
        0 => Vec::new(),
        1 => planes[0].to_vec(),
        n => {
            let len = planes.iter().map(|p| p.len()).min().unwrap_or(0);
            (0..len).map(|i| planes.iter().map(|p| p[i]).sum::<f32>() / n as f32).collect()
        }
    }
}
