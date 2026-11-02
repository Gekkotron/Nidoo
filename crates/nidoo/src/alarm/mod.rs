//! The native alarm's hub side: the retained documents, the published panel entity and the
//! settings API.

pub(crate) mod effects;
pub(crate) mod engine;
pub(crate) mod entity;
pub(crate) mod hub;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Mutex, MutexGuard};

use nidoo_alarm::doc::AlarmSettings;
use nidoo_alarm::machine::Runtime;

/// The id of the alarm panel entity (its discovery `unique_id`).
pub const ALARM_ENTITY_ID: &str = "nidoo_alarm_house";

/// What the hub keeps for the alarm. It exists whether or not the alarm is switched on: the
/// engine idles while the settings say disabled.
pub struct AlarmHub {
    pub(crate) prefix: String,
    pub(crate) settings: Mutex<AlarmSettings>,
    /// Why the last settings document was refused; the previous one stays in force.
    pub(crate) settings_invalid: Mutex<Option<String>>,
    pub(crate) runtime: Mutex<Runtime>,
    /// Set once the retained runtime document was read (or written by this hub).
    pub(crate) runtime_loaded: AtomicBool,
    /// Held for a whole engine pass, so two callers never drive the machine at once.
    pub(crate) pass: tokio::sync::Mutex<()>,
    /// Set once the engine has resumed: from then on the in-memory runtime is the truth and
    /// the hub's own retained `<p>/state` echo is ignored (an older echo must not overwrite it).
    pub(crate) engine_started: AtomicBool,
    /// Unix seconds and the monotonic instant they were read at: the engine's clock is this
    /// anchor plus the time elapsed since, so deadlines never depend on the wall clock moving.
    /// It is read again at every resume: on a host without a battery clock the wall clock is
    /// wrong at process start and right once the network time has arrived.
    pub(crate) anchor: Mutex<(u64, tokio::time::Instant)>,
    /// Makes the trigger alert ids unique within one clock second.
    pub(crate) alert_seq: AtomicU64,
    /// The watched sensors whose offline warning is open, so it is raised once per outage.
    pub(crate) offline_raised: Mutex<BTreeSet<String>>,
}

impl AlarmHub {
    pub(crate) fn new(prefix: &str) -> Self {
        Self {
            prefix: prefix.trim_end_matches('/').to_owned(),
            settings: Mutex::new(AlarmSettings::default()),
            settings_invalid: Mutex::new(None),
            runtime: Mutex::new(Runtime::default()),
            runtime_loaded: AtomicBool::new(false),
            pass: tokio::sync::Mutex::new(()),
            engine_started: AtomicBool::new(false),
            anchor: Mutex::new((crate::clock::now(), tokio::time::Instant::now())),
            alert_seq: AtomicU64::new(0),
            offline_raised: Mutex::new(BTreeSet::new()),
        }
    }
}

/// Locks `mutex`, carrying on with the last value if a holder panicked.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
