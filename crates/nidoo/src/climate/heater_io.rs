//! On/off heaters on the hub side: switch them as the decision says, keep each one's on/off
//! clocks, tell a switch flipped by hand from the engine's own echo, and send one last off
//! when a zone stops heating with them.

use std::sync::Arc;

use nidoo_api::ConnectionStatus;
use nidoo_climate::doc::{
    Decision, HeaterRun, Mode, Override, OverrideOrigin, Setpoint, ZoneSettings, ZoneState,
};
use nidoo_core::model::{Availability, Command, StateValue};

use super::ClimateRuntime;
use super::engine::{InflightGuard, Outcome, UNCONFIRMED_S};
use crate::hub::Hub;

impl Hub {
    /// A switch that is some zone's heater changed state: update its clock, and record a hand
    /// change when the engine did not ask for it.
    pub fn observe_heater(&self, id: &str, on: bool, now: u64) {
        if !self.climate_enabled() {
            return;
        }
        let Some(zone) = self.heater_claims().remove(id) else {
            return;
        };
        self.seed_heater_clocks(&zone, now);
        if self.note_switch(id, on, now) && self.is_hand_flip(&zone, id, on, now) {
            self.record_heater_hand(&zone, id, on, now);
        }
    }

    /// Send on/off for the switch targets of `decision`; same pause, resend spacing and
    /// failure counting as climate units.
    pub(crate) async fn command_heaters(
        self: &Arc<Self>,
        zone: &str,
        decision: &Decision,
        now: u64,
    ) -> Outcome {
        let settings = self.zone_settings(zone);
        // The decision only targets switches in heat, on a zone that is not off; checked
        // again here because a stray command lights a stove.
        if decision.mode != Some(Mode::Heat) || settings.state == ZoneState::Off {
            return Outcome::default();
        }
        let plans: Vec<(String, bool)> = settings
            .heaters
            .iter()
            .filter(|heater| heater.enabled)
            .filter_map(|heater| {
                let target = decision.entities.get(&heater.entity)?;
                if target.held {
                    return None;
                }
                let on = target.mode == "heat";
                (self.switch_state(&heater.entity) != Some(on)).then(|| (heater.entity.clone(), on))
            })
            .collect();
        let mut outcome = Outcome {
            needed: plans.len(),
            accepted: 0,
        };
        for (id, on) in plans {
            if self.switch_heater(&id, on, now, true).await {
                outcome.accepted += 1;
            }
        }
        outcome
    }

    /// The zone stopped deciding in heat: one off to each heater the previous decision had on
    /// and unheld, that is still on.
    pub(crate) async fn release_heaters(
        self: &Arc<Self>,
        zone: &str,
        previous: &Decision,
        now: u64,
    ) {
        if previous.mode != Some(Mode::Heat) {
            return;
        }
        let settings = self.zone_settings(zone);
        let ids: Vec<String> = settings
            .heaters
            .iter()
            // A heater disabled since is the user's now.
            .filter(|heater| heater.enabled)
            .filter(|heater| {
                previous
                    .entities
                    .get(&heater.entity)
                    .is_some_and(|target| target.mode == "heat" && !target.held)
            })
            .filter(|heater| self.switch_state(&heater.entity) == Some(true))
            .map(|heater| heater.entity.clone())
            .collect();
        for id in ids {
            // Switching off is always allowed: not held back by a recent command.
            if self.switch_heater(&id, false, now, false).await {
                tracing::info!(zone, entity = %id, "heater switched off on leaving heat");
            }
        }
    }

    /// Brings the clocks of `zone`'s heaters up to date with the registry before a pass: a
    /// change whose event was lost is seen here, a heater seen for the first time starts its
    /// clock now, and the clocks of heaters no zone lists any more are dropped.
    pub(crate) fn sync_heaters(&self, zone: &str, settings: &ZoneSettings, now: u64) {
        let claims = self.heater_claims();
        {
            let mut rt = self.climate_runtime();
            rt.heater_clocks.retain(|id, _| claims.contains_key(id));
            rt.heater_sent.retain(|id, _| claims.contains_key(id));
        }
        if settings.heaters.is_empty() {
            return;
        }
        self.seed_heater_clocks(zone, now);
        for heater in &settings.heaters {
            match self.switch_state(&heater.entity) {
                Some(on) => self.observe_heater(&heater.entity, on, now),
                None => start_clock(&mut self.climate_runtime(), &heater.entity, None, now),
            }
        }
    }

    /// Writes the runtime clocks into `decision`, so the retained copy holds what this pass's
    /// commands changed.
    pub(crate) fn copy_heater_clocks(&self, decision: &mut Decision) {
        let rt = self.climate_runtime();
        for (id, run) in &mut decision.heaters {
            if let Some(clock) = rt.heater_clocks.get(id) {
                run.off_since = clock.off_since;
                run.on_since = clock.on_since;
            }
        }
    }

    /// Once per zone after start: its heaters' clocks from its retained decision, and what that
    /// decision asked of them as what was last sent. A heater with no clock there starts one
    /// now from what it shows (unknown counts as off), so a fresh hub never lights a stove
    /// early.
    fn seed_heater_clocks(&self, zone: &str, now: u64) {
        if self.climate_runtime().heater_seeded.contains(zone) {
            return;
        }
        let (ids, retained) = {
            let store = self.climate_store();
            let docs = store.zones.get(zone);
            let ids: Vec<String> = docs
                .map(|d| {
                    d.settings
                        .heaters
                        .iter()
                        .map(|h| h.entity.clone())
                        .collect()
                })
                .unwrap_or_default();
            (ids, docs.and_then(|d| d.decision.clone()))
        };
        let states: Vec<(String, Option<bool>)> = ids
            .iter()
            .map(|id| (id.clone(), self.switch_state(id)))
            .collect();
        let mut rt = self.climate_runtime();
        if !rt.heater_seeded.insert(zone.to_owned()) {
            return;
        }
        if let Some(decision) = retained {
            for id in &ids {
                if let Some(run) = decision.heaters.get(id) {
                    rt.heater_clocks
                        .entry(id.clone())
                        .or_insert_with(|| HeaterRun {
                            off_since: run.off_since,
                            on_since: run.on_since,
                            wait_until: None,
                        });
                }
                if let Some(target) = decision.entities.get(id).filter(|t| !t.held) {
                    rt.heater_sent
                        .entry(id.clone())
                        .or_insert((target.mode == "heat", 0));
                }
            }
        }
        for (id, on) in states {
            start_clock(&mut rt, &id, on, now);
        }
    }

