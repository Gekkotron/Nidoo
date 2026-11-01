//! Wall-clock time for state timestamps and snapshots.

use std::time::{SystemTime, UNIX_EPOCH};

/// Unix time in seconds; `0` if the clock is before the epoch.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Unix time in microseconds. Used to stamp history rows so two changes within a second
/// keep their order, falling back to `0` when the clock reads before the epoch.
pub fn now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as i64)
}

/// A random number for a random delay. std's `RandomState` is seeded from the OS and hashing
/// the clock into it gives a fresh value per call; good enough for picking a wait, no crate.
pub fn random_roll() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    hasher.finish()
}
