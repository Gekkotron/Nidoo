//! System health: machine figures, restart tracking and service status.

pub mod discovery;
pub mod heartbeat;
pub mod restarts;
pub mod sample;
pub mod services;
pub mod task;

use std::sync::atomic::Ordering;

use chrono::NaiveDate;

use crate::hub::{Hub, HubError};
use crate::setup::store;

/// A per-day view of a monotonic command counter: it reads 0 at each new local day.
#[derive(Debug, Default)]
pub struct CommandsToday {
    day: Option<NaiveDate>,
    baseline: u64,
}

impl CommandsToday {
    /// Commands since the start of `today`, given the running `total`. The first call
    /// counts from boot; a date change takes `total` as the new baseline.
    pub fn count(&mut self, total: u64, today: NaiveDate) -> u64 {
        if self.day != Some(today) {
            if self.day.is_some() {
                self.baseline = total;
            }
            self.day = Some(today);
        }
        total.saturating_sub(self.baseline)
    }
}

/// How long the broker's retained-message count stays valid without a new report. Mosquitto
/// sends one about every ten seconds, so three minutes of silence means it stopped telling.
pub const RETAINED_FRESH_SECS: u64 = 180;

/// The broker's last retained-message report: the count, when it arrived and the
/// [`Hub::link_epoch`] it arrived in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetainedReport {
    count: u64,
    received_at: u64,
    epoch: u64,
}

impl RetainedReport {
    /// The count while it is fresh at `now` and from the current link, else `None`.
    fn fresh(&self, now: u64, epoch: u64) -> Option<u64> {
        (self.epoch == epoch && now.saturating_sub(self.received_at) <= RETAINED_FRESH_SECS)
            .then_some(self.count)
    }
}

impl Hub {
    /// Retained messages on the broker; `None` until the broker has reported a count, when
    /// the last report is older than [`RETAINED_FRESH_SECS`], or after a link drop until the
    /// next report.
    pub fn retained_count(&self) -> Option<u64> {
        self.retained_count_at(crate::clock::now())
    }

    /// [`Hub::retained_count`] at a given time.
    pub fn retained_count_at(&self, now: u64) -> Option<u64> {
        let report = *self
            .system_retained
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        report?.fresh(now, self.link_epoch())
    }

    /// Capture the broker's retained-message count; an unparsable payload is ignored.
    pub(crate) fn ingest_system_retained(&self, topic: &str, payload: &[u8]) -> bool {
        if topic != services::RETAINED_TOPIC {
            return false;
        }
        if let Some(count) = std::str::from_utf8(payload)
            .ok()
            .and_then(|text| text.trim().parse::<u64>().ok())
        {
            let report = RetainedReport {
                count,
                received_at: crate::clock::now(),
                epoch: self.link_epoch(),
            };
            *self
                .system_retained
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(report);
        }
        true
    }

    /// Device commands published since boot.
    pub fn commands_sent(&self) -> u64 {
        self.commands_sent.load(Ordering::Relaxed)
    }

    /// Record a deliberate stop so the next boot counts it as a clean restart.
    /// No-op without a setup context; errors are logged and swallowed.
    pub fn mark_clean_stop(&self) {
        let Some(ctx) = self.setup.get() else {
            return;
        };
        if let Err(e) = heartbeat::write_clean_stop(&ctx.data_dir, crate::clock::now()) {
            tracing::warn!(error = %e, "could not write the clean-stop marker");
        }
    }

    /// Whether the "notify me" switch is on.
    pub fn system_notify(&self) -> bool {
        self.system_notify.load(Ordering::Relaxed)
    }

