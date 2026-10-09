//! Streaming windowed-sinc resampler (arbitrary ratio, e.g. 48 kHz → 16 kHz or
//! 44.1 kHz → 16 kHz). Quality is far beyond what speech recognition needs while
//! costing ~1 M multiply-adds per second of output.

pub struct Resampler {
    in_rate: u32,
    out_rate: u32,
    step: f64,
    half: usize,
    cutoff: f64,
    buf: Vec<f32>,
    pos: f64,
}

impl Resampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        let half = 24;
        let cutoff = (out_rate as f64 / in_rate as f64).min(1.0) * 0.92;
        Self {
            in_rate,
            out_rate,
            step: in_rate as f64 / out_rate as f64,
            half,
            cutoff,
            buf: vec![0.0; half],
            pos: half as f64,
        }
    }

    pub fn in_rate(&self) -> u32 {
        self.in_rate
    }

    #[inline]
    fn kernel(&self, x: f64) -> f64 {
        let half = self.half as f64;
        if x.abs() >= half {
            return 0.0;
        }
        let t = x / half;
        let window = 0.42 + 0.5 * (std::f64::consts::PI * t).cos() + 0.08 * (2.0 * std::f64::consts::PI * t).cos();
        let arg = std::f64::consts::PI * x * self.cutoff;
        let sinc = if arg.abs() < 1e-9 { 1.0 } else { arg.sin() / arg };
        self.cutoff * sinc * window
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.in_rate == self.out_rate {
            out.extend_from_slice(input);
            return;
        }
        self.buf.extend_from_slice(input);
        let half = self.half as isize;
        loop {
            let center = self.pos.floor() as isize;
            if center + half >= self.buf.len() as isize {
                break;
            }
            let mut acc = 0.0f64;
            for n in (center - half + 1)..=(center + half) {
                if n < 0 {
                    continue;
                }
                acc += self.buf[n as usize] as f64 * self.kernel(self.pos - n as f64);
            }
            out.push(acc as f32);
            self.pos += self.step;
        }
        // drop consumed history, keep `half` samples before the current position
        let keep_from = (self.pos.floor() as isize - half).max(0) as usize;
        if keep_from > 0 {
            self.buf.drain(..keep_from);
            self.pos -= keep_from as f64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, freq: f32, secs: f32) -> Vec<f32> {
        (0..(rate as f32 * secs) as usize)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * 0.5)
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn length_and_passband() {
        for in_rate in [48_000, 44_100, 32_000] {
            let mut r = Resampler::new(in_rate, 16_000);
            let input = sine(in_rate, 440.0, 2.0);
            let mut out = Vec::new();
            // feed in odd-sized chunks to exercise streaming
            for c in input.chunks(997) {
                r.process(c, &mut out);
            }
            let expected = 32_000f32;
            assert!((out.len() as f32 - expected).abs() < 40.0, "{in_rate}: {}", out.len());
            let level = rms(&out[1000..30_000]);
            assert!((level - 0.3536).abs() < 0.02, "{in_rate}: rms {level}");
        }
    }

    #[test]
    fn stopband_attenuated() {
        // 12 kHz cannot be represented at 16 kHz – must be filtered, not aliased
        let mut r = Resampler::new(48_000, 16_000);
        let mut out = Vec::new();
        r.process(&sine(48_000, 12_000.0, 1.0), &mut out);
        assert!(rms(&out[500..15_000]) < 0.01);
    }
}
