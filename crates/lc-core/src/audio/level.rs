//! Signal level metering and "no signal" detection.

pub fn rms_db(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return -120.0;
    }
    let e = samples.iter().map(|v| v * v).sum::<f32>() / samples.len() as f32;
    10.0 * (e + 1e-12).log10()
}

pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

/// Tracks a smoothed level and how long the signal has been silent.
#[derive(Clone, Debug)]
pub struct LevelMeter {
    pub threshold_db: f32,
    pub level_db: f32,
    pub peak: f32,
    silent_since_ms: Option<u64>,
    pub total_samples: u64,
    pub last_signal_ms: Option<u64>,
}

impl LevelMeter {
    pub fn new(threshold_db: f32) -> Self {
        Self {
            threshold_db,
            level_db: -120.0,
            peak: 0.0,
            silent_since_ms: Some(0),
            total_samples: 0,
            last_signal_ms: None,
        }
    }

    pub fn process(&mut self, block: &[f32], now_ms: u64) {
        if block.is_empty() {
            return;
        }
        self.total_samples += block.len() as u64;
        let db = rms_db(block);
        // fast attack, slow release – nice for a UI meter
        self.level_db = if db > self.level_db { db } else { self.level_db * 0.7 + db * 0.3 };
        self.peak = peak(block);
        if db > self.threshold_db {
            self.silent_since_ms = None;
            self.last_signal_ms = Some(now_ms);
        } else if self.silent_since_ms.is_none() {
            self.silent_since_ms = Some(now_ms);
        }
    }

    /// Milliseconds of continuous silence (0 when signal present).
    pub fn silent_for(&self, now_ms: u64) -> u64 {
        self.silent_since_ms.map(|s| now_ms.saturating_sub(s)).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_silence_duration() {
        let mut m = LevelMeter::new(-60.0);
        m.process(&vec![0.1; 1600], 100);
        assert_eq!(m.silent_for(5_000), 0);
        m.process(&vec![0.0; 1600], 200);
        assert_eq!(m.silent_for(30_200), 30_000);
        m.process(&vec![0.2; 1600], 30_300);
        assert_eq!(m.silent_for(31_000), 0);
    }

    #[test]
    fn db_values() {
        assert!(rms_db(&[0.0; 100]) < -100.0);
        assert!((rms_db(&[1.0; 100])).abs() < 0.01);
    }
}
