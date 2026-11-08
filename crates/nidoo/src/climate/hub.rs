//! The climate side of the hub: the read view and the writers behind the REST and MCP layers.

use std::collections::{BTreeMap, VecDeque};
use std::sync::MutexGuard;

use nidoo_api::WsEvent;
use nidoo_climate::doc::{Decision, HouseSettings, ZoneSettings, ZoneState};
use nidoo_climate::topics::{self, ClimateTopic, ZoneDoc};
use nidoo_climate::view::{
    ClimateView, DecisionLog, DecisionRow, LearnStatus, PresenceView, ZoneView,
};
use nidoo_core::command::Publish;
use nidoo_core::model::{Kind, UNASSIGNED_AREA};
use serde::Serialize;

use super::inputs;
use super::store::{Applied, ClimateStore, ZoneDocs};
use super::{ClimateRuntime, GROUP_PREFIX};
use crate::clock;
use crate::hub::{Hub, HubError};

impl Hub {
    /// Whether `[climate] enabled` is on.
    pub fn climate_enabled(&self) -> bool {
        self.climate.is_some()
    }

    /// The topic prefix of the climate documents, `None` while the engine is off.
    pub(crate) fn climate_prefix(&self) -> Option<&str> {
        self.climate.as_ref().map(|c| c.prefix.as_str())
    }

    /// The retained documents. Callers have checked [`Hub::climate_enabled`] (or went through
    /// `require_climate`); the store only exists while the engine is on.
    pub(crate) fn climate_store(&self) -> MutexGuard<'_, ClimateStore> {
        self.climate
            .as_ref()
            .expect("climate_store is only used while [climate] is enabled")
            .store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The engine's runtime state; same rules as [`Hub::climate_store`], and never held across
    /// an `await`.
    pub(crate) fn climate_runtime(&self) -> MutexGuard<'_, ClimateRuntime> {
        self.climate
            .as_ref()
            .expect("climate_runtime is only used while [climate] is enabled")
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The whole engine as one document: the settings, and one entry per zone.
    pub fn climate_view(&self) -> Result<ClimateView, HubError> {
        self.require_climate()?;
        let areas = self.climate_areas();
        let history = self.history_configured();
        let now = clock::now();
        let claims = self.climate_claims();
        let (seen, learn) = {
            let rt = self.climate_runtime();
            (rt.seen_at.clone(), rt.learn_status.clone())
        };
        let mut view = {
            let store = self.climate_store();
            let mut zones: Vec<ZoneView> = areas
                .iter()
                .map(|(zone, (name, devices))| {
                    let docs = store.zones.get(zone);
                    zone_view(
                        zone,
                        name,
                        false,
                        devices,
                        &claims,
                        docs,
                        history,
                        learn.get(zone),
                        now,
                    )
                })
                .collect();
            for (zone, docs) in &store.zones {
                if !areas.contains_key(zone) {
                    zones.push(zone_view(
                        zone,
                        zone,
                        true,
                        &[],
                        &claims,
                        Some(docs),
                        history,
                        learn.get(zone),
                        now,
                    ));
                }
            }
            zones.sort_by(|a, b| a.orphan.cmp(&b.orphan).then_with(|| a.name.cmp(&b.name)));
            ClimateView {
                enabled: true,
                house: store.house.clone(),
                house_invalid: store.house_invalid.clone(),
                history,
                timezone: self.zone().name().to_owned(),
                zones,
            }
        };
        // The reading and its age are what the engine sees now, not what the last decision froze.
        let stale_after_s = u64::from(view.house.stale_after_min) * 60;
        let outdoor = view.house.outdoor_entity.clone();
        let (feedback, presence) = {
            let rt = self.climate_runtime();
            (rt.feedback.clone(), rt.presence.clone())
        };
        for zone in &mut view.zones {
            zone.room_sensor = inputs::room_sensor(
                self,
                &zone.zone,
                &zone.settings,
                &zone.entities,
                outdoor.as_deref(),
            );
            let (reading, age) = inputs::view_reading(
                self,
                zone.room_sensor.as_deref(),
                &zone.entities,
                stale_after_s,
                u64::from(zone.settings.feedback.quiet_limit_min) * 60,
                now,
                &seen,
            );
            zone.reading = reading;
            zone.reading_age_s = age;
            zone.thermometer =
                inputs::view_thermometer(self, zone.room_sensor.as_deref(), now, &seen);
            zone.correction_step_at = feedback
                .get(&zone.zone)
                .and_then(|state| state.last_step_at);
            zone.presence = zone.settings.presence.enabled.then(|| PresenceView {
                empty_since: presence.get(&zone.zone).and_then(|state| state.empty_since),
            });
        }
        Ok(view)
    }

