//! The server's monotonic clock, in microseconds. Every Sendspin timestamp
//! (audio chunks, `server/time`) is in this time domain.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

pub trait Clock: Send + Sync {
    /// Microseconds on the server's monotonic clock.
    fn now_us(&self) -> i64;
}

/// Microseconds since construction, from [`Instant`].
#[derive(Debug, Clone, Copy)]
pub struct MonotonicClock {
    origin: Instant,
}

impl MonotonicClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MonotonicClock {
    fn now_us(&self) -> i64 {
        i64::try_from(self.origin.elapsed().as_micros()).unwrap_or(i64::MAX)
    }
}

/// A clock tests move by hand.
#[derive(Debug, Default)]
pub struct FakeClock {
    now: AtomicI64,
}

impl FakeClock {
    pub fn set(&self, us: i64) {
        self.now.store(us, Ordering::SeqCst);
    }

    pub fn advance(&self, us: i64) {
        self.now.fetch_add(us, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now_us(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_clock_sets_and_advances() {
        let c = FakeClock::default();
        c.set(1_000);
        c.advance(500);
        assert_eq!(c.now_us(), 1_500);
    }

    #[test]
    fn monotonic_clock_never_goes_back() {
        let c = MonotonicClock::new();
        let a = c.now_us();
        let b = c.now_us();
        assert!(a >= 0 && b >= a);
    }
}
