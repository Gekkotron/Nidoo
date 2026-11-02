//! Heartbeat file: tells the next boot whether this run ended on purpose.

use std::io;
use std::path::Path;

use nidoo_api::StopKind;
use serde::{Deserialize, Serialize};

use crate::setup::fs::write_private_atomic;

/// File name inside the data directory.
pub const FILE: &str = "system-heartbeat.json";

/// What the file holds: the last sign of life and, on a clean stop, when.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heartbeat {
    pub heartbeat: u64,
    pub clean_stop: Option<u64>,
}

/// What the previous run left behind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Previous {
    None,
    Corrupt,
    Found(Heartbeat),
}

/// Read the previous run's file. Missing is `None`; unreadable or invalid is `Corrupt`.
pub fn read(dir: &Path) -> Previous {
    match std::fs::read(dir.join(FILE)) {
        Ok(bytes) => match serde_json::from_slice::<Heartbeat>(&bytes) {
            Ok(h) => Previous::Found(h),
            Err(_) => Previous::Corrupt,
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Previous::None,
        Err(_) => Previous::Corrupt,
    }
}

/// How the previous run ended and how long the hub was down (seconds, if known).
pub fn classify(prev: &Previous, boot: u64) -> (StopKind, Option<u64>) {
    match prev {
        Previous::None => (StopKind::First, Some(0)),
        Previous::Corrupt => (StopKind::Unexpected, None),
        Previous::Found(h) => match h.clean_stop {
            Some(t) => (StopKind::Clean, Some(boot.saturating_sub(t))),
            None => (StopKind::Unexpected, Some(boot.saturating_sub(h.heartbeat))),
        },
    }
}

fn write(dir: &Path, value: &Heartbeat) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    write_private_atomic(&dir.join(FILE), &bytes)
}

/// Record a sign of life (the run is still going).
pub fn write_heartbeat(dir: &Path, now: u64) -> io::Result<()> {
    write(
        dir,
        &Heartbeat {
            heartbeat: now,
            clean_stop: None,
        },
    )
}

/// Record a deliberate stop, keeping the previous heartbeat value.
pub fn write_clean_stop(dir: &Path, now: u64) -> io::Result<()> {
    let heartbeat = match read(dir) {
        Previous::Found(h) => h.heartbeat,
        _ => now,
    };
    write(
        dir,
        &Heartbeat {
            heartbeat,
            clean_stop: Some(now),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(heartbeat: u64, clean_stop: Option<u64>) -> Previous {
        Previous::Found(Heartbeat {
            heartbeat,
            clean_stop,
        })
    }

    #[test]
    fn first_boot_is_first() {
        assert_eq!(classify(&Previous::None, 500), (StopKind::First, Some(0)));
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(dir.path()), Previous::None);
    }

    #[test]
    fn clean_stop_counts_from_stop_time() {
        assert_eq!(
            classify(&found(90, Some(100)), 160),
            (StopKind::Clean, Some(60))
        );
    }

    #[test]
    fn missing_marker_is_unexpected() {
        assert_eq!(
            classify(&found(100, None), 400),
            (StopKind::Unexpected, Some(300))
        );
    }

    #[test]
    fn clock_backwards_saturates() {
        assert_eq!(
            classify(&found(500, None), 400),
            (StopKind::Unexpected, Some(0))
        );
    }

    #[test]
    fn corrupt_file_is_unexpected_unknown() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), b"{\"heart").unwrap();
        let prev = read(dir.path());
        assert_eq!(prev, Previous::Corrupt);
        assert_eq!(classify(&prev, 400), (StopKind::Unexpected, None));
    }

    #[test]
    fn heartbeat_then_clean_stop_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        write_heartbeat(dir.path(), 10).unwrap();
        assert_eq!(read(dir.path()), found(10, None));
        write_clean_stop(dir.path(), 20).unwrap();
        assert_eq!(read(dir.path()), found(10, Some(20)));
    }
}
