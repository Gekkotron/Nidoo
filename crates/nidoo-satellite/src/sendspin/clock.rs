//! Clock sync with the server, from `client/time` / `server/time` exchanges.

use std::collections::VecDeque;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Samples kept; the lowest round trip among them sets the offset.
const KEPT: usize = 8;
/// Quick first samples, then a slow pace that still keeps the server's 35 s
/// silence limit away.
const FAST: Duration = Duration::from_secs(1);
const FAST_SAMPLES: u32 = 5;
const SLOW: Duration = Duration::from_secs(10);

/// Microseconds on this process's monotonic clock: every `local_play_at_us` is in it.
pub fn now_us() -> i64 {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    let origin = *ORIGIN.get_or_init(Instant::now);
    i64::try_from(origin.elapsed().as_micros()).unwrap_or(i64::MAX)
}

/// Maps the server's clock onto ours (NTP-style, lowest round trip of the last 8).
#[derive(Debug, Default)]
pub struct ClockSync {
    /// (round trip, server minus local), oldest first.
    samples: VecDeque<(i64, i64)>,
    count: u32,
}

impl ClockSync {
    /// One exchange: we sent at `client_tx`, the server received at `server_rx` and
    /// answered at `server_tx`, we received at `client_rx`.
    pub fn on_sample(&mut self, client_tx: i64, server_rx: i64, server_tx: i64, client_rx: i64) {
        let rtt = (client_rx - client_tx) - (server_tx - server_rx);
        let offset = ((server_rx - client_tx) + (server_tx - client_rx)) / 2;
        if self.samples.len() == KEPT {
            self.samples.pop_front();
        }
        self.samples.push_back((rtt, offset));
        self.count = self.count.saturating_add(1);
    }

    /// The local time for a server timestamp; `None` before the first sample.
    pub fn to_local(&self, server_us: i64) -> Option<i64> {
        let (_, offset) = self.samples.iter().min_by_key(|(rtt, _)| *rtt)?;
        Some(server_us - offset)
    }

    /// How long to wait before the next `client/time`.
    pub fn interval(&self) -> Duration {
        if self.count < FAST_SAMPLES {
            FAST
        } else {
            SLOW
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_none_before_first_sample() {
        assert_eq!(ClockSync::default().to_local(1_000), None);
    }

    #[test]
    fn clock_picks_lowest_rtt_sample() {
        let mut c = ClockSync::default();
        // rtt 10 ms, server ahead by 1 s
        c.on_sample(0, 1_005_000, 1_005_000, 10_000);
        assert_eq!(c.to_local(5_000_000), Some(4_000_000));
        // rtt 1 ms, server ahead by 2 s: the better sample wins
        c.on_sample(100_000, 2_100_500, 2_100_500, 101_000);
        assert_eq!(c.to_local(5_000_000), Some(3_000_000));
        // a worse one later does not replace it
        c.on_sample(200_000, 1_205_000, 1_205_000, 210_000);
        assert_eq!(c.to_local(5_000_000), Some(3_000_000));
    }

    #[test]
    fn clock_keeps_only_the_last_8_samples() {
        let mut c = ClockSync::default();
        c.on_sample(0, 500, 500, 1_000); // rtt 1 ms, offset 0
        for i in 1..=8 {
            let t = i * 1_000_000;
            c.on_sample(t, t + 5_000 + 7, t + 5_000 + 7, t + 10_000); // rtt 10 ms, offset 7
        }
        assert_eq!(c.to_local(1_000_007), Some(1_000_000));
    }

    #[test]
    fn asks_every_second_then_every_ten() {
        let mut c = ClockSync::default();
        for _ in 0..5 {
            assert_eq!(c.interval(), std::time::Duration::from_secs(1));
            c.on_sample(0, 500, 500, 1_000);
        }
        assert_eq!(c.interval(), std::time::Duration::from_secs(10));
    }

    #[test]
    fn now_us_rises() {
        let a = now_us();
        std::thread::sleep(std::time::Duration::from_millis(2));
        assert!(now_us() > a);
    }
}
