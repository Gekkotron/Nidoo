//! System health: machine figures, restart tracking and service status.

pub mod heartbeat;
pub mod restarts;
pub mod sample;

use std::sync::atomic::Ordering;

use crate::hub::{Hub, HubError};
use crate::setup::store;

impl Hub {
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
        // The row is saved; a cache failure is only logged, the cache heals on the next write.
        if let Err(error) = store::write_cache(&ctx.data_dir, &next) {
            tracing::warn!(%error, "could not refresh the settings cache");
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
