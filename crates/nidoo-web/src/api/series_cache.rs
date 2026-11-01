//! The 24-hour series the cards draw, fetched once per entity every few minutes and never more
//! than a few at a time, so a page full of sensors neither floods the hub nor refetches on every
//! state echo (a card is rebuilt each time its entity changes).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use gloo_timers::future::TimeoutFuture;
use nidoo_api::HistoryRange;

use super::history::fetch_series;
use crate::components::now::unix_now;

/// How long a fetched series (or a failure) is reused, in seconds.
pub const CACHE_TTL_S: u64 = 300;
/// The most history requests in flight at once.
pub const MAX_IN_FLIGHT: usize = 4;

/// One series as `(time, mean)`; `None` when the hub has none to give.
pub type Series = Option<Vec<(i64, f64)>>;

/// What a caller asking for a series should do.
#[derive(Debug, PartialEq)]
pub(crate) enum Begin {
    /// Here it is, fresh enough.
    Cached(Series),
    /// Fetch it: a slot is yours, give it back with [`CacheState::finish`].
    Start,
    /// Someone is fetching it already: wait for the entry.
    Join,
    /// Every slot is busy: try again shortly.
    Wait,
}

/// The cache's bookkeeping, kept free of I/O so it can be tested.
#[derive(Default)]
pub(crate) struct CacheState {
    entries: HashMap<String, (u64, Series)>,
    loading: HashSet<String>,
    in_flight: usize,
}

impl CacheState {
    /// The entry for `id` if it is still fresh at `now`.
    pub(crate) fn peek(&self, id: &str, now: u64) -> Option<Series> {
        self.entries
            .get(id)
            .filter(|(at, _)| now < at + CACHE_TTL_S)
            .map(|(_, series)| series.clone())
    }

    pub(crate) fn begin(&mut self, id: &str, now: u64) -> Begin {
        if let Some(series) = self.peek(id, now) {
            return Begin::Cached(series);
        }
        if self.loading.contains(id) {
            return Begin::Join;
        }
        if self.in_flight >= MAX_IN_FLIGHT {
            return Begin::Wait;
        }
        self.in_flight += 1;
        self.loading.insert(id.to_owned());
        Begin::Start
    }

    pub(crate) fn finish(&mut self, id: &str, now: u64, series: Series) {
        self.entries.insert(id.to_owned(), (now, series));
        if self.loading.remove(id) {
            self.in_flight = self.in_flight.saturating_sub(1);
        }
    }
}

thread_local! {
    static CACHE: RefCell<CacheState> = RefCell::new(CacheState::default());
}

/// The series of `id` if one was fetched (or failed) recently, without asking the hub.
pub fn cached_day_series(id: &str) -> Option<Series> {
    CACHE.with(|c| c.borrow().peek(id, unix_now()))
}

/// The last 24 hours of `id` as `(time, mean)`, from the cache or the hub. `None` when the hub
/// has no history for it (no database, not recorded, an error, no points).
pub async fn day_series(id: &str) -> Series {
    loop {
        let begin = CACHE.with(|c| c.borrow_mut().begin(id, unix_now()));
        match begin {
            Begin::Cached(series) => return series,
            Begin::Start => {
                let series = fetch_series(id, HistoryRange::Day)
                    .await
                    .ok()
                    .map(|s| {
                        s.points
                            .into_iter()
                            .map(|(t, mean, _, _)| (t, mean))
                            .collect::<Vec<_>>()
                    })
                    .filter(|points| !points.is_empty());
                CACHE.with(|c| c.borrow_mut().finish(id, unix_now(), series.clone()));
                return series;
            }
            Begin::Join | Begin::Wait => TimeoutFuture::new(100).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Begin, CACHE_TTL_S, CacheState, MAX_IN_FLIGHT};

    #[test]
    fn cache_reuses_and_caps_in_flight() {
        let mut c = CacheState::default();
        for i in 0..MAX_IN_FLIGHT {
            assert_eq!(c.begin(&format!("s{i}"), 0), Begin::Start);
        }
        // A fifth entity waits for a free slot; a second caller for a loading one joins it.
        assert_eq!(c.begin("s9", 0), Begin::Wait);
        assert_eq!(c.begin("s0", 0), Begin::Join);
        c.finish("s0", 10, Some(vec![(1, 20.0)]));
        assert_eq!(c.peek("s0", 10), Some(Some(vec![(1, 20.0)])));
        assert_eq!(c.begin("s9", 11), Begin::Start);
        // Reused while fresh, failures included; fetched again once stale.
        assert_eq!(
            c.begin("s0", 10 + CACHE_TTL_S - 1),
            Begin::Cached(Some(vec![(1, 20.0)]))
        );
        c.finish("s1", 10, None);
        assert_eq!(c.begin("s1", 20), Begin::Cached(None));
        assert_eq!(c.begin("s0", 10 + CACHE_TTL_S), Begin::Start);
        assert_eq!(c.peek("unknown", 0), None);
    }
}
