//! Proposals and their approval (spec §6.4, §8.3): publishing a pending proposal with its push,
//! approving or rejecting it from the web, MCP or a push button, and the override writers.

use std::sync::Arc;

use nidoo_climate::doc::{
    Mode, Override, OverrideOrigin, Proposal, ProposalKind, Rejection, RoomEco, Rules, ZoneState,
    check_setpoint,
};
use nidoo_climate::topics::{self, ZoneDoc};
use nidoo_climate::view::LearnStatus;
use serde::Deserialize;
use serde_json::json;

use super::learning;
use crate::clock;
use crate::hub::{Hub, HubError};

/// The push alert of a proposal is `<ALERT_PREFIX><proposal id>`.
const ALERT_PREFIX: &str = "climate-proposal-";
/// A zone gets at most one proposal push in this many seconds.
const PUSH_EVERY_S: u64 = 24 * 3600;
/// How long a rejected proposal is remembered, in seconds.
const REJECT_FOR_S: u64 = 7 * 24 * 3600;
const SOURCE: &str = "climate";

fn alert_id(proposal_id: &str) -> String {
    format!("{ALERT_PREFIX}{proposal_id}")
}

/// Resolves the push of a proposal. Sent whether or not this run raised it: the alert may
/// have been raised before a restart, or the push history may not have loaded yet.
fn cancel_alert(hub: &Hub, proposal_id: &str) {
    let payload = json!({ "v": 1, "id": alert_id(proposal_id), "cancel": true });
    if let Err(rejected) = hub.notify(SOURCE, &payload) {
        tracing::warn!(reason = %rejected.reason, "climate proposal alert cancel refused");
    }
}

/// The zone a proposal alert is about: proposal ids are `p-<YYYYMMDD-HHMMSS>-<zone>`.
fn alert_zone(alert_id: &str) -> Option<&str> {
    let stamped = alert_id.strip_prefix(ALERT_PREFIX)?.strip_prefix("p-")?;
    stamped.get("YYYYMMDD-HHMMSS-".len()..)
}

/// When `zone`'s last proposal push was sent, as the push history holds it.
fn last_push_in_history(hub: &Hub, zone: &str) -> Option<u64> {
    hub.alerts()
        .iter()
        .filter(|a| alert_zone(&a.id) == Some(zone))
        .map(|a| a.sent_at)
        .max()
}

/// Publishes `proposal` retained as the zone's one pending proposal, and tells the user unless
/// they were told less than a day ago. A pending proposal it replaces has its alert resolved.
pub(crate) fn publish(hub: &Hub, zone: &str, proposal: Proposal, now: u64) {
    if !hub.climate_enabled() {
        return;
    }
    let pending = hub
        .climate_store()
        .zones
        .get(zone)
        .and_then(|docs| docs.proposal.clone());
    let replaced = pending
        .as_ref()
        .filter(|old| old.id != proposal.id || old.fingerprint != proposal.fingerprint)
        .map(|old| old.id.clone());
    hub.write_zone_doc(zone, ZoneDoc::Proposal, Some(&proposal));
    if let Some(old) = replaced {
        cancel_alert(hub, &old);
    }
    // After a restart nothing is remembered: the push history says when the user was told.
    let last = hub.climate_runtime().last_proposal_push.get(zone).copied();
    let due = last
        .or_else(|| last_push_in_history(hub, zone))
        .is_none_or(|at| now.saturating_sub(at) >= PUSH_EVERY_S);
    if !due {
        return;
    }
    let name = hub.zone_display_name(zone);
    let title = match proposal.kind {
        ProposalKind::Initial => format!("Climate: new rules for {name}"),
        ProposalKind::Change => format!("Climate: rule changes for {name}"),
    };
    let mut lines: Vec<&str> = proposal
        .changes
        .iter()
        .take(2)
        .map(String::as_str)
        .collect();
    lines.push("Review on the Climate screen.");
    let payload = json!({
        "v": 1,
        "id": alert_id(&proposal.id),
        "title": title,
        "body": lines.join("\n"),
        "channel": "climate",
        "severity": "info",
        "actions": [
            { "id": "approve", "label": "Approve" },
            { "id": "reject", "label": "Reject" },
        ],
    });
    match hub.notify(SOURCE, &payload) {
        Ok(_) => {
            hub.climate_runtime()
                .last_proposal_push
                .insert(zone.to_owned(), now);
        }
        Err(rejected) => {
            tracing::warn!(reason = %rejected.reason, "climate proposal alert refused");
        }
    }
}