    /// Turns the "notify me" switch on or off and stores it, without restarting the hub.
    /// The database row is written first: a failed save (or no database) is a `Publish`
    /// error, mapped to 503, and leaves the settings, the cache file and the flag as they were.
    pub async fn set_system_notify(&self, notify: bool) -> Result<bool, HubError> {
        let unavailable = |why: &str| HubError::Publish(format!("settings unavailable: {why}"));
        let ctx = self
            .setup
            .get()
            .ok_or_else(|| unavailable("no setup context"))?;
        let db = ctx.db.as_ref().ok_or_else(|| unavailable("no database"))?;
        let mut held = ctx.sections.lock().await;
        let mut next = held.clone();
        next.system.notify = notify;
        if let Err(error) = store::save_section(db, "system", &next).await {
            tracing::warn!(%error, "could not store the system notify switch");
            return Err(unavailable("the database did not take the write"));
        }
        if let Err(error) = store::write_cache(&ctx.data_dir, &next) {
            // Boot reads the cache while the database is down, so a switch the cache lacks
            // could revert silently: put the row back and report the failure.
            tracing::error!(error = %error.kind(), "could not refresh the settings cache");
            if let Err(error) = store::save_section(db, "system", &held).await {
                tracing::warn!(%error, "could not restore the system row after a cache failure");
            }
            return Err(unavailable("the settings cache could not be written"));
        }
        *held = next;
        self.system_notify.store(notify, Ordering::Relaxed);
        Ok(notify)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::config::Config;
    use crate::history::db::Db;
    use crate::setup::sections::Sections;
    use crate::setup::{HubFile, Mode, RestartHook, SetupCtx};

    #[test]
    fn commands_today_resets_at_midnight() {
        let d1 = NaiveDate::from_ymd_opt(2026, 11, 2).expect("date");
        let d2 = NaiveDate::from_ymd_opt(2026, 11, 3).expect("date");
        let mut today = CommandsToday::default();
        assert_eq!(today.count(5, d1), 5);
        assert_eq!(today.count(8, d1), 8);
        assert_eq!(today.count(9, d2), 0);
        assert_eq!(today.count(12, d2), 3);
    }

    fn hub_with(db: Option<Db>, dir: &std::path::Path) -> (Arc<Hub>, Arc<SetupCtx>) {
        let ctx = Arc::new(SetupCtx::new(
            dir.to_path_buf(),
            Mode::Run,
            HubFile::default(),
            db,
            Sections::default(),
            RestartHook::Record(Arc::default()),
        ));
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let hub = Hub::new(&Config::default(), tx);
        hub.set_setup(Arc::clone(&ctx));
        (hub, ctx)
    }

    #[test]
    fn retained_count_goes_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let (hub, _ctx) = hub_with(None, tmp.path());
        let at = crate::clock::now();
        assert!(hub.ingest_system_retained(services::RETAINED_TOPIC, b"2431"));
        assert_eq!(hub.retained_count_at(at), Some(2431));
        assert_eq!(hub.retained_count_at(at + RETAINED_FRESH_SECS), Some(2431));
        // Mosquitto reports every ten seconds: three minutes of silence is a broker that
        // stopped telling, and the count is unknown.
        assert_eq!(hub.retained_count_at(at + RETAINED_FRESH_SECS + 60), None);
    }

    #[test]
    fn retained_count_is_unknown_after_a_link_drop_until_reported_again() {
        let tmp = tempfile::tempdir().unwrap();
        let (hub, _ctx) = hub_with(None, tmp.path());
        hub.set_connection(nidoo_api::ConnectionStatus::Online);
        hub.ingest_system_retained(services::RETAINED_TOPIC, b"2431");
        assert_eq!(hub.retained_count(), Some(2431));
        hub.set_connection(nidoo_api::ConnectionStatus::Offline {
            reason: "gone".into(),
        });
        hub.set_connection(nidoo_api::ConnectionStatus::Online);
        assert_eq!(hub.retained_count(), None);
        hub.ingest_system_retained(services::RETAINED_TOPIC, b"2440");
        assert_eq!(hub.retained_count(), Some(2440));
    }

    #[tokio::test]
    async fn set_system_notify_db_down_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let before = Sections::default();
        store::write_cache(tmp.path(), &before).unwrap();
        let cache = tmp.path().join(store::CACHE_FILE);
        let cache_before = std::fs::read(&cache).unwrap();
        let db = Db::connect_lazy("postgres://u:p@127.0.0.1:1/x").unwrap();
        let (hub, ctx) = hub_with(Some(db), tmp.path());

        let err = hub.set_system_notify(true).await.unwrap_err();
        // `Publish` is the variant the HTTP layer answers with 503.
        assert!(matches!(err, HubError::Publish(_)), "{err:?}");
        assert!(!hub.system_notify());
        assert!(!ctx.sections.lock().await.system.notify);
        assert_eq!(std::fs::read(&cache).unwrap(), cache_before);
        assert!(!ctx.restarting.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn set_system_notify_without_a_database_is_unavailable() {
        let tmp = tempfile::tempdir().unwrap();
        let (hub, _ctx) = hub_with(None, tmp.path());
        let err = hub.set_system_notify(true).await.unwrap_err();
        // `Publish` is the variant the HTTP layer answers with 503.
        assert!(matches!(err, HubError::Publish(_)), "{err:?}");
        assert!(!hub.system_notify());
        assert!(!tmp.path().join(store::CACHE_FILE).exists());
    }
}
