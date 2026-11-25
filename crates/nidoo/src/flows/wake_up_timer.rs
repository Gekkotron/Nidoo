//! Wall-clock bookkeeping for `trigger.wake_up`: a node armed for its next pre-ring.

use chrono::DateTime;
use chrono_tz::Tz;

/// A wake-up node armed for its next pre-ring.
pub(super) struct ArmedWakeUp {
    pub node: String,
    /// The instant the node fires at next: an alarm's ring minus `before_minutes`.
    pub at: DateTime<Tz>,
    /// What the instant was computed from; a different one means the node must be re-armed.
    pub fingerprint: String,
}
