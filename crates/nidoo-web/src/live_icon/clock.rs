//! One requestAnimationFrame loop for every live icon, and the phases it moves. A phase is kept
//! by key (`"<entity id>/<part>"`), outside any card, so a card rebuilt by a state update picks
//! up where the old one was, and a new fan speed changes the pace without a jump.

use std::collections::BTreeMap;

use leptos::prelude::Signal;

/// The longest step a frame may take, so a tab shown again after minutes does not jump.
pub const MAX_DT_MS: f64 = 100.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Track {
    phase: f64,
    period_ms: f64,
    users: u32,
}

/// The phases of every live icon part, by key. A part only moves while an icon uses it; its
/// phase is kept when the last user leaves, for the card that replaces it.
#[derive(Clone, Debug, Default)]
pub struct PhaseStore {
    tracks: BTreeMap<String, Track>,
}

impl PhaseStore {
    /// One more icon uses `key`, at `period_ms` per loop: the newest caller sets the pace,
    /// and any phase already reached is kept.
    pub fn acquire(&mut self, key: &str, period_ms: f64) {
        let track = self.tracks.entry(key.to_owned()).or_insert(Track {
            phase: 0.0,
            period_ms,
            users: 0,
        });
        track.period_ms = period_ms;
        track.users += 1;
    }

    /// One icon stops using `key`; its phase stays.
    pub fn release(&mut self, key: &str) {
        if let Some(track) = self.tracks.get_mut(key) {
            track.users = track.users.saturating_sub(1);
        }
    }

    /// Moves every part in use on by `dt_ms` (at most [`MAX_DT_MS`]).
    pub fn advance(&mut self, dt_ms: f64) {
        let dt = dt_ms.clamp(0.0, MAX_DT_MS);
        for track in self.tracks.values_mut().filter(|t| t.users > 0) {
            track.phase = (track.phase + dt / track.period_ms.max(1.0)).rem_euclid(1.0);
        }
    }

    /// Where `key` is in its loop, 0..1; 0 for a part never used.
    pub fn phase(&self, key: &str) -> f64 {
        self.tracks.get(key).map_or(0.0, |t| t.phase)
    }

    /// Whether any icon is using a part, so the loop has something to move.
    pub fn is_active(&self) -> bool {
        self.tracks.values().any(|t| t.users > 0)
    }
}

/// Whether live icons move: only in the browser, and only when the system does not ask for less
/// motion. Otherwise every icon shows its still pose.
pub fn animations_on() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        !crate::energy_dots::reduced_motion()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        false
    }
}

/// The phase of `key`, looping every `period_ms`, for the component being built: it uses the part
/// until the component is cleaned up and follows the shared frame loop. Off wasm it stays at 0.
pub fn use_phase(key: String, period_ms: f64) -> Signal<f64> {
    #[cfg(target_arch = "wasm32")]
    {
        live::use_phase(key, period_ms)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (key, period_ms);
        Signal::stored(0.0)
    }
}

#[cfg(target_arch = "wasm32")]
mod live {
    use std::cell::{Cell, RefCell};

    use leptos::prelude::*;

    use super::PhaseStore;

    thread_local! {
        static STORE: RefCell<PhaseStore> = RefCell::new(PhaseStore::default());
        /// Bumped once a frame; every phase signal reads it, so each icon redraws its parts.
        static FRAME: ArcRwSignal<u64> = ArcRwSignal::new(0);
        static RUNNING: Cell<bool> = const { Cell::new(false) };
    }

    pub fn use_phase(key: String, period_ms: f64) -> Signal<f64> {
        STORE.with_borrow_mut(|s| s.acquire(&key, period_ms));
        start();
        let leaving = key.clone();
        on_cleanup(move || STORE.with_borrow_mut(|s| s.release(&leaving)));
        let frame = FRAME.with(Clone::clone);
        Signal::derive(move || {
            frame.track();
            STORE.with_borrow(|s| s.phase(&key))
        })
    }

    /// Starts the loop unless it runs already.
    fn start() {
        if RUNNING.replace(true) {
            return;
        }
        let last = js_sys::Date::now();
        request_animation_frame(move || frame(last));
    }

    /// One frame: moves the parts in use, then asks for the next frame, or stops once no icon
    /// is left.
    fn frame(last: f64) {
        let now = js_sys::Date::now();
        let active = STORE.with_borrow_mut(|s| {
            s.advance(now - last);
            s.is_active()
        });
        if !active {
            RUNNING.set(false);
            return;
        }
        FRAME.with(|f| f.update(|n| *n = n.wrapping_add(1)));
        request_animation_frame(move || frame(now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_moves_by_dt_over_period() {
        let mut s = PhaseStore::default();
        s.acquire("a", 1000.0);
        s.advance(50.0);
        s.advance(50.0);
        assert!((s.phase("a") - 0.1).abs() < 1e-9);
    }

    #[test]
    fn a_new_period_carries_on_from_the_current_phase() {
        let mut s = PhaseStore::default();
        s.acquire("a", 1000.0);
        s.advance(100.0);
        s.advance(100.0);
        // A rebuild with a faster fan.
        s.acquire("a", 500.0);
        s.release("a");
        s.advance(50.0);
        assert!((s.phase("a") - 0.3).abs() < 1e-9);
    }

    #[test]
    fn store_keeps_phase_across_a_rebuild() {
        let mut s = PhaseStore::default();
        s.acquire("r", 1000.0);
        s.advance(100.0);
        s.advance(100.0);
        // The new card mounts before the old one cleans up.
        s.acquire("r", 1000.0);
        s.release("r");
        assert!((s.phase("r") - 0.2).abs() < 1e-9);
        assert!(s.is_active());
        s.advance(100.0);
        assert!((s.phase("r") - 0.3).abs() < 1e-9);
    }

    #[test]
    fn advance_clamps_long_frames() {
        let mut s = PhaseStore::default();
        s.acquire("a", 1000.0);
        s.advance(60_000.0);
        assert!((s.phase("a") - 0.1).abs() < 1e-9);
    }

    #[test]
    fn store_goes_idle_when_released() {
        let mut s = PhaseStore::default();
        s.acquire("a", 1000.0);
        s.release("a");
        s.release("a");
        assert!(!s.is_active());
        s.advance(50.0);
        assert_eq!(s.phase("a"), 0.0);
    }

    #[test]
    fn phases_wrap() {
        let mut s = PhaseStore::default();
        s.acquire("a", 100.0);
        s.advance(100.0);
        s.advance(30.0);
        assert!((s.phase("a") - 0.3).abs() < 1e-9);
    }

    #[test]
    fn animations_are_off_natively() {
        assert!(!animations_on());
    }
}
