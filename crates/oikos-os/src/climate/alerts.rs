//! The climate engine's push alerts (spec §9): safety actions, a stale reading and a failing
//! command. Raising and resolving are idempotent, so the passes can call them every time.

use oikos_climate::doc::{Decision, SafetyCause};
use serde_json::json;

use crate::hub::Hub;

/// How long a raised alert stays relevant, in seconds.
const TTL_S: u64 = 43_200;
/// The push source and channel of every climate alert.
const SOURCE: &str = "climate";

/// How long a safety cause must stay absent before its alert resolves, in seconds: a reading
/// hovering at a limit must not raise a push every time it crosses.
const RESOLVE_AFTER_S: u64 = 600;
/// A safety alert that resolved is not raised again for its zone and cause for this many
/// seconds; the safety action itself still runs.
const REPEAT_AFTER_S: u64 = 6 * 3600;

fn slug(cause: SafetyCause) -> &'static str {
    match cause {
        SafetyCause::Frost => "frost",
        SafetyCause::Overheat => "overheat",
        SafetyCause::Overcool => "overcool",
        SafetyCause::Stale => "stale",
    }
}

fn severity(cause: SafetyCause) -> &'static str {
    match cause {
        SafetyCause::Frost | SafetyCause::Overheat => "critical",
        SafetyCause::Overcool | SafetyCause::Stale => "warning",
    }
}

fn title(cause: SafetyCause) -> &'static str {
    match cause {
        SafetyCause::Frost => "Frost protection",
        SafetyCause::Overheat => "Overheat protection",
        SafetyCause::Overcool => "Overcool protection",
        SafetyCause::Stale => "No fresh temperature reading",
    }
}

const ALL_CAUSES: [SafetyCause; 4] = [
    SafetyCause::Frost,
    SafetyCause::Overheat,
    SafetyCause::Overcool,
    SafetyCause::Stale,
];

fn safety_id(zone: &str, cause: SafetyCause) -> String {
    format!("climate-safety-{zone}-{}", slug(cause))
}

pub(crate) fn command_id(entity: &str) -> String {
    format!("climate-command-{entity}")
}

/// Takes alert `id` as raised when the push history still holds it open: the raised set is
/// in memory only, and after a restart the retained history is what remembers it.
fn adopt(hub: &Hub, id: &str) {
    if hub.climate_runtime().active_alerts.contains(id) {
        return;
    }
    if hub.alerts().iter().any(|a| a.id == id && !a.resolved) {
        hub.climate_runtime().active_alerts.insert(id.to_owned());
    }
}

/// When the newest push history entry of alert `id` was cancelled, if it was and says when:
/// after a restart the in-memory resolve times are gone and the retained history is what
/// remembers them.
fn resolved_in_history(hub: &Hub, id: &str) -> Option<u64> {
    hub.alerts()
        .iter()
        .find(|a| a.id == id)
        .filter(|a| a.resolved)
        .and_then(|a| a.resolved_at)
}

/// Raises alert `id` unless it is already raised.
pub(crate) fn raise(hub: &Hub, id: &str, title: &str, body: &str, severity: &str) {
    adopt(hub, id);
    if hub.climate_runtime().active_alerts.contains(id) {
        return;
    }
    let payload = json!({
        "v": 1,
        "id": id,
        "title": title,
        "body": body,
        "channel": "climate",
        "severity": severity,
        "ttl": TTL_S,
    });
    match hub.notify(SOURCE, &payload) {
        Ok(_) => {
            hub.climate_runtime().active_alerts.insert(id.to_owned());
        }
        Err(rejected) => tracing::warn!(%id, reason = %rejected.reason, "climate alert refused"),
    }
}

/// Resolves alert `id` when it is raised.
pub(crate) fn resolve(hub: &Hub, id: &str) {
    adopt(hub, id);
    if !hub.climate_runtime().active_alerts.remove(id) {
        return;
    }
    let payload = json!({ "v": 1, "id": id, "cancel": true });
    if let Err(rejected) = hub.notify(SOURCE, &payload) {
        tracing::warn!(%id, reason = %rejected.reason, "climate alert cancel refused");
    }
}

