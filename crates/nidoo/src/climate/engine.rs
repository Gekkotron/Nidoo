//! The evaluation loop: decide per zone, command the devices, tell a hand change from the
//! engine's own echo, and raise the safety alerts (spec §5.5, §7, §9).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, NaiveDateTime, TimeZone};
use chrono_tz::Tz;
use nidoo_api::{ConnectionStatus, WsEvent};
use nidoo_climate::decide::{Device, SafetyMemory, decide};
use nidoo_climate::doc::{
    Decision, DeviceTarget, FanRestore, FanState, Mode, ModeRules, Override, OverrideOrigin,
    RoomEco, Setpoint, Source, ZoneState,
};
use nidoo_climate::eco::fan_plan;
use nidoo_climate::feedback::FeedbackState;
use nidoo_climate::ledger::{Ledger, Observed, hold_until};
use nidoo_climate::topics::{self, ZoneDoc};
use nidoo_core::history::Micros;
use nidoo_core::model::{Availability, Command, EntityState, Kind, StateValue, UNASSIGNED_AREA};
use serde::Serialize;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::hub::valid_zone_id;
use super::inputs::{
    HomeNow, OUTDOOR_CACHE_S, OwnedZoneInputs, detecting, local_time, note_detection, outdoor_mean,
    outdoor_reading, presence_home_now, room_sensor, step_from, step_of, zone_entity_ids,
    zone_inputs_owned,
};
use super::{GROUP_PREFIX, alerts, learning};
use crate::clock;
use crate::history::climate::ClimateDecisionRow;
use crate::hub::Hub;

/// Every zone is evaluated at least this often.
const TICK: Duration = Duration::from_secs(60);
/// A burst of state changes is evaluated once, this long after the first.
const DEBOUNCE: Duration = Duration::from_secs(2);
/// After the broker link is up, the retained documents and states get this long to arrive.
const BOOT_SETTLE: Duration = Duration::from_secs(10);
/// At most one command per entity in this many seconds.
const MIN_RESEND_S: u64 = 60;
/// Consecutive failed commands that pause an entity.
const MAX_FAILURES: u32 = 3;
/// How long a failing entity is left alone, in seconds.
const PAUSE_S: u64 = 15 * 60;
/// Decisions kept in memory per zone.
const DECISION_LOG: usize = 200;
/// How far back the live outdoor readings are kept, and how far apart they are sampled, in seconds.
const OUTDOOR_LIVE_S: u64 = 24 * 3600;
const OUTDOOR_SAMPLE_GAP_S: u64 = 300;
/// A command published but never echoed is forgotten after this many seconds.
const UNCONFIRMED_S: u64 = 120;
/// Half-hour steps tried to get past a local time that does not exist (spring forward).
const GAP_STEPS: i64 = 4;

/// What commanding a zone achieved.
#[derive(Default)]
struct Outcome {
    needed: usize,
    accepted: usize,
}

/// Clears an entity's in-flight mark when dropped, so a cancelled pass cannot leave it set.
/// A command abandoned while armed may already be published, so it is remembered as
/// unconfirmed: the device echoing it later is not a hand change.
struct InflightGuard<'a> {
    hub: &'a Hub,
    id: &'a str,
    intended: &'a DeviceTarget,
    now: u64,
    armed: bool,
}

impl InflightGuard<'_> {
    /// The command finished, so dropping the guard is no abandonment.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for InflightGuard<'_> {
    fn drop(&mut self) {
        let mut rt = self.hub.climate_runtime();
        rt.inflight.remove(self.id);
        if self.armed {
            rt.unconfirmed
                .insert(self.id.to_owned(), (self.intended.clone(), self.now));
        }
    }
}

impl Outcome {
    fn complete(&self) -> bool {
        self.needed == self.accepted
    }
}

impl Hub {
    /// Starts the evaluation loop; `None` while `[climate] enabled` is off.
    pub fn start_climate(self: &Arc<Self>) -> Option<JoinHandle<()>> {
        if !self.climate_enabled() {
            return None;
        }
        // Subscribed now, so nothing that happens during the boot settle is missed.
        let events = self.subscribe();
        let seen = self.subscribe_state_seen();
        let hub = Arc::clone(self);
        Some(tokio::spawn(
            async move { hub.climate_loop(events, seen).await },
        ))
    }

    /// One full pass over every zone at `now` (Unix seconds).
    pub async fn evaluate_climate(self: &Arc<Self>, now: u64) {
        let Some(climate) = &self.climate else {
            return;
        };
        let _pass = climate.pass.lock().await;
        self.ensure_seeded();
        self.sweep_once(now);
        self.refresh_outdoor(now).await;
        self.refresh_house(now);
        for zone in self.climate_zone_ids() {
            self.evaluate_zone(&zone, now).await;
        }
    }

    /// Evaluates one zone at `now`, serialised with the full passes. What an approval does so
    /// the new rules apply at once.
    pub(crate) async fn evaluate_climate_zone(self: &Arc<Self>, zone: &str, now: u64) {
        let Some(climate) = &self.climate else {
            return;
        };
        let _pass = climate.pass.lock().await;
        self.ensure_seeded();
        self.refresh_house(now);
        self.evaluate_zone(zone, now).await;
    }

    /// Looks at one entity whose state changed: tracks availability, and turns a change of a
    /// climate device nobody commanded into a hand-change override (spec §7).
    pub fn observe_climate(&self, id: &str, now: u64) {
        if !self.climate_enabled() {
            return;
        }
        self.ensure_seeded();
        let observed = {
            let reg = self.read();
            reg.entity(id)
                .zip(reg.state(id))
                .map(|(entity, state)| (entity.kind, entity.capabilities.temp_step, state.clone()))
        };
        let Some((kind, temp_step, state)) = observed else {
            return;
        };
        self.note_availability(id, state.availability, now);
        let StateValue::Climate(climate) = &state.value else {
            return;
        };
        if kind != Kind::Climate || state.availability == Availability::Offline {
            return;
        }
        // No mode yet means the state is not known: nothing to compare.
        let Some(mode) = climate.mode.clone() else {
            return;
        };
        let Some(zone) = self.zone_of_device(id) else {
            return;
        };
        let active = self
            .climate_store()
            .zones
            .get(&zone)
            .is_some_and(|docs| docs.settings.state == ZoneState::Active);
        if !active {
            return;
        }
        let step = step_from(temp_step);
        let verdict = {
            let mut rt = self.climate_runtime();
            if rt.inflight.contains(id) {
                return;
            }
            if let Some((sent, at)) = rt.unconfirmed.get(id).cloned() {
                if same_target(&sent, climate.target, &mode, step) {
                    // The device caught up with a command that timed out: it is ours.
                    rt.ledger.record(id, &sent, now);
                    rt.unconfirmed.remove(id);
                    return;
                }
                if now.saturating_sub(at) > UNCONFIRMED_S {
                    rt.unconfirmed.remove(id);
                }
            }
            let online_since = rt.online_since.get(id).copied();
            rt.ledger
                .classify(id, climate.target, Some(&mode), step, now, online_since)
        };
        if verdict == Observed::HandChange {
            self.record_hand_change(&zone, id, climate.target, &mode, now);
        }
    }

