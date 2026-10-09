//! Lecture clock. All timeline timestamps are milliseconds since lecture start.
//!
//! The production clock is based on wall time (`SystemTime`) rather than a monotonic
//! clock on purpose: on macOS the monotonic clock stops while the machine sleeps, and
//! we want lecture time to keep matching the real world after a sleep/wake cycle.

use chrono::{DateTime, Duration, Local};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

pub trait Clock: Send + Sync {
    /// Milliseconds since lecture start.
    fn now_ms(&self) -> u64;
    /// Local wall-clock time at which the lecture started.
    fn started_at(&self) -> DateTime<Local>;
    fn wall_at(&self, ms: u64) -> DateTime<Local> {
        self.started_at() + Duration::milliseconds(ms as i64)
    }
}

pub struct SystemClock {
    start: SystemTime,
    start_local: DateTime<Local>,
    offset_ms: u64,
}

impl SystemClock {
    pub fn new() -> Self {
        let start = SystemTime::now();
        Self { start, start_local: DateTime::<Local>::from(start), offset_ms: 0 }
    }

    /// Resume an existing lecture clock (used after crash recovery).
    pub fn resume(started_at: DateTime<Local>) -> Self {
        let start: SystemTime = started_at.into();
        Self { start, start_local: started_at, offset_ms: 0 }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(self.start)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
            + self.offset_ms
    }
    fn started_at(&self) -> DateTime<Local> {
        self.start_local
    }
}

/// Deterministic clock for tests.
pub struct ManualClock {
    ms: AtomicU64,
    start: DateTime<Local>,
}

impl ManualClock {
    pub fn new() -> Self {
        Self { ms: AtomicU64::new(0), start: Local::now() }
    }
    pub fn set(&self, ms: u64) {
        self.ms.store(ms, Ordering::SeqCst);
    }
    pub fn advance(&self, ms: u64) {
        self.ms.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.ms.load(Ordering::SeqCst)
    }
    fn started_at(&self) -> DateTime<Local> {
        self.start
    }
}