    /// The latest decisions of `zone` (`limit`, newest first, plus any sharing the oldest one's
    /// second) from memory, then older rows from the history database when it is on (none when
    /// it is off or unreachable).
    pub async fn climate_decision_log(
        &self,
        zone: &str,
        limit: usize,
    ) -> Result<DecisionLog, HubError> {
        self.require_climate()?;
        if !self.climate_zone_ids().iter().any(|z| z == zone) {
            return Err(HubError::UnknownArea(zone.to_owned()));
        }
        let recent: Vec<Decision> = self
            .climate_runtime()
            .decisions
            .get(zone)
            .map(|log| newest_decisions(log, limit))
            .unwrap_or_default();
        // Rows of `recent` are in the database too: take only what is older than them.
        let before = recent.last().map(|d| {
            i64::try_from(d.at)
                .unwrap_or(i64::MAX)
                .saturating_mul(1_000_000)
        });
        let mut older = Vec::new();
        if let Some(history) = self.history().cloned() {
            match history.db() {
                Ok(db) => match db
                    .climate_decisions(zone, before, i64::try_from(limit).unwrap_or(i64::MAX))
                    .await
                {
                    Ok(rows) => {
                        older = rows
                            .into_iter()
                            .map(|row| DecisionRow {
                                at: u64::try_from(row.at / 1_000_000).unwrap_or(0),
                                entity_id: row.entity_id,
                                mode: row.mode,
                                setpoint: row.setpoint,
                                source: row.source,
                                reason: row.reason,
                                applied: row.applied,
                                adjust: row.adjust,
                            })
                            .collect();
                    }
                    Err(err) => tracing::warn!(%zone, error = %err, "climate decisions not read"),
                },
                Err(err) => tracing::warn!(%zone, error = %err, "climate history unavailable"),
            }
        }
        Ok(DecisionLog { recent, older })
    }

    /// Replaces the house settings: validates, publishes retained and applies locally.
    pub fn set_climate_house(&self, house: HouseSettings) -> Result<(), HubError> {
        let prefix = self.require_climate()?.prefix.clone();
        house.validate().map_err(HubError::Rejected)?;
        self.publish_climate(topics::house_topic(&prefix), Some(&house))
    }

    /// Replaces the settings of `zone`, an area with climate entities: validates, publishes
    /// retained and applies locally.
    ///
    /// Setting non-empty rules that differ from the zone's is an approval of them (spec §4.4):
    /// `approved_at` becomes now and a pending zone becomes active. A device another zone
    /// already lists in `entities` is refused.
    pub fn set_climate_zone(&self, zone: &str, mut settings: ZoneSettings) -> Result<(), HubError> {
        let prefix = self.require_climate()?.prefix.clone();
        if !valid_zone_id(zone) {
            return Err(HubError::Rejected(format!("invalid zone id `{zone}`")));
        }
        settings.validate().map_err(HubError::Rejected)?;
        if let Some(listed) = &settings.entities {
            let claims = self.climate_claims();
            let taken = listed.iter().find_map(|id| {
                claims
                    .get(id)
                    .filter(|owner| *owner != zone)
                    .map(|o| (id, o))
            });
            if let Some((id, owner)) = taken {
                return Err(HubError::Rejected(format!(
                    "`{id}` is already listed by zone `{owner}`"
                )));
            }
        }
        let known = zone != UNASSIGNED_AREA
            && self
                .read()
                .effective_areas()
                .iter()
                .any(|area| area.id == zone);
        if !known {
            return Err(HubError::UnknownArea(zone.to_owned()));
        }
        let current = self.zone_settings(zone);
        // New correction settings restart it from the schedule's own target.
        if settings.feedback != current.feedback {
            self.climate_runtime().feedback.remove(zone);
        }
        // New presence settings start over: occupied until `empty_after` passes.
        if settings.presence != current.presence {
            self.climate_runtime().presence.remove(zone);
        }
        if !settings.rules.is_empty() && settings.rules != current.rules {
            settings.approved_at = Some(clock::now());
            if current.state == ZoneState::Pending && settings.state == ZoneState::Pending {
                settings.state = ZoneState::Active;
            }
        }
        self.publish_climate(
            topics::zone_topic(&prefix, zone, ZoneDoc::Settings),
            Some(&settings),
        )
    }