/// A push response: the alert id and the button the user pressed.
#[derive(Deserialize)]
struct Reply {
    id: String,
    action: String,
}

impl Hub {
    /// Learns `zone` now and returns where the learner stands.
    pub async fn learn_climate_zone(self: &Arc<Self>, zone: &str) -> Result<LearnStatus, HubError> {
        self.require_climate()?;
        self.require_zone(zone)?;
        Ok(learning::learn_zone(self, zone, clock::now()).await)
    }

    /// Makes the pending proposal of `zone` its rules: `edited` (validated) when given, else the
    /// proposal's. A pending zone becomes active, and the zone is evaluated at once.
    pub async fn approve_climate_proposal(
        self: &Arc<Self>,
        zone: &str,
        edited: Option<Rules>,
    ) -> Result<(), HubError> {
        let climate = self.require_climate()?;
        let now = clock::now();
        {
            let _pass = climate.pass.lock().await;
            let proposal = self.pending_proposal(zone)?;
            let rules = edited.unwrap_or_else(|| proposal.rules.clone());
            rules.validate().map_err(HubError::Rejected)?;
            let mut settings = self.zone_settings(zone);
            settings.rules = rules;
            if settings.state == ZoneState::Pending {
                settings.state = ZoneState::Active;
            }
            settings.approved_at = Some(now);
            settings.approved_proposal = Some(proposal.id.clone());
            settings.validate().map_err(HubError::Rejected)?;
            self.publish_zone_doc(zone, ZoneDoc::Settings, Some(&settings))?;
            self.publish_zone_doc(zone, ZoneDoc::Proposal, None::<&Proposal>)?;
            cancel_alert(self, &proposal.id);
            self.climate_runtime().learn_status.remove(zone);
        }
        self.evaluate_climate_zone(zone, now).await;
        Ok(())
    }

    /// Drops the pending proposal of `zone` and remembers its rules for a week, so the same
    /// proposal is not made again.
    pub async fn reject_climate_proposal(self: &Arc<Self>, zone: &str) -> Result<(), HubError> {
        let climate = self.require_climate()?;
        let now = clock::now();
        let _pass = climate.pass.lock().await;
        let proposal = self.pending_proposal(zone)?;
        let mut settings = self.zone_settings(zone);
        settings.rejected.retain(|r| r.until > now);
        settings.rejected.push(Rejection {
            fingerprint: proposal.fingerprint.clone(),
            until: now + REJECT_FOR_S,
        });
        self.publish_zone_doc(zone, ZoneDoc::Settings, Some(&settings))?;
        self.publish_zone_doc(zone, ZoneDoc::Proposal, None::<&Proposal>)?;
        cancel_alert(self, &proposal.id);
        self.climate_runtime()
            .learn_status
            .insert(zone.to_owned(), LearnStatus::Rejected);
        Ok(())
    }

    /// Adds `ov` to the zone's overrides, replacing the one already held for the same device
    /// (or for the whole zone).
    pub async fn set_climate_override(
        self: &Arc<Self>,
        zone: &str,
        ov: Override,
    ) -> Result<(), HubError> {
        self.set_climate_override_inner(zone, ov, false).await
    }

    /// [`Self::set_climate_override`] that also drops the zone's per-device overrides, so the
    /// devices follow the new override: for a setpoint the assistant was asked for.
    pub async fn replace_climate_overrides(
        self: &Arc<Self>,
        zone: &str,
        ov: Override,
    ) -> Result<(), HubError> {
        self.set_climate_override_inner(zone, ov, true).await
    }

