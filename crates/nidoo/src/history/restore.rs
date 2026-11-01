//! Boot-time restore of recorded states: a sensor that does not retain its state stays
//! unknown after a hub restart until its next message, though the history holds its last
//! value. Once the broker's retained replay has settled, that value goes back into the
//! registry for every entity that has still not reported.

use std::sync::Arc;
use std::time::Duration;

use nidoo_api::ConnectionStatus;

use crate::hub::Hub;

/// How long after the link first comes up the retained replay is given to land. A restored
/// value followed by a different retained one would look like a real change to the flow
/// engine, so the restore waits for the burst to end.
const SETTLE_DELAY: Duration = Duration::from_secs(10);

/// Waits for the first `Online`, lets the retained replay settle, then restores once. It runs
/// once per process: after a reconnect the registry already holds its values. A history that
/// is off or unreachable is logged and skipped; boot never waits on it.
pub async fn run(hub: Arc<Hub>) {
    let mut connection = hub.connection();
    if connection
        .wait_for(|status| matches!(status, ConnectionStatus::Online))
        .await
        .is_err()
    {
        return;
    }
    tokio::time::sleep(SETTLE_DELAY).await;
    let Some(history) = hub.history() else {
        return;
    };
    let db = match history.db() {
        Ok(db) => db,
        Err(e) => {
            tracing::info!(error = %e, "recorded states not restored: no history database");
            return;
        }
    };
    match db.latest_states().await {
        Ok(values) => {
            let restored = hub.restore_recorded_states(values);
            tracing::info!(restored, "restored recorded states of silent entities");
        }
        Err(e) => tracing::warn!(error = %e, "recorded states not restored: history unreachable"),
    }
}
