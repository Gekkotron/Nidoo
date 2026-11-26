//! Proposals and their approval (spec §6.4, §8.3): publishing a pending proposal with its push,
//! approving or rejecting it from the web, MCP or a push button, and the override writers.

use std::sync::Arc;

use nidoo_climate::doc::{
    Mode, Override, OverrideOrigin, Proposal, ProposalKind, Rejection, RoomEco, Rules, Setpoint,
    ZoneSettings, ZoneState, check_setpoint,
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

/// Drops from every heat block's `heated_by` the ids that are neither heaters of the zone nor
/// one of its climate `units`; a list emptied that way becomes `None` (the room default).
fn drop_unknown_heated_by(settings: &mut ZoneSettings, units: &[String]) {
    let heaters: Vec<String> = settings.heaters.iter().map(|h| h.entity.clone()).collect();
    let Some(heat) = settings.rules.heat.as_mut() else {
        return;
    };
    for block in &mut heat.blocks {
        if let Some(ids) = &mut block.heated_by {
            ids.retain(|id| heaters.contains(id) || units.contains(id));
            if ids.is_empty() {
                block.heated_by = None;
            }
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
            // A proposal's blocks inherit their `heated_by`; an id that has left the room since
            // would make every later save of the settings fail, so it is dropped here.
            let units = super::inputs::zone_entity_ids(self, zone, &settings);
            drop_unknown_heated_by(&mut settings, &units);
            settings
                .validate_heated_by(&units)
                .map_err(HubError::Rejected)?;
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
        // A boost (whole-zone temperature) means the room is wanted warm: it ends the room's
        // eco, so the schedule follows the boost instead of eco coming back.
        let boost = ov.entity.is_none() && matches!(ov.setpoint, Some(Setpoint::Temp(_)));
        let in_eco = self
            .climate_store()
            .zones
            .get(zone)
            .is_some_and(|docs| docs.eco.is_some());
        if boost && in_eco {
            self.publish_zone_doc(zone, ZoneDoc::Eco, None::<&RoomEco>)?;
            tracing::info!(zone, "boost: room eco ended");
        }
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
mod tests;
