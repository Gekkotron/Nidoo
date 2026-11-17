//! Rate limits for alerts that arrive on the broker. The `<source>` in
//! `<prefix>/notification/<source>` is whatever the publisher chose, so each source gets its own
//! bucket and all of them share one more: a noisy producer cannot starve the others, and
//! inventing new sources does not get past the shared limit. Alerts raised inside the hub
//! (alarm, Frigate bridge, watchdogs, flows, MCP) never come through here.

use std::collections::HashMap;

/// One source: a burst of 10, then one alert every 6 s (10 a minute).
pub const SOURCE_BURST: u32 = 10;
pub const SOURCE_EVERY: u64 = 6;
/// Every source together: a burst of 30, then one alert every 2 s (30 a minute).
pub const ALL_BURST: u32 = 30;
pub const ALL_EVERY: u64 = 2;
/// Sources tracked before idle (full) buckets are dropped. A bucket is busy only if it took a
/// token in the last `SOURCE_BURST * SOURCE_EVERY` seconds, and the shared bucket hands out at
/// most `ALL_BURST` plus one per `ALL_EVERY` in that time, so pruning always gets the map back
/// under this bound.
pub const MAX_SOURCES: usize = 64;
const _: () = assert!(
    (ALL_BURST as u64) + (SOURCE_BURST as u64) * SOURCE_EVERY / ALL_EVERY < MAX_SOURCES as u64
);

/// A token bucket on a whole-second clock: `burst` tokens, one more every `every` seconds.
#[derive(Clone, Debug)]
struct Bucket {
    burst: u32,
    every: u64,
    level: u32,
    /// When the next token started to build up.
    since: u64,
    /// Alerts refused since the last one let through (to log once, not per drop). Only the
    /// per-source buckets use it.
    dropped: u64,
}

impl Bucket {
    fn new(burst: u32, every: u64, now: u64) -> Bucket {
        Bucket {
            burst,
            every: every.max(1),
            level: burst,
            since: now,
            dropped: 0,
        }
    }

    fn refill(&mut self, now: u64) {
        if self.level >= self.burst {
            self.since = now;
            return;
        }
        let earned = now.saturating_sub(self.since) / self.every;
        if earned == 0 {
            return;
        }
        let earned = u32::try_from(earned).unwrap_or(u32::MAX);
        self.level = self.level.saturating_add(earned).min(self.burst);
        self.since = if self.level >= self.burst {
            now
        } else {
            self.since + u64::from(earned) * self.every
        };
    }

    fn is_full(&self) -> bool {
        self.level >= self.burst
    }
}

/// What `RateLimit::check` decided. `resumed` is how many alerts from that source were
/// refused before this one; `first_drop` marks the first refusal of a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allowed { resumed: u64 },
    Refused { first_drop: bool },
}

#[derive(Debug)]
pub struct RateLimit {
    sources: HashMap<String, Bucket>,
    all: Bucket,
}

impl RateLimit {
    pub fn new(now: u64) -> RateLimit {
        RateLimit {
            sources: HashMap::new(),
            all: Bucket::new(ALL_BURST, ALL_EVERY, now),
        }
    }

    /// Takes one token from `source`'s bucket and from the shared one, or none if either is
    /// empty.
    pub fn check(&mut self, source: &str, now: u64) -> Verdict {
        self.all.refill(now);
        if !self.sources.contains_key(source) && self.sources.len() >= MAX_SOURCES {
            self.sources.retain(|_, b| {
                b.refill(now);
                !b.is_full()
            });
        }
        let bucket = self
            .sources
            .entry(source.to_string())
            .or_insert_with(|| Bucket::new(SOURCE_BURST, SOURCE_EVERY, now));
        bucket.refill(now);
        if bucket.level == 0 || self.all.level == 0 {
            bucket.dropped += 1;
            return Verdict::Refused {
                first_drop: bucket.dropped == 1,
            };
        }
        bucket.level -= 1;
        self.all.level -= 1;
        let resumed = std::mem::take(&mut bucket.dropped);
        Verdict::Allowed { resumed }
    }

    pub fn sources(&self) -> usize {
        self.sources.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(v: Verdict) -> bool {
        matches!(v, Verdict::Allowed { .. })
    }

    #[test]
    fn a_source_gets_its_burst_then_one_per_interval() {
        let mut r = RateLimit::new(0);
        for _ in 0..SOURCE_BURST {
            assert!(allowed(r.check("a", 0)));
        }
        assert_eq!(r.check("a", 0), Verdict::Refused { first_drop: true });
        assert_eq!(r.check("a", 5), Verdict::Refused { first_drop: false });
        assert_eq!(r.check("a", 6), Verdict::Allowed { resumed: 2 });
        assert!(!allowed(r.check("a", 6)));
        assert!(allowed(r.check("a", 12)));
    }

    #[test]
    fn a_quiet_source_refills_to_its_burst_only() {
        let mut r = RateLimit::new(0);
        for _ in 0..SOURCE_BURST {
            r.check("a", 0);
        }
        let later = 10_000;
        let mut passed = 0;
        while allowed(r.check("a", later)) {
            passed += 1;
        }
        assert_eq!(passed, SOURCE_BURST);
    }

    #[test]
    fn partial_intervals_are_not_lost() {
        let mut r = RateLimit::new(0);
        for _ in 0..SOURCE_BURST {
            r.check("a", 0);
        }
        // 4 s then 2 s more: the token earned at 6 s is there.
        assert!(!allowed(r.check("a", 4)));
        assert!(allowed(r.check("a", 6)));
    }

    #[test]
    fn one_noisy_source_does_not_block_another() {
        let mut r = RateLimit::new(0);
        while allowed(r.check("noisy", 0)) {}
        assert!(allowed(r.check("quiet", 0)));
    }

    #[test]
    fn inventing_sources_hits_the_shared_limit() {
        let mut r = RateLimit::new(0);
        let passed = (0..1000)
            .filter(|i| allowed(r.check(&format!("s{i}"), 0)))
            .count();
        assert_eq!(passed, ALL_BURST as usize);
        // The shared bucket refills like any other.
        assert!(allowed(r.check("s0", ALL_EVERY)));
    }

    #[test]
    fn the_source_map_is_bounded() {
        let mut r = RateLimit::new(0);
        // A flood of invented sources, one per second for an hour.
        for t in 0..3600u64 {
            r.check(&format!("s{t}"), t);
            assert!(r.sources() <= MAX_SOURCES, "at {t}: {}", r.sources());
        }
        // Real sources still get through once the shared bucket has refilled.
        assert!(allowed(
            r.check("frigate", 3600 + ALL_EVERY * u64::from(ALL_BURST))
        ));
    }

    #[test]
    fn a_clock_going_back_never_panics_or_mints_tokens() {
        let mut r = RateLimit::new(100);
        for _ in 0..SOURCE_BURST {
            r.check("a", 100);
        }
        assert!(!allowed(r.check("a", 0)));
        assert!(!allowed(r.check("a", 50)));
    }
}