    async fn set_climate_override_inner(
        self: &Arc<Self>,
        zone: &str,
        ov: Override,
        drop_device_overrides: bool,
    ) -> Result<(), HubError> {
        self.require_climate()?;
        self.require_zone(zone)?;
        let now = clock::now();
        let settings = self.zone_settings(zone);
        // Only an active zone follows overrides: anywhere else it would be silently ignored.
        if settings.state != ZoneState::Active {
            return Err(HubError::Rejected("zone is not active".to_owned()));
        }
        if ov.until <= now {
            return Err(HubError::Rejected("the override already ended".to_owned()));
        }
        if let Some(entity) = &ov.entity
            && !super::inputs::zone_entity_ids(self, zone, &settings).contains(entity)
        {
            return Err(HubError::Rejected(format!(
                "`{entity}` is not a climate device of zone `{zone}`"
            )));
        }
        if let Some(setpoint) = ov.setpoint {
            // The same range check as a schedule's setpoints, named after the override's mode.
            let label = match ov.mode {
                Some(Mode::Heat) => "heat override",
                Some(Mode::Cool) => "cool override",
                None => "override",
            };
            check_setpoint(&setpoint).map_err(|e| HubError::Rejected(format!("{label}: {e}")))?;
        }
        let ov = Override {
            origin: OverrideOrigin::User,
            ..ov
        };
        self.update_overrides(zone, |list| {
            list.retain(|o| {
                o.until > now
                    && o.entity != ov.entity
                    && !(drop_device_overrides && o.entity.is_some())
            });
            list.push(ov);
        });
        Ok(())
    }

    /// Removes the zone's boost (its whole-zone override) and keeps the per-device holds.
    pub async fn clear_climate_boost(self: &Arc<Self>, zone: &str) -> Result<(), HubError> {
        self.require_climate()?;
        self.require_zone(zone)?;
        self.update_overrides(zone, |list| list.retain(|o| o.entity.is_some()));
        Ok(())
    }

    /// Removes every override of the zone, so the schedule applies again.
    pub async fn clear_climate_override(self: &Arc<Self>, zone: &str) -> Result<(), HubError> {
        self.require_climate()?;
        self.require_zone(zone)?;
        self.update_overrides(zone, Vec::clear);
        Ok(())
    }

    /// Holds the zone in eco until `eco.until`, replacing any eco it had.
    pub async fn set_climate_eco(
        self: &Arc<Self>,
        zone: &str,
        eco: RoomEco,
    ) -> Result<(), HubError> {
        self.require_climate()?;
        self.require_zone(zone)?;
        if self.zone_settings(zone).state == ZoneState::Off {
            return Err(HubError::Rejected("zone is off".to_owned()));
        }
        if eco.until <= clock::now() {
            return Err(HubError::Rejected("eco already ended".to_owned()));
        }
        self.publish_zone_doc(zone, ZoneDoc::Eco, Some(&eco))
    }

    /// Ends the zone's eco, so its schedule applies again.
    pub async fn clear_climate_eco(self: &Arc<Self>, zone: &str) -> Result<(), HubError> {
        self.require_climate()?;
        self.require_zone(zone)?;
        self.publish_zone_doc(zone, ZoneDoc::Eco, None::<&RoomEco>)
    }

    /// A push response on `<push prefix>/responses`. Only a button of a climate proposal that is
    /// still pending does anything; a stale or repeated one is ignored.
    pub(crate) fn on_climate_reply(self: &Arc<Self>, payload: &[u8]) {
        let Ok(reply) = serde_json::from_slice::<Reply>(payload) else {
            return;
        };
        let Some(proposal_id) = reply.id.strip_prefix(ALERT_PREFIX) else {
            return;
        };
        let zone = self
            .climate_store()
            .zones
            .iter()
            .find(|(_, docs)| docs.proposal.as_ref().is_some_and(|p| p.id == proposal_id))
            .map(|(zone, _)| zone.clone());
        let Some(zone) = zone else {
            tracing::debug!(id = %reply.id, "climate reply for a proposal that is not pending");
            return;
        };
        let approve = match reply.action.as_str() {
            "approve" => true,
            "reject" => false,
            _ => return,
        };
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let hub = Arc::clone(self);
        tokio::spawn(async move {
            let done = if approve {
                hub.approve_climate_proposal(&zone, None).await
            } else {
                hub.reject_climate_proposal(&zone).await
            };
            if let Err(err) = done {
                tracing::info!(%zone, error = %err, "climate proposal reply not applied");
            }
        });
    }

