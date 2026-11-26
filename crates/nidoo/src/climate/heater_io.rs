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
use super::engine::{InflightGuard, Outcome};
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
        if self.note_switch(id, on, now) {
            self.judge_flip(&zone, id, on, now);
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
                Some(on) => {
                    self.observe_heater(&heater.entity, on, now);
                    self.recheck_flip(zone, &heater.entity, on, now);
                }
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
    /// decision asked of them as what was last sent (at an unknown time). The switch as it is
    /// wins over the retained clock: one that shows another state (a power cut, an off that
    /// never applied) restarts its clock now from what it shows, and that is no hand flip. A
    /// heater with no clock starts one now too (unknown counts as off), so a fresh hub, or a
    /// stove found off, never lights early.
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
            let disagrees = on.is_some()
                && rt
                    .heater_clocks
                    .get(&id)
                    .is_some_and(|run| clock_shows(run) != on);
            if disagrees {
                rt.heater_clocks.remove(&id);
            }
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

    /// `id` was just seen turning `on`. A state the engine did not ask for (another than the
    /// one it last sent, or nothing sent at all) is a hand flip, at once: only the echo of a
    /// command to that same state is the engine's, in flight or late. Outside an active zone
    /// deciding in heat, or on a disabled heater, it is no hand flip and is taken as the state
    /// to start from, so it never turns into a hold later.
    fn judge_flip(&self, zone: &str, id: &str, on: bool, now: u64) {
        if !self.flips_are_held(zone, id) {
            acknowledge(&mut self.climate_runtime(), id, on);
            return;
        }
        let sent = self.climate_runtime().heater_sent.get(id).copied();
        if sent.is_none_or(|(sent, _)| sent != on) {
            self.record_heater_hand(zone, id, on, now);
        }
    }

    /// The per-pass net under [`Self::judge_flip`]: a switch that changed after the engine's
    /// last command to it, into another state than that command's, is a hand flip, even if
    /// its change was seen where it was not judged. A switch that never moved after a command
    /// (a failing command) is not, nor one whose last command has no known send time (what a
    /// retained decision asked for, seeded at start: it may never have been sent).
    fn recheck_flip(&self, zone: &str, id: &str, on: bool, now: u64) {
        let unanswered = {
            let rt = self.climate_runtime();
            let since = rt
                .heater_clocks
                .get(id)
                .and_then(|run| if on { run.on_since } else { run.off_since });
            matches!(
                (since, rt.heater_sent.get(id)),
                (Some(since), Some(&(sent, at))) if sent != on && at > 0 && since > at
            )
        };
        if unanswered && self.flips_are_held(zone, id) {
            self.record_heater_hand(zone, id, on, now);
        }
    }

    /// Whether a flip of `id` becomes a hold: an enabled heater of an active zone whose
    /// current decision is in heat.
    fn flips_are_held(&self, zone: &str, id: &str) -> bool {
        let settings = self.zone_settings(zone);
        settings.state == ZoneState::Active
            && settings.heater(id).is_some_and(|heater| heater.enabled)
            && self.last_decision(zone).and_then(|d| d.mode) == Some(Mode::Heat)
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
        acknowledge(&mut self.climate_runtime(), id, on);
        tracing::info!(zone, entity = id, on, until, "heater hand flip held");
    }

    /// Whether `id` may be switched on at this moment of the pass (`now` is its start). The
    /// last guard of the cool-down, on the one path that publishes an on: a switch seen off
    /// that its clock (what the decision saw) still has on was switched off during the pass,
    /// so the change is noted now and the switch is left alone this pass; and no on goes out
    /// before `off_since + min_off_min`.
    fn may_switch_on(&self, id: &str, now: u64) -> bool {
        let at = self.moment(now);
        let clock_on = self
            .climate_runtime()
            .heater_clocks
            .get(id)
            .is_some_and(|run| run.on_since.is_some());
        if clock_on && self.switch_state(id) == Some(false) {
            self.observe_heater(id, false, at);
            return false;
        }
        let min_off_min = self
            .heater_claims()
            .remove(id)
            .and_then(|zone| self.zone_settings(&zone).heater(id).map(|h| h.min_off_min));
        let off_since = self
            .climate_runtime()
            .heater_clocks
            .get(id)
            .and_then(|run| run.off_since);
        match (off_since, min_off_min) {
            (Some(off), Some(min)) if off + u64::from(min) * 60 <= at => true,
            // The decision waits out the cool-down itself: getting here is a bug worth seeing.
            (Some(off), Some(min)) => {
                tracing::warn!(entity = id, off, min, "heater on refused: cooling down");
                false
            }
            // No off time to go by (its clock says on while its state is unknown), or a heater
            // no zone lists: refused, quietly, as it may repeat every pass.
            _ => {
                tracing::debug!(entity = id, ?off_since, "heater on refused: no off time");
                false
            }
        }
    }

    /// The time at a moment later in the pass that started at `now`.
    fn moment(&self, now: u64) -> u64 {
        match self.climate_runtime().pass_clock {
            Some((at, started)) if at == now => now + started.elapsed().as_secs(),
            _ => now,
        }
    }

    /// Switches `id` on or off, unless it is paused or (when `spaced`) was tried less than a
    /// minute ago. True when the switch confirmed it; its clock then follows.
    async fn switch_heater(&self, id: &str, on: bool, now: u64, spaced: bool) -> bool {
        // No broker link: nothing can be sent, and that is not the switch's fault.
        let link = self.connection();
        if !matches!(*link.borrow(), ConnectionStatus::Online) {
            return false;
        }
        if on && !self.may_switch_on(id, now) {
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
            // Stamped when the switch confirmed it, not when the pass started.
            self.note_switch(id, on, self.moment(now));
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

/// Takes `on` as the state the engine last asked of `id`, keeping when it last sent one.
fn acknowledge(rt: &mut ClimateRuntime, id: &str, on: bool) {
    rt.heater_sent
        .entry(id.to_owned())
        .and_modify(|sent| sent.0 = on)
        .or_insert((on, 0));
}

/// What a clock says the switch is: on, off, or not known.
fn clock_shows(run: &HeaterRun) -> Option<bool> {
    match (run.on_since, run.off_since) {
        (Some(_), None) => Some(true),
        (None, Some(_)) => Some(false),
        _ => None,
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

#[cfg(test)]
mod tests {
    use nidoo_climate::doc::Heater;

    use super::*;

    #[tokio::test]
    async fn no_on_leaves_before_the_cool_down_ends() {
        let mut cfg = crate::config::Config::default();
        cfg.climate.enabled = true;
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let hub = Hub::new(&cfg, tx);
        hub.climate_store()
            .zones
            .entry("living".to_owned())
            .or_default()
            .settings
            .heaters = vec![Heater {
            entity: "switch.stove".to_owned(),
            min_off_min: 15,
            ..Heater::default()
        }];
        let now = 10_000;
        // No clock: never on.
        assert!(!hub.may_switch_on("switch.stove", now));
        let off_since = |at: u64| HeaterRun {
            off_since: Some(at),
            ..HeaterRun::default()
        };
        let set = |run: HeaterRun| {
            hub.climate_runtime()
                .heater_clocks
                .insert("switch.stove".to_owned(), run);
        };
        set(off_since(now - 14 * 60));
        assert!(!hub.may_switch_on("switch.stove", now));
        set(off_since(now - 15 * 60));
        assert!(hub.may_switch_on("switch.stove", now));
        // A heater no zone lists has no cool-down to go by: never on.
        assert!(!hub.may_switch_on("switch.other", now));
    }

    /// A hub whose active zone `living`, deciding in heat, has the heater `switch.stove`.
    fn living_with_stove() -> Arc<Hub> {
        let mut cfg = crate::config::Config::default();
        cfg.climate.enabled = true;
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let hub = Hub::new(&cfg, tx);
        {
            let mut store = hub.climate_store();
            let living = store.zones.entry("living".to_owned()).or_default();
            living.settings.state = ZoneState::Active;
            living.settings.heaters = vec![Heater {
                entity: "switch.stove".to_owned(),
                ..Heater::default()
            }];
            let mut decision = super::super::engine::no_safety("living");
            decision.mode = Some(Mode::Heat);
            living.decision = Some(decision);
        }
        hub
    }

    /// The stove off since `since`, after the engine last asked it on at `sent_at` (`0`: at an
    /// unknown time, as seeded at start).
    fn seen_off_after_on_sent_at(hub: &Hub, since: u64, sent_at: u64) {
        let mut rt = hub.climate_runtime();
        rt.heater_sent
            .insert("switch.stove".to_owned(), (true, sent_at));
        rt.heater_clocks.insert(
            "switch.stove".to_owned(),
            HeaterRun {
                off_since: Some(since),
                ..HeaterRun::default()
            },
        );
    }

    fn living_overrides(hub: &Hub) -> Vec<Override> {
        hub.climate_store().zones["living"].overrides.clone()
    }

    #[tokio::test]
    async fn the_pass_recheck_holds_a_change_made_after_the_last_command_only() {
        let hub = living_with_stove();
        // Off since before the engine's on: the on failed, nobody touched it.
        seen_off_after_on_sent_at(&hub, 50, 100);
        hub.recheck_flip("living", "switch.stove", false, 300);
        assert!(living_overrides(&hub).is_empty());
        // Off since after the engine's on: someone switched it off.
        seen_off_after_on_sent_at(&hub, 200, 100);
        hub.recheck_flip("living", "switch.stove", false, 300);
        let held = living_overrides(&hub);
        assert_eq!(held.len(), 1, "{held:?}");
        assert_eq!(held[0].setpoint, Some(Setpoint::Off));
        // Taken as the state to start from: not held again.
        hub.recheck_flip("living", "switch.stove", false, 400);
        assert_eq!(living_overrides(&hub), held);
    }

    #[tokio::test]
    async fn the_pass_recheck_never_holds_against_a_send_of_unknown_time() {
        let hub = living_with_stove();
        // What a retained decision asked for, seeded at start: it may never have been sent.
        seen_off_after_on_sent_at(&hub, 200, 0);
        hub.recheck_flip("living", "switch.stove", false, 300);
        assert!(living_overrides(&hub).is_empty());
    }
}