    /// Publishes one climate document retained (`None` publishes the empty payload that deletes
    /// it), applies it locally so the next read sees it before the broker echo, then tells the
    /// UI.
    pub(crate) fn publish_climate(
        &self,
        topic: String,
        value: Option<&impl Serialize>,
    ) -> Result<(), HubError> {
        let payload = match value {
            Some(value) => serde_json::to_string(value)
                .map_err(|err| HubError::Rejected(format!("serialize: {err}")))?,
            None => String::new(),
        };
        self.publish(Publish {
            topic: topic.clone(),
            payload: payload.clone(),
            retain: true,
        })?;
        self.ingest_climate(&topic, payload.as_bytes());
        Ok(())
    }

    /// Feeds a message to the climate store when `topic` is one of its documents; false when it
    /// is not, so the message goes on to the registry.
    pub(crate) fn ingest_climate(&self, topic: &str, payload: &[u8]) -> bool {
        let Some(prefix) = self.climate_prefix() else {
            return false;
        };
        let Some(parsed) = topics::parse(prefix, topic) else {
            return false;
        };
        let was_off = matches!(
            &parsed,
            ClimateTopic::Zone { zone, doc: ZoneDoc::Settings }
                if self.climate_store().zones.get(zone)
                    .is_some_and(|docs| docs.settings.state == ZoneState::Off)
        );
        let applied = self.climate_store().apply(&parsed, payload);
        if was_off
            && let ClimateTopic::Zone { zone, .. } = &parsed
            && self.zone_settings(zone).state != ZoneState::Off
            && let Some(hub) = self.me.upgrade()
        {
            // "On enable": a zone that just left `off` is learned at once.
            super::learning::spawn_zone_learning(&hub, zone);
        }
        match &applied {
            Applied::Changed => {}
            Applied::Invalid(reason) => {
                tracing::warn!(%topic, %reason, "climate document refused, keeping the previous one");
            }
            Applied::Unchanged => return true,
        }
        let zone = match parsed {
            ClimateTopic::House => None,
            ClimateTopic::Zone { zone, .. } => Some(zone),
        };
        self.broadcast(WsEvent::ClimateChanged { zone });
        true
    }

    /// The areas that can be zones, by id: each with its name and the climate entities it holds.
    /// An area whose id cannot be a topic level is skipped, with a warning the first time.
    pub(crate) fn climate_areas(&self) -> BTreeMap<String, (String, Vec<String>)> {
        let mut out: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
        let mut skipped: Vec<String> = Vec::new();
        let reg = self.read();
        let areas = reg.effective_areas();
        for entity in reg.entities() {
            if entity.kind != Kind::Climate
                || entity.unique_id.starts_with(GROUP_PREFIX)
                || entity.area == UNASSIGNED_AREA
            {
                continue;
            }
            let area = &entity.area;
            if !valid_zone_id(area) {
                skipped.push(area.clone());
                continue;
            }
            out.entry(area.clone())
                .or_insert_with(|| {
                    let name = areas
                        .iter()
                        .find(|a| a.id == *area)
                        .map_or_else(|| area.clone(), |a| a.name.clone());
                    (name, Vec::new())
                })
                .1
                .push(entity.unique_id.clone());
        }
        drop(reg);
        for (_, devices) in out.values_mut() {
            devices.sort();
        }
        if !skipped.is_empty() && self.climate_enabled() {
            let mut rt = self.climate_runtime();
            for area in skipped {
                if rt.skipped_areas.insert(area.clone()) {
                    tracing::warn!(
                        area = %area,
                        "area id cannot be a climate zone (empty, or holds `/`, `+` or `#`); skipped"
                    );
                }
            }
        }
        out
    }

    /// Entity -> the zone whose `settings.entities` lists it. A device listed by two zones (only
    /// possible through documents published around the hub) belongs to the first by id.
    pub(crate) fn climate_claims(&self) -> BTreeMap<String, String> {
        let store = self.climate_store();
        let mut claims = BTreeMap::new();
        for (zone, docs) in &store.zones {
            for id in docs.settings.entities.iter().flatten() {
                claims.entry(id.clone()).or_insert_with(|| zone.clone());
            }
        }
        claims
    }