    fn pending_proposal(&self, zone: &str) -> Result<Proposal, HubError> {
        self.climate_store()
            .zones
            .get(zone)
            .and_then(|docs| docs.proposal.clone())
            .ok_or_else(|| HubError::Rejected("no pending proposal".to_owned()))
    }

    /// `zone` is a zone the engine knows: an area with climate devices or one with documents.
    fn require_zone(&self, zone: &str) -> Result<(), HubError> {
        if self.climate_zone_ids().iter().any(|z| z == zone) {
            Ok(())
        } else {
            Err(HubError::UnknownArea(zone.to_owned()))
        }
    }

    /// Publishes a zone document retained, failing when the broker link is down: an approval
    /// must not look done while nothing was sent.
    fn publish_zone_doc<T: serde::Serialize>(
        &self,
        zone: &str,
        doc: ZoneDoc,
        value: Option<&T>,
    ) -> Result<(), HubError> {
        let prefix = self.require_climate()?.prefix.clone();
        self.publish_climate(topics::zone_topic(&prefix, zone, doc), value)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nidoo_climate::doc::{Window, ZoneSettings};
    use nidoo_core::command::Publish;
    use tokio::sync::mpsc;

    use super::*;
    use crate::config::Config;

    fn hub() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
        let mut cfg = Config::default();
        cfg.climate.enabled = true;
        let (tx, rx) = mpsc::channel(64);
        (Hub::new(&cfg, tx), rx)
    }

    fn proposal(id: &str, kind: ProposalKind, changes: &[&str]) -> Proposal {
        Proposal {
            id: id.to_owned(),
            zone: "living".to_owned(),
            kind,
            created: 0,
            window: Window {
                from: 0,
                to: 1,
                days: 28,
            },
            rules: Rules::default(),
            changes: changes.iter().map(|c| (*c).to_owned()).collect(),
            fingerprint: id.to_owned(),
        }
    }