/// Raises the alert of `decision`'s safety cause, unless the same alert resolved less than six
/// hours ago. The zone's other safety alerts resolve once their cause has been absent for ten
/// minutes.
pub(crate) fn sync_safety(
    hub: &Hub,
    zone: &str,
    zone_name: impl Fn() -> String,
    d: &Decision,
    now: u64,
) {
    for cause in ALL_CAUSES {
        let id = safety_id(zone, cause);
        adopt(hub, &id);
        if d.cause == Some(cause) {
            let known = {
                let mut rt = hub.climate_runtime();
                rt.absent_since.remove(&id);
                rt.safety_resolved_at.get(&id).copied()
            };
            let resolved_at = known.or_else(|| {
                let at = resolved_in_history(hub, &id)?;
                hub.climate_runtime()
                    .safety_resolved_at
                    .insert(id.clone(), at);
                Some(at)
            });
            if resolved_at.is_some_and(|at| now.saturating_sub(at) < REPEAT_AFTER_S) {
                continue;
            }
            let name = title(cause);
            raise(
                hub,
                &id,
                &format!("{name} in {}", zone_name()),
                &d.reason,
                severity(cause),
            );
            continue;
        }
        let due = {
            let mut rt = hub.climate_runtime();
            if !rt.active_alerts.contains(&id) {
                rt.absent_since.remove(&id);
                continue;
            }
            let since = *rt.absent_since.entry(id.clone()).or_insert(now);
            now.saturating_sub(since) >= RESOLVE_AFTER_S
        };
        if due {
            {
                let mut rt = hub.climate_runtime();
                rt.absent_since.remove(&id);
                rt.safety_resolved_at.insert(id.clone(), now);
            }
            resolve(hub, &id);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use oikos_climate::doc::Source;
    use oikos_core::command::Publish;
    use tokio::sync::mpsc;

    use super::*;
    use crate::clock;
    use crate::config::Config;

    const ID: &str = "climate-safety-living-frost";

    fn hub() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
        let mut cfg = Config::default();
        cfg.climate.enabled = true;
        let (tx, rx) = mpsc::channel(64);
        (Hub::new(&cfg, tx), rx)
    }

    fn frost() -> Decision {
        Decision {
            zone: "living".to_owned(),
            mode: None,
            setpoint: None,
            source: Source::Safety,
            cause: Some(SafetyCause::Frost),
            reason: "too cold".to_owned(),
            reading: None,
            reading_age_s: None,
            outdoor_mean: None,
            home: true,
            applied: false,
            at: 0,
            entities: Default::default(),
            eco_restore: Default::default(),
            adjust: None,
        }
    }

    /// Whether `sync_safety` pushes a frost alert for a hub restarted with the frost alert
    /// cancelled `ago` seconds ago in the retained history and nothing in memory.
    async fn raised_after_restart(ago: u64) -> bool {
        let (hub, mut rx) = hub();
        hub.on_connected();
        hub.on_suback();
        tokio::time::sleep(Duration::from_secs(5)).await;
        let now = clock::now();
        let history = json!([{
            "v": 1, "id": ID, "channel": "climate", "severity": "critical",
            "title": "Frost protection in Living", "body": "", "actions": [],
            "silent": false, "source": "climate", "sent_at": now - ago - 600,
            "cancel": true, "resolved_at": now - ago,
        }]);
        hub.ingest_message("oikos/push/history", history.to_string().as_bytes(), true);
        sync_safety(&hub, "living", || "Living".to_owned(), &frost(), now);
        while let Ok(p) = rx.try_recv() {
            if p.topic == "oikos/push/history" {
                hub.ingest_message(&p.topic, p.payload.as_bytes(), false);
            }
        }
        hub.alerts().iter().any(|a| a.id == ID && !a.resolved)
    }

    #[tokio::test(start_paused = true)]
    async fn a_safety_alert_resolved_an_hour_before_a_restart_is_not_raised_again() {
        assert!(!raised_after_restart(3_600).await);
    }

    #[tokio::test(start_paused = true)]
    async fn a_safety_alert_resolved_seven_hours_before_a_restart_is_raised_again() {
        assert!(raised_after_restart(7 * 3_600).await);
    }
}