    fn history_configured(&self) -> bool {
        self.history().is_some()
    }
}

/// Whether `id` can be a zone: one topic level, no MQTT wildcard.
pub(crate) fn valid_zone_id(id: &str) -> bool {
    !id.is_empty() && !id.contains(['/', '+', '#'])
}

/// `settings.entities`, or the area's climate entities that no zone lists; never a device
/// another zone claims (see [`Hub::climate_claims`]).
pub(crate) fn zone_devices(
    zone: &str,
    settings: &ZoneSettings,
    area_entities: &[String],
    claims: &BTreeMap<String, String>,
) -> Vec<String> {
    let listed = settings.entities.as_deref();
    listed
        .unwrap_or(area_entities)
        .iter()
        .filter(|id| claims.get(*id).is_none_or(|owner| owner == zone))
        .cloned()
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn zone_view(
    zone: &str,
    name: &str,
    orphan: bool,
    area_entities: &[String],
    claims: &BTreeMap<String, String>,
    docs: Option<&ZoneDocs>,
    history: bool,
    ran: Option<&LearnStatus>,
    now: u64,
) -> ZoneView {
    let empty = ZoneDocs::default();
    let docs = docs.unwrap_or(&empty);
    let learn = if docs.settings.state == ZoneState::Off {
        LearnStatus::Off
    } else if docs.proposal.is_some() {
        LearnStatus::Proposed
    } else if docs.eco.as_ref().is_some_and(|eco| eco.until > now) {
        LearnStatus::Eco
    } else if !history {
        LearnStatus::HistoryOff
    } else {
        match ran {
            Some(
                status @ (LearnStatus::NotEnough { .. }
                | LearnStatus::Rejected
                | LearnStatus::HistoryOff),
            ) => status.clone(),
            _ => LearnStatus::UpToDate,
        }
    };
    ZoneView {
        zone: zone.to_owned(),
        name: name.to_owned(),
        orphan,
        entities: zone_devices(zone, &docs.settings, area_entities, claims),
        settings: docs.settings.clone(),
        settings_invalid: docs.settings_invalid.clone(),
        reading: None,
        reading_age_s: None,
        room_sensor: None,
        thermometer: None,
        correction_step_at: None,
        presence: None,
        decision: docs.decision.clone(),
        overrides: docs.overrides.clone(),
        eco: docs.eco.clone(),
        proposal: docs.proposal.clone(),
        learn,
    }
}

/// The newest `limit` decisions of a log kept oldest first, newest first, plus any sharing the
/// oldest one's second: the database is read strictly before that second, so a decision left
/// out here would be missing from both.
fn newest_decisions(log: &VecDeque<Decision>, limit: usize) -> Vec<Decision> {
    let mut newest = log.iter().rev();
    let mut out: Vec<Decision> = newest.by_ref().take(limit).cloned().collect();
    if let Some(oldest) = out.last().map(|d| d.at) {
        out.extend(newest.take_while(|d| d.at == oldest).cloned());
    }
    out
}

#[cfg(test)]
mod tests {
    use nidoo_climate::doc::Source;

    use super::*;

    fn decision(at: u64, reason: &str) -> Decision {
        Decision {
            zone: "z".into(),
            mode: None,
            setpoint: None,
            source: Source::Idle,
            cause: None,
            reason: reason.into(),
            reading: None,
            reading_age_s: None,
            outdoor_mean: None,
            home: true,
            holiday: false,
            applied: false,
            at,
            entities: BTreeMap::new(),
            eco_restore: BTreeMap::new(),
            adjust: None,
        }
    }

    fn reasons(list: &[Decision]) -> Vec<&str> {
        list.iter().map(|d| d.reason.as_str()).collect()
    }

    #[test]
    fn newest_decisions_keep_a_shared_second_together() {
        let log: VecDeque<Decision> = [
            decision(10, "a"),
            decision(20, "b"),
            decision(20, "c"),
            decision(30, "d"),
        ]
        .into();
        assert_eq!(reasons(&newest_decisions(&log, 1)), ["d"]);
        // "c" is the second newest; "b" shares its second, so it comes too.
        assert_eq!(reasons(&newest_decisions(&log, 2)), ["d", "c", "b"]);
        assert_eq!(reasons(&newest_decisions(&log, 9)), ["d", "c", "b", "a"]);
        assert!(newest_decisions(&log, 0).is_empty());
    }
}
