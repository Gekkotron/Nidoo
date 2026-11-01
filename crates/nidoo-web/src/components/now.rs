//! The clock the relative times read: one shared signal, in unix seconds.

use leptos::prelude::*;

/// Unix seconds, re-set every 60 s while the app runs in the browser.
#[derive(Clone, Copy)]
pub struct Now(pub RwSignal<u64>);

#[cfg(target_arch = "wasm32")]
pub(crate) fn unix_now() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Creates the clock. Off the browser it is read once and never ticks.
pub fn install_now() -> Now {
    let now = RwSignal::new(unix_now());
    #[cfg(target_arch = "wasm32")]
    gloo_timers::callback::Interval::new(60_000, move || now.set(unix_now())).forget();
    Now(now)
}