    /// Notes what a presence sensor's state change says, from the state the event carries: the
    /// registry may already hold a later state when the loop gets to the event.
    pub fn observe_presence(&self, id: &str, state: &EntityState, now: u64) {
        let mut rt = self.climate_runtime();
        if rt.presence_sensors.contains(id) {
            note_detection(&mut rt, id, detecting(state), now);
        }
    }

    async fn climate_loop(
        self: Arc<Self>,
        mut events: tokio::sync::broadcast::Receiver<WsEvent>,
        mut seen: tokio::sync::broadcast::Receiver<crate::hub::StateSeen>,
    ) {
        use tokio::sync::broadcast::error::RecvError;

        let mut link = self.connection();
        while !matches!(*link.borrow_and_update(), ConnectionStatus::Online) {
            if link.changed().await.is_err() {
                return;
            }
        }
        tokio::time::sleep(BOOT_SETTLE).await;
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The "on enable" run, then one every night at `learn_at`.
        let mut last_learn = clock::now();
        learning::spawn_learning(&self);
        let mut due: Option<tokio::time::Instant> = None;
        loop {
            let deadline = due;
            tokio::select! {
                biased;
                report = seen.recv() => match report {
                    Ok(report) => self.note_seen(&report.id, clock::now()),
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                },
                event = events.recv() => match event {
                    Ok(WsEvent::StateChanged { id, state }) if self.climate_watches(&id) => {
                        let now = clock::now();
                        self.observe_presence(&id, &state, now);
                        self.observe_climate(&id, now);
                        due.get_or_insert(tokio::time::Instant::now() + DEBOUNCE);
                    }
                    Ok(WsEvent::ClimateChanged { .. }) | Err(RecvError::Lagged(_)) => {
                        due.get_or_insert(tokio::time::Instant::now() + DEBOUNCE);
                    }
                    Ok(_) => {}
                    Err(RecvError::Closed) => break,
                },
                () = async {
                    match deadline {
                        Some(at) => tokio::time::sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                } => {
                    due = None;
                    self.evaluate_climate(clock::now()).await;
                }
                _ = tick.tick() => {
                    due = None;
                    let now = clock::now();
                    self.evaluate_climate(now).await;
                    let learn_at = self.climate_store().house.learn_at;
                    if learning::next_learn_at(self.zone(), last_learn, learn_at) <= now {
                        last_learn = now;
                        learning::spawn_learning(&self);
                    }
                }
            }
        }
    }

    /// Whether a change of `id` matters to the engine (everything does until the first pass
    /// has said what is watched).
    fn climate_watches(&self, id: &str) -> bool {
        let rt = self.climate_runtime();
        rt.watched.is_empty() || rt.watched.contains(id)
    }

    fn note_seen(&self, id: &str, now: u64) {
        let mut rt = self.climate_runtime();
        if rt.watched.is_empty() || rt.watched.contains(id) {
            rt.seen_at.insert(id.to_owned(), now);
        }
    }

    /// Records the availability of `id`; only a real unavailable -> online transition starts the
    /// drift window.
    fn note_availability(&self, id: &str, availability: Availability, now: u64) {
        let mut rt = self.climate_runtime();
        let previous = rt.availability.insert(id.to_owned(), availability);
        if availability == Availability::Online && previous == Some(Availability::Offline) {
            rt.online_since.insert(id.to_owned(), now);
        }
    }

    /// Seeds the command ledger from the retained decisions, newest last so it wins, and picks
    /// up each zone's room-thermometer correction where it was.
    fn ensure_seeded(&self) {
        if self.climate_runtime().seeded {
            return;
        }
        let (mut decisions, feedback): (Vec<Decision>, Vec<(String, FeedbackState)>) = {
            let store = self.climate_store();
            let decisions = store
                .zones
                .values()
                .filter_map(|docs| docs.decision.clone())
                .collect();
            let feedback = store
                .zones
                .iter()
                .filter_map(|(zone, docs)| {
                    let decision = docs.decision.as_ref()?;
                    let state = FeedbackState::restored(decision, docs.settings.feedback.enabled)?;
                    Some((zone.clone(), state))
                })
                .collect();
            (decisions, feedback)
        };
        decisions.sort_by_key(|d| d.at);
        let mut rt = self.climate_runtime();
        if !rt.seeded {
            rt.ledger = Ledger::from_decisions(&decisions);
            rt.feedback.extend(feedback);
            rt.seeded = true;
        }
    }

    /// After a boot, classifies every zone device once: the retained states may have arrived
    /// before the ledger was seeded, or been missed.
    fn sweep_once(&self, now: u64) {
        {
            let mut rt = self.climate_runtime();
            if rt.swept {
                return;
            }
            rt.swept = true;
        }
        let ids: Vec<String> = self
            .climate_zone_ids()
            .iter()
            .flat_map(|zone| {
                let settings = self.zone_settings(zone);
                zone_entity_ids(self, zone, &settings)
            })
            .collect();
        for id in ids {
            self.observe_climate(&id, now);
        }
    }

    pub(crate) fn zone_settings(&self, zone: &str) -> nidoo_climate::doc::ZoneSettings {
        self.climate_store()
            .zones
            .get(zone)
            .map(|docs| docs.settings.clone())
            .unwrap_or_default()
    }

    /// The areas that hold climate entities and the zones that have documents.
    pub(crate) fn climate_zone_ids(&self) -> Vec<String> {
        let mut ids: BTreeSet<String> = self.climate_areas().into_keys().collect();
        ids.extend(self.climate_store().zones.keys().cloned());
        ids.into_iter().collect()
    }

    /// The zone a climate entity belongs to: the one that lists it, else its area.
    fn zone_of_device(&self, id: &str) -> Option<String> {
        let area = {
            let reg = self.read();
            let entity = reg.entity(id)?;
            if entity.kind != Kind::Climate || entity.unique_id.starts_with(GROUP_PREFIX) {
                return None;
            }
            entity.area.clone()
        };
        if let Some(zone) = self.climate_claims().remove(id) {
            return Some(zone);
        }
        let area_is_zone = area != UNASSIGNED_AREA
            && valid_zone_id(&area)
            && self
                .climate_store()
                .zones
                .get(&area)
                .is_none_or(|docs| docs.settings.entities.is_none());
        area_is_zone.then_some(area)
    }

    pub(crate) fn zone_display_name(&self, zone: &str) -> String {
        self.climate_areas()
            .remove(zone)
            .map_or_else(|| zone.to_owned(), |(name, _)| name)
    }

    /// Keeps the outdoor temperature: a live sample, and the history mean when it is due.
    async fn refresh_outdoor(&self, now: u64) {
        let house = self.climate_store().house.clone();
        let Some(entity) = house.outdoor_entity.clone() else {
            return;
        };
        if let Some(value) = outdoor_reading(self, &house, now) {
            let mut rt = self.climate_runtime();
            if rt
                .outdoor_live
                .back()
                .is_none_or(|(at, _)| now.saturating_sub(*at) >= OUTDOOR_SAMPLE_GAP_S)
            {
                rt.outdoor_live.push_back((now, value));
            }
            while rt
                .outdoor_live
                .front()
                .is_some_and(|(at, _)| now.saturating_sub(*at) > OUTDOOR_LIVE_S)
            {
                rt.outdoor_live.pop_front();
            }
        }
        let due = self
            .climate_runtime()
            .outdoor_cache
            .is_none_or(|(at, _)| now.saturating_sub(at) >= OUTDOOR_CACHE_S);
        if !due {
            return;
        }
        let mean = match self.history().map(|history| history.db()) {
            Some(Ok(db)) => db
                .mean_since(
                    &entity,
                    micros(now.saturating_sub(OUTDOOR_LIVE_S)),
                    micros(now),
                )
                .await
                .unwrap_or_else(|err| {
                    tracing::warn!(error = %err, "outdoor mean from history failed");
                    None
                }),
            _ => None,
        };
        self.climate_runtime().outdoor_cache = Some((now, mean));
    }

    /// House-level state of a pass: who is home, the season's mode, and what is watched.
    fn refresh_house(&self, now: u64) {
        let house = self.climate_store().house.clone();
        let home_now = presence_home_now(self, &house);
        let mut watched: BTreeSet<String> = house.presence.iter().cloned().collect();
        // The hub's own alarm sends the house away when armed away, listed or not.
        watched.insert(crate::alarm::ALARM_ENTITY_ID.to_owned());
        watched.extend(house.holiday_entity.clone());
        watched.extend(house.outdoor_entity.clone());
        let mut presence_sensors = HashSet::new();
        for zone in self.climate_zone_ids() {
            let settings = self.zone_settings(&zone);
            if settings.presence.enabled {
                presence_sensors.extend(settings.presence.sensors.iter().cloned());
            }
            let entity_ids = zone_entity_ids(self, &zone, &settings);
            watched.extend(room_sensor(
                self,
                &zone,
                &settings,
                &entity_ids,
                house.outdoor_entity.as_deref(),
            ));
            watched.extend(entity_ids);
        }
        let outdoor = outdoor_mean(self, now);
        let mut rt = self.climate_runtime();
        if home_now == HomeNow::Home || rt.last_home_at.is_none() {
            rt.last_home_at = Some(now);
        }
        if let Some(mode) = nidoo_climate::schedule::season_mode(&house, outdoor, rt.previous_mode)
        {
            rt.previous_mode = Some(mode);
        }
        watched.extend(presence_sensors.iter().cloned());
        rt.watched = watched.into_iter().collect();
        rt.presence_sensors = presence_sensors;
    }

    async fn evaluate_zone(self: &Arc<Self>, zone: &str, now: u64) {
        // A hand change that landed while this pass waited on another zone, or whose event
        // was lost, is caught here against the registry as it is now, before it is commanded
        // back.
        let settings = self.zone_settings(zone);
        if settings.state == ZoneState::Active {
            for id in zone_entity_ids(self, zone, &settings) {
                self.observe_climate(&id, now);
            }
        }
        let Some(mut inp) = zone_inputs_owned(self, zone, now) else {
            return;
        };
        if inp.entity_ids.is_empty() {
            alerts::sync_safety(self, zone, String::new, &no_safety(zone), now);
            return;
        }
        self.tidy_zone(&mut inp);
        inp.previous_cause = self.last_decision(zone).and_then(|d| d.cause);
        let mut decision = decide(&inp.borrowed());
        {
            let kept = FeedbackState::after(
                inp.feedback.as_ref(),
                &decision,
                inp.settings.feedback.enabled,
                now,
            );
            let mut rt = self.climate_runtime();
            match kept {
                Some(state) => {
                    rt.feedback.insert(zone.to_owned(), state);
                }
                None => {
                    rt.feedback.remove(zone);
                }
            }
            match inp.presence {
                Some(state) => {
                    rt.presence.insert(zone.to_owned(), state);
                }
                None => {
                    rt.presence.remove(zone);
                }
            }
        }
        let state = inp.settings.state;
        if state == ZoneState::Off {
            // What is done by hand while the zone is off is not a change against the engine:
            // once back on, its devices start from what they hold, with no hold.
            let mut rt = self.climate_runtime();
            for id in &inp.entity_ids {
                rt.ledger.forget(id);
            }
        }
        if state == ZoneState::Active {
            self.acknowledge_at_target(&inp, &decision, now);
        }
        if state != ZoneState::Pending {
            self.climate_runtime().safety_memory.remove(zone);
        }
        let restoring = state == ZoneState::Pending
            && inp.safety_memory.is_some()
            && decision.source != Source::Safety;
        let may_command = match state {
            ZoneState::Active => true,
            ZoneState::Pending => decision.source == Source::Safety || restoring,
            ZoneState::Off => false,
        };
        let outcome = if may_command {
            self.command_zone(&inp, &decision, now).await
        } else {
            Outcome::default()
        };
        decision.applied = match state {
            ZoneState::Active => outcome.complete(),
            ZoneState::Pending => may_command && outcome.needed > 0 && outcome.complete(),
            ZoneState::Off => false,
        };
        if restoring && outcome.complete() {
            let mut rt = self.climate_runtime();
            if let Some(memory) = rt.safety_memory.remove(zone) {
                for id in memory.found.keys() {
                    rt.ledger.forget(id);
                }
            }
        }
        let previous_restore = self
            .last_decision(zone)
            .map(|d| d.eco_restore)
            .unwrap_or_default();
        self.command_fans(&inp, &mut decision, &previous_restore, now)
            .await;
        alerts::sync_safety(self, zone, || self.zone_display_name(zone), &decision, now);
        self.publish_decision(zone, decision, now);
    }

    /// Forgets entities that left the registry, drops the zone's expired overrides and the ones
    /// for entities it no longer holds, and deletes an eco that has ended.
    fn tidy_zone(&self, inp: &mut OwnedZoneInputs) {
        let previous: Vec<String> = self
            .last_decision(&inp.zone)
            .map(|d| d.entities.keys().cloned().collect())
            .unwrap_or_default();
        let gone: BTreeSet<String> = previous
            .into_iter()
            .chain(inp.overrides.iter().filter_map(|o| o.entity.clone()))
            .filter(|id| self.read().entity(id).is_none())
            .collect();
        for id in &gone {
            {
                let mut rt = self.climate_runtime();
                rt.ledger.forget(id);
                rt.failures.remove(id);
                rt.last_sent.remove(id);
                rt.unconfirmed.remove(id);
                rt.availability.remove(id);
                rt.online_since.remove(id);
            }
            alerts::resolve(self, &alerts::command_id(id));
        }
        let now = inp.now;
        if inp.room_eco.as_ref().is_some_and(|eco| eco.until <= now) {
            self.write_zone_doc(&inp.zone, ZoneDoc::Eco, None::<&RoomEco>);
            inp.room_eco = None;
        }
        let ids = inp.entity_ids.clone();
        inp.overrides = self.update_overrides(&inp.zone, |list| {
            list.retain(|o| {
                o.until > now && o.entity.as_ref().is_none_or(|entity| ids.contains(entity))
            });
        });
    }

    /// Changes the zone's override list under the write lock, from what the store holds now,
    /// so it never undoes a change made meanwhile by another writer. Writes only when the list
    /// changed; returns the list.
    pub(crate) fn update_overrides(
        &self,
        zone: &str,
        change: impl FnOnce(&mut Vec<Override>),
    ) -> Vec<Override> {
        let Some(climate) = &self.climate else {
            return Vec::new();
        };
        let _write = climate
            .overrides_write
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = self
            .climate_store()
            .zones
            .get(zone)
            .map(|docs| docs.overrides.clone())
            .unwrap_or_default();
        let mut list = before.clone();
        change(&mut list);
        if list != before {
            self.write_overrides(zone, &list);
        }
        list
    }

    /// A device already at what the decision wants is taken as the engine's baseline, so a hand
    /// change on it later is seen as one, even though the engine never had to command it.
    fn acknowledge_at_target(&self, inp: &OwnedZoneInputs, decision: &Decision, now: u64) {
        let mut rt = self.climate_runtime();
        for device in &inp.devices {
            let (Some(target), Some(mode)) = (decision.entities.get(&device.id), &device.mode)
            else {
                continue;
            };
            if target.held || needs_command(device, target) {
                continue;
            }
            let step = step_of(&device.caps);
            let known = rt
                .ledger
                .classify(&device.id, device.target, Some(mode), step, now, None);
            if known != Observed::Matches {
                let state = DeviceTarget {
                    setpoint: device.target,
                    mode: mode.clone(),
                    held: false,
                };
                rt.ledger.acknowledge(&device.id, &state, now);
            }
        }
    }

    /// Sends what the decision wants to every device that differs and is not held.
    async fn command_zone(
        self: &Arc<Self>,
        inp: &OwnedZoneInputs,
        decision: &Decision,
        now: u64,
    ) -> Outcome {
        let plans: Vec<(&Device, &DeviceTarget)> = inp
            .devices
            .iter()
            .filter_map(|device| {
                let target = decision.entities.get(&device.id)?;
                (!target.held && needs_command(device, target)).then_some((device, target))
            })
            .collect();
        if plans.is_empty() {
            return Outcome::default();
        }
        if inp.settings.state == ZoneState::Pending
            && decision.source == Source::Safety
            && inp.safety_memory.is_none()
        {
            // Only the devices safety is about to change are put back when it clears.
            let touched: Vec<Device> = plans.iter().map(|(device, _)| (*device).clone()).collect();
            self.climate_runtime()
                .safety_memory
                .insert(inp.zone.clone(), SafetyMemory::capture(&touched));
        }
        let mut outcome = Outcome {
            needed: plans.len(),
            accepted: 0,
        };
        for (device, target) in plans {
            if self.command_device(device, target, now).await {
                outcome.accepted += 1;
            }
        }
        outcome
    }

    /// Commands one device unless it was tried less than a minute ago or is paused. True when
    /// everything it needed was accepted.
    async fn command_device(&self, device: &Device, target: &DeviceTarget, now: u64) -> bool {
        // No broker link: nothing can be sent, and that is not the device's fault.
        let link = self.connection();
        if !matches!(*link.borrow(), ConnectionStatus::Online) {
            return false;
        }
        {
            let mut rt = self.climate_runtime();
            let paused = rt
                .failures
                .get(&device.id)
                .and_then(|(_, until)| *until)
                .is_some_and(|until| now < until);
            let recent = rt
                .last_sent
                .get(&device.id)
                .is_some_and(|at| now.saturating_sub(*at) < MIN_RESEND_S);
            if paused || recent {
                return false;
            }
            rt.last_sent.insert(device.id.clone(), now);
            rt.inflight.insert(device.id.clone());
        }
        let mut inflight = InflightGuard {
            hub: self,
            id: &device.id,
            intended: target,
            now,
            armed: true,
        };
        let result = self.send_target(device, target, now).await;
        inflight.disarm();
        if result.is_ok() {
            self.climate_runtime().unconfirmed.remove(&device.id);
        }
        self.note_command_result(&device.id, result, &link, now)
    }

    /// Whether `id` is paused after repeated failures.
    fn command_paused(&self, id: &str, now: u64) -> bool {
        self.climate_runtime()
            .failures
            .get(id)
            .and_then(|(_, until)| *until)
            .is_some_and(|until| now < until)
    }

    /// Counts a command's outcome for `id`: a success clears its failures and alert; the
    /// `MAX_FAILURES`th failure in a row pauses it and raises the alert. A failure while the
    /// broker link changed since `link` was taken is not the device's fault and is not
    /// counted. True on success.
    fn note_command_result(
        &self,
        id: &str,
        result: Result<(), String>,
        link: &watch::Receiver<ConnectionStatus>,
        now: u64,
    ) -> bool {
        let alert = alerts::command_id(id);
        match result {
            Ok(()) => {
                self.climate_runtime().failures.remove(id);
                alerts::resolve(self, &alert);
                true
            }
            Err(reason) if link.has_changed().unwrap_or(true) => {
                tracing::debug!(entity = %id, %reason, "climate command lost with the broker link");
                false
            }
            Err(reason) => {
                let failures = {
                    let mut rt = self.climate_runtime();
                    let entry = rt.failures.entry(id.to_owned()).or_insert((0, None));
                    entry.0 += 1;
                    if entry.0 >= MAX_FAILURES {
                        entry.1 = Some(now + PAUSE_S);
                    }
                    entry.0
                };
                tracing::warn!(entity = %id, %reason, failures, "climate command failed");
                if failures >= MAX_FAILURES {
                    alerts::raise(
                        self,
                        &alert,
                        "Climate command failing",
                        &format!(
                            "{id} did not accept {MAX_FAILURES} commands in a row; paused for {} minutes. Last error: {reason}",
                            PAUSE_S / 60
                        ),
                        "warning",
                    );
                }
                false
            }
        }
    }

    /// Sets the fan mode of `id` for eco (spec §7). Shares the failure count and pause of the
    /// target commands, but not their resend spacing, so it can follow one in the same pass.
    async fn command_fan(&self, id: &str, fan_mode: &str, now: u64) -> bool {
        let link = self.connection();
        if !matches!(*link.borrow(), ConnectionStatus::Online) || self.command_paused(id, now) {
            return false;
        }
        let command = Command::SetFanMode {
            mode: fan_mode.to_owned(),
        };
        let result = match self.send_command(id, command).await {
            Ok(response) if response.accepted => Ok(()),
            Ok(response) => Err(response
                .reason
                .unwrap_or_else(|| "not confirmed".to_owned())),
            Err(err) => Err(err.to_string()),
        };
        self.note_command_result(id, result, &link, now)
    }

    /// Sends the fan commands of eco for `decision` and keeps what to put back in it. Only an
    /// active zone sets fans; a zone switched off or back to pending puts back what eco changed,
    /// once, and so does a unit that left the zone.
    async fn command_fans(
        &self,
        inp: &OwnedZoneInputs,
        decision: &mut Decision,
        previous: &BTreeMap<String, FanRestore>,
        now: u64,
    ) {
        let (here, departed): (BTreeMap<_, _>, BTreeMap<_, _>) = previous
            .clone()
            .into_iter()
            .partition(|(id, _)| inp.entity_ids.contains(id));
        let mut kept = self.restore_departed(departed, now).await;
        let (mut memory, commands) = fan_plan(
            &here,
            decision,
            &inp.devices,
            &inp.house.eco.actions,
            inp.settings.state != ZoneState::Active,
        );
        for command in commands {
            let landed = self
                .command_fan(&command.entity, &command.fan_mode, now)
                .await;
            match (memory.get_mut(&command.entity), landed) {
                // Set and confirmed.
                (Some(entry), true) if entry.set == command.fan_mode => {
                    entry.state = FanState::Set;
                }
                // Not confirmed: the entry stays `Sent`, and the next pass looks again.
                (Some(_), _) => {}
                // A restore that did not land: keep what to put back, try again next pass.
                (None, false) => {
                    if let Some(entry) = here.get(&command.entity) {
                        memory.insert(command.entity, entry.clone());
                    }
                }
                (None, true) => {}
            }
        }
        memory.append(&mut kept);
        decision.eco_restore = memory;
    }

    /// Puts back the fans eco changed on units that left the zone, if still at the eco value,
    /// and forgets them; returns the ones whose restore did not land, to try again.
    async fn restore_departed(
        &self,
        departed: BTreeMap<String, FanRestore>,
        now: u64,
    ) -> BTreeMap<String, FanRestore> {
        let mut kept = BTreeMap::new();
        for (id, entry) in departed {
            let fan = {
                let reg = self.read();
                if reg.entity(&id).is_none() {
                    continue;
                }
                reg.state(&id).and_then(|state| match &state.value {
                    StateValue::Climate(climate) => climate.fan_mode.clone(),
                    _ => None,
                })
            };
            if entry.state == FanState::Released || fan.as_deref() != Some(entry.set.as_str()) {
                continue;
            }
            if !self.command_fan(&id, &entry.was, now).await {
                kept.insert(id, entry);
            }
        }
        kept
    }

    /// The mode first when it differs, then the temperature; the ledger follows each step the
    /// device accepted.
    async fn send_target(
        &self,
        device: &Device,
        target: &DeviceTarget,
        now: u64,
    ) -> Result<(), String> {
        let mut current = device.target;
        if device.mode.as_deref() != Some(target.mode.as_str()) {
            let intended = DeviceTarget {
                setpoint: device.target,
                mode: target.mode.clone(),
                held: false,
            };
            let command = Command::SetHvacMode {
                mode: target.mode.clone(),
            };
            self.send_step(&device.id, command, &intended, now).await?;
            // A device may change its target with its mode: read it back.
            current = match self.read().state(&device.id).map(|s| &s.value) {
                Some(StateValue::Climate(climate)) => climate.target,
                _ => current,
            };
            let accepted = DeviceTarget {
                setpoint: current,
                ..intended
            };
            self.climate_runtime()
                .ledger
                .record(&device.id, &accepted, now);
        }
        // An off device has no target worth sending.
        if let Some(wanted) = target.setpoint.filter(|_| target.mode != "off") {
            let step = step_of(&device.caps);
            if current.is_none_or(|c| (c - wanted).abs() > step / 2.0) {
                let command = Command::SetTemperature {
                    temperature: wanted,
                };
                self.send_step(&device.id, command, target, now).await?;
            }
        }
        self.climate_runtime()
            .ledger
            .record(&device.id, target, now);
        Ok(())
    }

    /// Sends one command and waits for its echo. A command that was published but never
    /// echoed is remembered, so the device catching up later is not taken for a hand change.
    async fn send_step(
        &self,
        id: &str,
        command: Command,
        intended: &DeviceTarget,
        now: u64,
    ) -> Result<(), String> {
        match self.send_command(id, command).await {
            Ok(response) if response.accepted => Ok(()),
            Ok(response) => {
                self.climate_runtime()
                    .unconfirmed
                    .insert(id.to_owned(), (intended.clone(), now));
                Err(response
                    .reason
                    .unwrap_or_else(|| "not confirmed".to_owned()))
            }
            Err(err) => Err(err.to_string()),
        }
    }

    /// Turns a hand change of `id` into an override held until the zone's next scheduled
    /// change, at most `manual_hold_max`, and acknowledges the state in the ledger so the
    /// events that follow are not hand changes again.
    fn record_hand_change(&self, zone: &str, id: &str, target: Option<f64>, mode: &str, now: u64) {
        let hint = match mode {
            "heat" => Some(Mode::Heat),
            "cool" => Some(Mode::Cool),
            _ => None,
        };
        let until = self.hand_hold_until(zone, now);
        let setpoint = if mode == "off" {
            Some(Setpoint::Off)
        } else {
            target.map(Setpoint::Temp)
        };
        self.update_overrides(zone, |list| {
            list.retain(|o| o.until > now && o.entity.as_deref() != Some(id));
            list.push(Override {
                entity: Some(id.to_owned()),
                setpoint,
                mode: hint,
                until,
                origin: OverrideOrigin::Hand,
            });
        });
        self.climate_runtime().ledger.acknowledge(
            id,
            &DeviceTarget {
                setpoint: target,
                mode: mode.to_owned(),
                held: false,
            },
            now,
        );
        tracing::info!(zone, entity = id, until, "hand change held");
    }

    /// How long a hand change on `zone` is held: until the zone's next scheduled change, at most
    /// `manual_hold_max_min` from `now`. The assistant's setpoints get the same hold.
    pub fn hand_hold_until(&self, zone: &str, now: u64) -> u64 {
        let settings = self.zone_settings(zone);
        // "Until the next scheduled change" is the zone's schedule for the season's mode, not
        // whatever mode the device was put in.
        let rules = self
            .season_mode_now(now)
            .and_then(|m| settings.rules.get(m));
        let tz = self.zone();
        hold_until(
            now,
            secs_until_next_change(tz, rules, local_time(tz, now), now),
            settings.manual_hold_max_min,
        )
    }

    /// The mode the house's season runs in now.
    fn season_mode_now(&self, now: u64) -> Option<Mode> {
        let house = self.climate_store().house.clone();
        let outdoor = outdoor_mean(self, now);
        let previous = self.climate_runtime().previous_mode;
        nidoo_climate::schedule::season_mode(&house, outdoor, previous)
    }

    fn write_overrides(&self, zone: &str, overrides: &[Override]) {
        if overrides.is_empty() {
            self.write_zone_doc(zone, ZoneDoc::Override, None::<&Vec<Override>>);
        } else {
            self.write_zone_doc(zone, ZoneDoc::Override, Some(&overrides.to_vec()));
        }
    }

    /// Publishes a zone document retained. When the broker is unreachable the hub still
    /// applies it locally, so the engine keeps honouring what it decided.
    pub(crate) fn write_zone_doc<T: Serialize>(&self, zone: &str, doc: ZoneDoc, value: Option<&T>) {
        let Some(prefix) = self.climate_prefix() else {
            return;
        };
        let topic = topics::zone_topic(prefix, zone, doc);
        if let Err(err) = self.publish_climate(topic.clone(), value) {
            tracing::warn!(%topic, error = %err, "climate document kept locally only");
            let payload = value
                .and_then(|v| serde_json::to_vec(v).ok())
                .unwrap_or_default();
            self.ingest_climate(&topic, &payload);
        }
    }

    fn last_decision(&self, zone: &str) -> Option<Decision> {
        let logged = self
            .climate_runtime()
            .decisions
            .get(zone)
            .and_then(|log| log.back().cloned());
        logged.or_else(|| {
            self.climate_store()
                .zones
                .get(zone)
                .and_then(|docs| docs.decision.clone())
        })
    }

    /// Publishes, logs and records `decision` when it differs from the previous one.
    fn publish_decision(&self, zone: &str, decision: Decision, now: u64) {
        if self
            .last_decision(zone)
            .is_some_and(|previous| previous.same_outcome(&decision))
        {
            return;
        }
        {
            let mut rt = self.climate_runtime();
            let log = rt.decisions.entry(zone.to_owned()).or_default();
            log.push_back(decision.clone());
            while log.len() > DECISION_LOG {
                log.pop_front();
            }
        }
        self.write_zone_doc(zone, ZoneDoc::Decision, Some(&decision));
        if let Some(history) = self.history().cloned() {
            let rows = decision_rows(&decision, micros(now));
            tokio::spawn(async move {
                let Ok(db) = history.db() else {
                    return;
                };
                if let Err(err) = db.insert_climate_decisions(&rows).await {
                    tracing::warn!(error = %err, "climate decision not recorded in history");
                }
            });
        }
    }
}

/// A decision that acts on nothing, to resolve a zone's safety alerts.
fn no_safety(zone: &str) -> Decision {
    Decision {
        zone: zone.to_owned(),
        mode: None,
        setpoint: None,
        source: Source::Idle,
        cause: None,
        reason: String::new(),
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

fn micros(secs: u64) -> Micros {
    i64::try_from(secs)
        .unwrap_or(i64::MAX / 1_000_000)
        .saturating_mul(1_000_000)
}

/// One history row per device; a decision with no devices is one row with empty entity and mode.
fn decision_rows(decision: &Decision, at: Micros) -> Vec<ClimateDecisionRow> {
    let source = serde_json::to_value(decision.source)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let row = |entity_id: &str, mode: &str, setpoint: Option<f64>, adjust: Option<f64>| {
        ClimateDecisionRow {
            at,
            zone: decision.zone.clone(),
            entity_id: entity_id.to_owned(),
            mode: mode.to_owned(),
            setpoint,
            source: source.clone(),
            reason: decision.reason.clone(),
            applied: decision.applied,
            adjust,
        }
    };
    if decision.entities.is_empty() {
        return vec![row("", "", None, None)];
    }
    decision
        .entities
        .iter()
        .map(|(id, target)| {
            // A corrected decision stores the target and the offset that was added to it for
            // each unheld device; a held device keeps its own value.
            if let (Some(_), false, Some(Setpoint::Temp(goal)), Some(commanded)) = (
                decision.adjust,
                target.held,
                decision.setpoint,
                target.setpoint,
            ) {
                let diff = commanded - goal;
                let adjust = (diff != 0.0).then_some(diff);
                return row(id, &target.mode, Some(goal), adjust);
            }
            row(id, &target.mode, target.setpoint, None)
        })
        .collect()
}

/// Whether `device` differs from `target` by more than half a step, or in mode. The target of
/// an off device does not count.
fn needs_command(device: &Device, target: &DeviceTarget) -> bool {
    let mode_differs = device.mode.as_deref() != Some(target.mode.as_str());
    let step = step_of(&device.caps);
    let temperature_differs = target.mode != "off"
        && target.setpoint.is_some_and(|wanted| {
            device
                .target
                .is_none_or(|current| (current - wanted).abs() > step / 2.0)
        });
    mode_differs || temperature_differs
}

/// Whether an observed state is the one `sent`.
fn same_target(sent: &DeviceTarget, target: Option<f64>, mode: &str, step: f64) -> bool {
    // An off device has no meaningful target.
    let same = sent.mode == "off"
        || match (sent.setpoint, target) {
            (None, None) => true,
            (Some(a), Some(b)) => (a - b).abs() <= step / 2.0,
            _ => false,
        };
    same && sent.mode == mode
}

/// Seconds from `now` to the next change of `rules`' setpoint, `None` when it never changes.
/// A local time that does not exist (the clocks jumped forward) moves on half an hour at a
/// time; an ambiguous one (the clocks went back) is its first occurrence.
fn secs_until_next_change(
    tz: Tz,
    rules: Option<&ModeRules>,
    local_now: NaiveDateTime,
    now: u64,
) -> Option<u64> {
    let next = rules?.next_change(local_now)?;
    let instant = (0..=GAP_STEPS).find_map(|step| {
        tz.from_local_datetime(&(next + ChronoDuration::minutes(30 * step)))
            .earliest()
    })?;
    let at = u64::try_from(instant.timestamp()).ok()?;
    Some(at.saturating_sub(now).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_climate::decide::DeviceCaps;
    use nidoo_climate::presence::PresenceState;

    fn device(target: Option<f64>, mode: &str) -> Device {
        Device {
            id: "climate.a".into(),
            caps: DeviceCaps {
                hvac_modes: vec!["heat".into(), "off".into()],
                fan_modes: vec![],
                min_temp: None,
                max_temp: None,
                temp_step: Some(0.5),
            },
            target,
            mode: Some(mode.into()),
            fan_mode: None,
        }
    }

    fn want(setpoint: Option<f64>, mode: &str) -> DeviceTarget {
        DeviceTarget {
            setpoint,
            mode: mode.into(),
            held: false,
        }
    }

    fn guard_hub() -> Arc<Hub> {
        let mut cfg = crate::config::Config::default();
        cfg.climate.enabled = true;
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        Hub::new(&cfg, tx)
    }

    #[test]
    fn an_armed_guard_dropped_leaves_the_command_unconfirmed() {
        let hub = guard_hub();
        let intended = want(Some(19.0), "heat");
        hub.climate_runtime().inflight.insert("climate.a".into());
        drop(InflightGuard {
            hub: &hub,
            id: "climate.a",
            intended: &intended,
            now: 7,
            armed: true,
        });
        let rt = hub.climate_runtime();
        assert!(rt.inflight.is_empty());
        assert_eq!(rt.unconfirmed.get("climate.a"), Some(&(intended, 7)));
    }

    #[test]
    fn a_disarmed_guard_dropped_only_clears_the_mark() {
        let hub = guard_hub();
        let intended = want(Some(19.0), "heat");
        hub.climate_runtime().inflight.insert("climate.a".into());
        let mut guard = InflightGuard {
            hub: &hub,
            id: "climate.a",
            intended: &intended,
            now: 7,
            armed: true,
        };
        guard.disarm();
        drop(guard);
        let rt = hub.climate_runtime();
        assert!(rt.inflight.is_empty());
        assert!(rt.unconfirmed.is_empty());
    }

    #[tokio::test]
    async fn settings_change_resets_correction() {
        let mut cfg = crate::config::Config::default();
        cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
        cfg.climate.enabled = true;
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let hub = Hub::new(&cfg, tx);
        hub.set_connection(ConnectionStatus::Online);
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../nidoo-core/tests/fixtures/discovery/climate__daikin_192_168_1_150__climate.json",
        );
        hub.ingest(
            "nidoo/climate/daikin_192_168_1_150/climate/config",
            &std::fs::read(fixture).unwrap(),
        );
        let id = hub.snapshot().entities[0].id.clone();
        let mut patch = serde_json::Map::new();
        patch.insert("area".to_owned(), serde_json::Value::from("living"));
        hub.set_override(&id, &patch).await.unwrap();
        let state = FeedbackState {
            target: 19.0,
            offset: -0.5,
            since: 1,
            last_step_at: Some(2),
        };
        hub.climate_runtime()
            .feedback
            .insert("living".to_owned(), state);
        // Other settings keep the correction.
        let settings = nidoo_climate::doc::ZoneSettings {
            sensor: Some("sensor.x".to_owned()),
            manual_hold_max_min: 100,
            ..Default::default()
        };
        hub.set_climate_zone("living", settings.clone()).unwrap();
        assert!(hub.climate_runtime().feedback.contains_key("living"));
        // A change to the correction settings drops it.
        let mut settings = settings;
        settings.feedback.band = 0.8;
        hub.set_climate_zone("living", settings).unwrap();
        assert!(!hub.climate_runtime().feedback.contains_key("living"));
    }

    #[tokio::test]
    async fn presence_settings_change_resets_state() {
        let mut cfg = crate::config::Config::default();
        cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
        cfg.climate.enabled = true;
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let hub = Hub::new(&cfg, tx);
        hub.set_connection(ConnectionStatus::Online);
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../nidoo-core/tests/fixtures/discovery/climate__daikin_192_168_1_150__climate.json",
        );
        hub.ingest(
            "nidoo/climate/daikin_192_168_1_150/climate/config",
            &std::fs::read(fixture).unwrap(),
        );
        let id = hub.snapshot().entities[0].id.clone();
        let mut patch = serde_json::Map::new();
        patch.insert("area".to_owned(), serde_json::Value::from("living"));
        hub.set_override(&id, &patch).await.unwrap();
        let state = PresenceState {
            last_detected: 1,
            empty_since: Some(2),
            arrival: None,
        };
        hub.climate_runtime()
            .presence
            .insert("living".to_owned(), state);
        let mut settings = nidoo_climate::doc::ZoneSettings::default();
        settings.presence.enabled = true;
        hub.set_climate_zone("living", settings.clone()).unwrap();
        hub.climate_runtime()
            .presence
            .insert("living".to_owned(), state);
        // Other settings keep the state.
        settings.manual_hold_max_min = 100;
        hub.set_climate_zone("living", settings.clone()).unwrap();
        assert!(hub.climate_runtime().presence.contains_key("living"));
        // A change to the presence settings drops it.
        settings.presence.setback = 3.0;
        hub.set_climate_zone("living", settings).unwrap();
        assert!(!hub.climate_runtime().presence.contains_key("living"));
    }

    #[test]
    fn a_command_is_needed_beyond_half_a_step_or_for_another_mode() {
        assert!(!needs_command(
            &device(Some(20.0), "heat"),
            &want(Some(20.25), "heat")
        ));
        assert!(needs_command(
            &device(Some(20.0), "heat"),
            &want(Some(20.5), "heat")
        ));
        assert!(needs_command(
            &device(Some(20.0), "off"),
            &want(Some(20.0), "heat")
        ));
        assert!(needs_command(
            &device(None, "heat"),
            &want(Some(20.0), "heat")
        ));
        assert!(!needs_command(&device(None, "off"), &want(None, "off")));
        assert!(!needs_command(
            &device(Some(9.0), "off"),
            &want(Some(10.0), "off")
        ));
    }

    #[tokio::test]
    async fn restart_restores_the_retained_correction() {
        let mut cfg = crate::config::Config::default();
        cfg.climate.enabled = true;
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let hub = Hub::new(&cfg, tx);
        let decision = |zone: &str| {
            let mut d = no_safety(zone);
            d.source = Source::Schedule;
            d.setpoint = Some(Setpoint::Temp(19.0));
            d.adjust = Some(-1.0);
            d.at = 500;
            d
        };
        {
            let mut store = hub.climate_store();
            let living = store.zones.entry("living".to_owned()).or_default();
            living.settings.feedback.enabled = true;
            living.decision = Some(decision("living"));
            // Correction switched off since: nothing to restore.
            let office = store.zones.entry("office".to_owned()).or_default();
            office.decision = Some(decision("office"));
        }
        hub.ensure_seeded();
        let rt = hub.climate_runtime();
        assert_eq!(
            rt.feedback.get("living"),
            Some(&FeedbackState {
                target: 19.0,
                offset: -1.0,
                since: 500,
                last_step_at: Some(500),
            })
        );
        assert!(!rt.feedback.contains_key("office"));
    }

    #[test]
    fn decision_rows_store_target_and_adjust() {
        let mut decision = no_safety("z");
        decision.setpoint = Some(Setpoint::Temp(19.0));
        decision.adjust = Some(-0.5);
        decision
            .entities
            .insert("a".into(), want(Some(18.5), "heat"));
        decision.entities.insert(
            "b".into(),
            DeviceTarget {
                setpoint: Some(21.0),
                mode: "heat".into(),
                held: true,
            },
        );
        let rows = decision_rows(&decision, 0);
        let got: Vec<_> = rows
            .iter()
            .map(|r| (r.entity_id.as_str(), r.setpoint, r.adjust))
            .collect();
        assert_eq!(
            got,
            vec![("a", Some(19.0), Some(-0.5)), ("b", Some(21.0), None)]
        );

        decision.adjust = None;
        let rows = decision_rows(&decision, 0);
        assert_eq!(rows[0].setpoint, Some(18.5));
        assert_eq!(rows[0].adjust, None);
    }

    #[tokio::test]
    async fn hand_hold_until_matches_hand_change() {
        use nidoo_climate::doc::{Block, Day, HhMm, ModeRules, Rules, Season};
        let mut cfg = crate::config::Config::default();
        cfg.climate.enabled = true;
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let hub = Hub::new(&cfg, tx);
        let tz = hub.zone();
        // A Monday, 10:00 house time.
        let now = tz
            .with_ymd_and_hms(2026, 11, 2, 10, 0, 0)
            .single()
            .unwrap()
            .timestamp() as u64;
        let all_days = vec![
            Day::Mon,
            Day::Tue,
            Day::Wed,
            Day::Thu,
            Day::Fri,
            Day::Sat,
            Day::Sun,
        ];
        let schedule = |from: u16, to: u16| Rules {
            heat: Some(ModeRules {
                base: Setpoint::Temp(17.0),
                blocks: vec![Block {
                    days: all_days.clone(),
                    from: HhMm(from),
                    to: HhMm(to),
                    setpoint: Setpoint::Temp(20.0),
                    evidence: None,
                }],
            }),
            cool: None,
        };
        {
            let mut store = hub.climate_store();
            store.house.season = Season::Heat;
            let living = store.zones.entry("living".to_owned()).or_default();
            living.settings.manual_hold_max_min = 240;
            // The next change is at 11:30, in 90 minutes.
            living.settings.rules = schedule(11 * 60 + 30, 12 * 60);
        }
        assert_eq!(hub.hand_hold_until("living", now), now + 90 * 60);
        hub.record_hand_change("living", "climate.a", Some(21.0), "heat", now);
        let held = hub.climate_store().zones["living"].overrides.clone();
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].until, hub.hand_hold_until("living", now));

        // The next change in 6 h: held for manual_hold_max_min.
        hub.climate_store()
            .zones
            .get_mut("living")
            .unwrap()
            .settings
            .rules = schedule(16 * 60, 17 * 60);
        assert_eq!(hub.hand_hold_until("living", now), now + 240 * 60);
    }
}