    /// Lets the push service accept alerts and returns the alerts as the hub sees them after
    /// the broker echoes the history back.
    async fn settle(hub: &Hub) {
        hub.on_connected();
        hub.on_suback();
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    fn echo(hub: &Hub, rx: &mut mpsc::Receiver<Publish>) {
        while let Ok(p) = rx.try_recv() {
            if p.topic == hub.push().history_topic() {
                hub.ingest_message(&p.topic, p.payload.as_bytes(), false);
            }
        }
    }

    fn alerts_of(hub: &Hub) -> Vec<nidoo_api::AlertView> {
        hub.alerts()
            .into_iter()
            .filter(|a| a.id.starts_with(ALERT_PREFIX))
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn proposal_push_has_buttons_and_daily_limit() {
        let (hub, mut rx) = hub();
        settle(&hub).await;
        let t = 1_000_000;

        let first = proposal(
            "p-1",
            ProposalKind::Initial,
            &["Base: 17.0 °C", "New: a", "New: b"],
        );
        publish(&hub, "living", first.clone(), t);
        echo(&hub, &mut rx);
        let alerts = alerts_of(&hub);
        assert_eq!(alerts.len(), 1, "{alerts:?}");
        let alert = &alerts[0];
        assert_eq!(alert.id, "climate-proposal-p-1");
        assert_eq!(alert.title, "Climate: new rules for living");
        assert_eq!(
            alert.body, "Base: 17.0 °C\nNew: a\nReview on the Climate screen.",
            "the first two changes, then the pointer to the screen"
        );
        let buttons: Vec<(&str, &str)> = alert
            .actions
            .iter()
            .map(|a| (a.id.as_str(), a.label.as_str()))
            .collect();
        assert_eq!(buttons, [("approve", "Approve"), ("reject", "Reject")]);
        assert_eq!(serde_json::to_value(alert.severity).unwrap(), "info");
        assert!(!alert.resolved);

        // The same proposal again, and then a newer one, within a day: the document is
        // replaced, nobody is pinged, and the old alert is resolved by the replacement.
        publish(&hub, "living", first, t + 600);
        let second = proposal(
            "p-2",
            ProposalKind::Change,
            &["Changed: weekdays start 07:00 → 07:30"],
        );
        publish(&hub, "living", second.clone(), t + 3_600);
        echo(&hub, &mut rx);
        let alerts = alerts_of(&hub);
        assert_eq!(alerts.len(), 1, "one push per zone per day: {alerts:?}");
        assert!(
            alerts[0].resolved,
            "the replaced proposal's alert is resolved"
        );
        assert_eq!(
            hub.climate_store().zones["living"]
                .proposal
                .as_ref()
                .map(|p| p.id.as_str()),
            Some("p-2")
        );

        // A day after the first push, a new proposal pings again.
        let third = proposal(
            "p-3",
            ProposalKind::Change,
            &["Changed: weekdays end 22:00 → 22:30"],
        );
        publish(&hub, "living", third, t + PUSH_EVERY_S);
        echo(&hub, &mut rx);
        let alerts = alerts_of(&hub);
        let fresh = alerts
            .iter()
            .find(|a| a.id == "climate-proposal-p-3")
            .unwrap();
        assert_eq!(fresh.title, "Climate: rule changes for living");
        assert!(!fresh.resolved);
    }

    #[tokio::test]
    async fn learn_status_from_the_run_reaches_the_view() {
        let (hub, _rx) = hub();
        let (history, _in) = crate::history::History::detached(8, chrono_tz::UTC);
        hub.set_history(history);
        hub.ingest_message(
            "nidoo/climate/zone/attic/settings",
            serde_json::to_string(&ZoneSettings {
                state: nidoo_climate::doc::ZoneState::Active,
                ..ZoneSettings::default()
            })
            .unwrap()
            .as_bytes(),
            true,
        );
        let learn = |hub: &Hub| hub.climate_view().unwrap().zones[0].learn.clone();
        assert_eq!(learn(&hub), LearnStatus::UpToDate);
        for status in [
            LearnStatus::NotEnough {
                days: 6,
                needed: 10,
            },
            LearnStatus::Rejected,
        ] {
            hub.climate_runtime()
                .learn_status
                .insert("attic".to_owned(), status.clone());
            assert_eq!(learn(&hub), status);
        }
        // A pending proposal wins over whatever the last run said.
        hub.ingest_message(
            "nidoo/climate/zone/attic/proposal",
            serde_json::to_string(&proposal("p-1", ProposalKind::Initial, &[]))
                .unwrap()
                .as_bytes(),
            true,
        );
        assert_eq!(learn(&hub), LearnStatus::Proposed);
    }

    #[tokio::test]
    async fn a_zone_in_eco_is_not_learned() {
        use nidoo_climate::doc::{RoomEco, ZoneState};
        let (hub, _rx) = hub();
        let (history, _in) = crate::history::History::detached(8, chrono_tz::UTC);
        hub.set_history(history);
        hub.ingest_message(
            "nidoo/climate/zone/attic/settings",
            settings_doc(ZoneState::Active).as_bytes(),
            true,
        );
        let now = clock::now();
        let eco = |until| {
            serde_json::to_string(&RoomEco {
                set_at: now - 60,
                until,
            })
            .unwrap()
        };
        let topic = "nidoo/climate/zone/attic/eco";
        hub.ingest_message(topic, eco(now + 3_600).as_bytes(), true);
        let learn = |hub: &Hub| hub.climate_view().unwrap().zones[0].learn.clone();
        assert_eq!(learn(&hub), LearnStatus::Eco);
        assert_eq!(
            learning::learn_zone(&hub, "attic", now).await,
            LearnStatus::Eco
        );
        assert_eq!(
            hub.climate_runtime().learn_status["attic"],
            LearnStatus::Eco
        );
        // An eco that has ended no longer holds learning back.
        hub.ingest_message(topic, eco(now - 1).as_bytes(), true);
        assert_eq!(learn(&hub), LearnStatus::UpToDate);
        assert_ne!(
            learning::learn_zone(&hub, "attic", now).await,
            LearnStatus::Eco
        );
    }

    #[tokio::test(start_paused = true)]
    async fn proposal_reply_follows_push_prefix() {
        let mut cfg = Config::default();
        cfg.climate.enabled = true;
        cfg.push.prefix = "custom/push".to_owned();
        let (tx, mut rx) = mpsc::channel(64);
        let hub = Hub::new(&cfg, tx);
        settle(&hub).await;
        publish(
            &hub,
            "living",
            proposal("p-1", ProposalKind::Initial, &["Base: 17.0 °C"]),
            1_000_000,
        );
        echo(&hub, &mut rx);
        assert_eq!(
            alerts_of(&hub).len(),
            1,
            "the history on custom/push came back"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_fingerprint_under_the_same_id_replaces_the_alert() {
        let (hub, mut rx) = hub();
        settle(&hub).await;
        let first = proposal("p-1", ProposalKind::Initial, &["Base: 17.0 °C"]);
        publish(&hub, "living", first.clone(), 1_000_000);
        let mut again = first;
        again.fingerprint = "other".to_owned();
        publish(&hub, "living", again, 1_000_010);
        echo(&hub, &mut rx);
        let alerts = alerts_of(&hub);
        assert_eq!(alerts.len(), 1);
        assert!(alerts[0].resolved, "the old rules' alert was cancelled");
    }

    /// The retained push history of a hub that pushed `alert_id` at `sent_at`, delivered as
    /// the broker does after a restart.
    fn ingest_pushed(hub: &Hub, alert_id: &str, sent_at: u64) {
        let history = json!([{
            "v": 1, "id": alert_id, "channel": "climate", "severity": "info",
            "title": "Climate: new rules for living", "body": "", "actions": [],
            "silent": false, "source": "climate", "sent_at": sent_at,
        }]);
        hub.ingest_message("nidoo/push/history", history.to_string().as_bytes(), true);
    }

    /// A pending proposal, as the broker delivers it after a restart.
    fn ingest_pending(hub: &Hub, id: &str, created: u64) {
        let mut old = proposal(id, ProposalKind::Initial, &[]);
        old.created = created;
        hub.ingest_message(
            "nidoo/climate/zone/living/proposal",
            serde_json::to_string(&old).unwrap().as_bytes(),
            true,
        );
    }

    #[test]
    fn a_proposal_alert_names_its_zone() {
        assert_eq!(
            alert_zone("climate-proposal-p-20261029-080000-living"),
            Some("living")
        );
        assert_eq!(
            alert_zone("climate-proposal-p-20261029-080000-x-living"),
            Some("x-living")
        );
        assert_eq!(alert_zone("climate-proposal-p-1"), None);
        assert_eq!(alert_zone("climate-safety-living-frost"), None);
    }

    #[tokio::test(start_paused = true)]
    async fn the_daily_limit_survives_a_restart() {
        let (hub, mut rx) = hub();
        let now = clock::now();
        // A proposal was pushed an hour ago, before the restart; a later one replaced it
        // without a push. Only the push history remembers when the user was told.
        ingest_pushed(
            &hub,
            "climate-proposal-p-20261029-080000-living",
            now - 3_600,
        );
        settle(&hub).await;
        ingest_pending(&hub, "p-20261029-084000-living", now - 1_200);
        publish(
            &hub,
            "living",
            proposal("p-new", ProposalKind::Change, &[]),
            now,
        );
        echo(&hub, &mut rx);
        assert!(
            alerts_of(&hub)
                .iter()
                .all(|a| a.id != "climate-proposal-p-new"),
            "no second push within a day"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_pending_proposal_never_pushed_does_not_hold_the_push() {
        let (hub, mut rx) = hub();
        let now = clock::now();
        // Another zone's push and a pending proposal of this zone that was never pushed.
        ingest_pushed(
            &hub,
            "climate-proposal-p-20261029-080000-bedroom",
            now - 3_600,
        );
        settle(&hub).await;
        ingest_pending(&hub, "p-20261029-084000-living", now - 1_200);
        publish(
            &hub,
            "living",
            proposal("p-new", ProposalKind::Change, &[]),
            now,
        );
        echo(&hub, &mut rx);
        assert!(
            alerts_of(&hub)
                .iter()
                .any(|a| a.id == "climate-proposal-p-new"),
            "pushed: {:?}",
            alerts_of(&hub)
        );
    }

    #[tokio::test]
    async fn history_off_runs_clear_a_stale_status() {
        let (hub, _rx) = hub();
        hub.climate_runtime()
            .learn_status
            .insert("living".to_owned(), LearnStatus::Rejected);
        // No history configured.
        assert_eq!(
            learning::learn_zone(&hub, "living", 1_000).await,
            LearnStatus::HistoryOff
        );
        assert_eq!(
            hub.climate_runtime().learn_status["living"],
            LearnStatus::HistoryOff
        );
        // Configured but unreachable.
        let (history, _in) = crate::history::History::detached(8, chrono_tz::UTC);
        hub.set_history(history);
        hub.climate_runtime().learn_status.insert(
            "living".to_owned(),
            LearnStatus::NotEnough {
                days: 1,
                needed: 10,
            },
        );
        assert_eq!(
            learning::learn_zone(&hub, "living", 1_000).await,
            LearnStatus::HistoryOff
        );
        assert_eq!(
            hub.climate_runtime().learn_status["living"],
            LearnStatus::HistoryOff
        );
    }

    fn settings_doc(state: nidoo_climate::doc::ZoneState) -> String {
        serde_json::to_string(&ZoneSettings {
            state,
            ..ZoneSettings::default()
        })
        .unwrap()
    }

    #[tokio::test]
    async fn a_zone_leaving_off_is_learned_at_once() {
        use nidoo_climate::doc::ZoneState;
        let (hub, _rx) = hub();
        let topic = "nidoo/climate/zone/attic/settings";
        hub.ingest_message(topic, settings_doc(ZoneState::Off).as_bytes(), true);
        hub.climate_runtime()
            .learn_status
            .insert("attic".to_owned(), LearnStatus::Rejected);
        // Still off: nothing runs.
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            hub.climate_runtime().learn_status["attic"],
            LearnStatus::Rejected
        );
        hub.ingest_message(topic, settings_doc(ZoneState::Pending).as_bytes(), true);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            hub.climate_runtime().learn_status["attic"],
            LearnStatus::HistoryOff,
            "the run replaced the stale status"
        );
        assert!(hub.climate_runtime().learning_now.is_empty());
    }

    #[tokio::test]
    async fn a_zone_already_being_learned_is_skipped() {
        let (hub, _rx) = hub();
        hub.climate_runtime()
            .learning_now
            .insert("attic".to_owned());
        learning::spawn_zone_learning(&hub, "attic");
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!hub.climate_runtime().learn_status.contains_key("attic"));
        assert!(
            hub.climate_runtime().learning_now.contains("attic"),
            "not ours to clear"
        );
        hub.climate_runtime().learning_now.clear();
        learning::spawn_zone_learning(&hub, "attic");
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(hub.climate_runtime().learn_status.contains_key("attic"));
        assert!(
            hub.climate_runtime().learning_now.is_empty(),
            "the guard clears it"
        );
    }
}