    /// Notes that `id` was seen `on` at `now`: an off starts `off_since` and clears `on_since`,
    /// an on the reverse. True when this changed its state; a heater with no clock yet only
    /// starts one.
    fn note_switch(&self, id: &str, on: bool, now: u64) -> bool {
        let mut rt = self.climate_runtime();
        let Some(run) = rt.heater_clocks.get_mut(id) else {
            start_clock(&mut rt, id, Some(on), now);
            return false;
        };
        let was_on = run.on_since.is_some();
        let was_off = run.off_since.is_some();
        if (on && was_on && !was_off) || (!on && was_off && !was_on) {
            return false;
        }
        *run = if on {
            HeaterRun {
                on_since: Some(now),
                ..HeaterRun::default()
            }
        } else {
            HeaterRun {
                off_since: Some(now),
                ..HeaterRun::default()
            }
        };
        true
    }

    /// Whether `id` turning `on` is someone else's doing: an enabled heater of an active zone
    /// deciding in heat, with no command on the wire, none sent within the confirmation
    /// window, and a state other than the one last sent (or nothing sent at all).
    fn is_hand_flip(&self, zone: &str, id: &str, on: bool, now: u64) -> bool {
        let settings = self.zone_settings(zone);
        if settings.state != ZoneState::Active
            || !settings.heater(id).is_some_and(|heater| heater.enabled)
            || self.last_decision(zone).and_then(|d| d.mode) != Some(Mode::Heat)
        {
            return false;
        }
        let rt = self.climate_runtime();
        if rt.inflight.contains(id) {
            return false;
        }
        match rt.heater_sent.get(id) {
            Some(&(sent, at)) => sent != on && now.saturating_sub(at) > UNCONFIRMED_S,
            None => true,
        }
    }

    /// Holds a hand flip of `id` until the zone's next scheduled change, at most
    /// `manual_hold_max`, and takes its state as the last asked for, so the reports that follow
    /// are not hand flips again.
    fn record_heater_hand(&self, zone: &str, id: &str, on: bool, now: u64) {
        let until = self.hand_hold_until(zone, now);
        self.update_overrides(zone, |list| {
            list.retain(|o| o.until > now && o.entity.as_deref() != Some(id));
            list.push(Override {
                entity: Some(id.to_owned()),
                setpoint: (!on).then_some(Setpoint::Off),
                mode: on.then_some(Mode::Heat),
                until,
                origin: OverrideOrigin::Hand,
            });
        });
        self.climate_runtime()
            .heater_sent
            .entry(id.to_owned())
            .and_modify(|sent| sent.0 = on)
            .or_insert((on, 0));
        tracing::info!(zone, entity = id, on, until, "heater hand flip held");
    }

    /// Switches `id` on or off, unless it is paused or (when `spaced`) was tried less than a
    /// minute ago. True when the switch confirmed it; its clock then follows.
    async fn switch_heater(&self, id: &str, on: bool, now: u64, spaced: bool) -> bool {
        // No broker link: nothing can be sent, and that is not the switch's fault.
        let link = self.connection();
        if !matches!(*link.borrow(), ConnectionStatus::Online) {
            return false;
        }
        if !self.claim_command(id, now, spaced) {
            return false;
        }
        // Noted before sending: an echo that comes late is still the engine's.
        self.climate_runtime()
            .heater_sent
            .insert(id.to_owned(), (on, now));
        let mut inflight = InflightGuard {
            hub: self,
            id,
            intended: None,
            now,
            armed: true,
        };
        let command = if on {
            Command::TurnOn {
                brightness: None,
                color: None,
                effect: None,
            }
        } else {
            Command::TurnOff
        };
        let result = match self.send_command(id, command).await {
            Ok(response) if response.accepted => Ok(()),
            Ok(response) => Err(response
                .reason
                .unwrap_or_else(|| "not confirmed".to_owned())),
            Err(err) => Err(err.to_string()),
        };
        inflight.disarm();
        let accepted = self.note_command_result(id, result, &link, now);
        if accepted {
            self.note_switch(id, on, now);
        }
        accepted
    }

    /// What switch `id` shows: `None` while it is unknown or unreachable.
    fn switch_state(&self, id: &str) -> Option<bool> {
        let reg = self.read();
        let state = reg.state(id)?;
        match state.value {
            StateValue::OnOff(on) if state.availability != Availability::Offline => Some(on),
            _ => None,
        }
    }
}

/// Starts the clock of a heater that has none, from what it shows now: on is on since now,
/// off or unknown is off since now.
fn start_clock(rt: &mut ClimateRuntime, id: &str, on: Option<bool>, now: u64) {
    rt.heater_clocks.entry(id.to_owned()).or_insert_with(|| {
        if on == Some(true) {
            HeaterRun {
                on_since: Some(now),
                ..HeaterRun::default()
            }
        } else {
            HeaterRun {
                off_since: Some(now),
                ..HeaterRun::default()
            }
        }
    });
}
