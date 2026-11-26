//! The hourly supervisor run and the live report clocks.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use nidoo_api::{ConnectionStatus, SupervisorSettings};
use nidoo_core::model::{Availability, Kind, StateValue};
use nidoo_watchdog::supervisor::{Baseline, Ctx, Reading, Step, Supervisor, alerts, baseline};
use serde_json::Value;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

use super::{sensitivity, store};
use crate::clock;
use crate::hub::Hub;

/// Seconds between two runs; the first run waits as long after the start.
pub const RUN_SECS: u64 = 3600;
/// The baseline looks back this far.
const WINDOW_SECS: u64 = 14 * 86_400;
/// The baselines are recomputed this often.
const BASELINE_EVERY: u64 = 86_400;
/// Closed problems are kept this long (they keep past outages out of the baselines).
const KEEP_CLOSED: u64 = 30 * 86_400;

/// The silent check waits while the broker link is down or the Zigbee2MQTT bridge says it is
/// offline: that outage has its own push. A bridge never heard from (no Zigbee2MQTT) does not
/// pause it.
pub fn silent_paused(link: &ConnectionStatus, bridge: Availability) -> bool {
    !matches!(link, ConnectionStatus::Online) || bridge == Availability::Offline
}

/// `readings` without the ignored sensors.
pub(crate) fn without_ignored(readings: Vec<Reading>, s: &SupervisorSettings) -> Vec<Reading> {
    readings
        .into_iter()
        .filter(|r| !s.ignored.contains(&r.entity_id))
        .collect()
}

/// A supervised sensor as the registry has it.
struct Sensor {
    id: String,
    name: String,
    device_class: Option<String>,
    unit: Option<String>,
    value: Option<f64>,
    offline: bool,
}

impl Hub {
    /// Starts the supervisor: keeps the report clocks, and runs the checks an hour after the
    /// start, then every hour. `None` without a database.
    pub fn start_supervisor(self: &Arc<Self>) -> Option<JoinHandle<()>> {
        self.supervisor_db()?;
        let hub = Arc::clone(self);
        let mut touched = self.subscribe_state_touched();
        let mut link = self.connection();
        let every = Duration::from_secs(RUN_SECS);
        Some(tokio::spawn(async move {
            // The open problems show on the page and answer a Mute from the start.
            if let Some(db) = hub.supervisor_db() {
                hub.supervisor_load(&db).await;
            }
            let mut next = tokio::time::Instant::now() + every;
            let mut running: Option<JoinHandle<()>> = None;
            loop {
                tokio::select! {
                    msg = touched.recv() => match msg {
                        Ok(t) => hub.supervisor_touched(&t.id, t.epoch, t.read, clock::now()),
                        Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => break,
                    },
                    changed = link.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        let online = matches!(*link.borrow(), ConnectionStatus::Online);
                        if online {
                            hub.supervisor_rearm(clock::now());
                        }
                    }
                    () = tokio::time::sleep_until(next) => {
                        next = tokio::time::Instant::now() + every;
                        // A run still going (a slow database) is not doubled.
                        if running.as_ref().is_none_or(JoinHandle::is_finished) {
                            let run = Arc::clone(&hub);
                            running = Some(tokio::spawn(async move {
                                run.supervisor_tick(clock::now()).await;
                            }));
                        }
                    }
                }
            }
        }))
    }

    /// A state message for `id`: a read one in the current link epoch sets its report clock.
    /// The retained replay right after the link comes online is not a report.
    pub(crate) fn supervisor_touched(&self, id: &str, epoch: u64, read: bool, now: u64) {
        if read && epoch == self.link_epoch() {
            let mut rt = self.supervisor_state();
            if now >= rt.online_at.saturating_add(super::SETTLE_SECS) {
                rt.last_report.insert(id.to_owned(), now);
            }
        }
    }

    /// The broker link is back: the clocks of a new link epoch start now.
    pub(crate) fn supervisor_rearm(&self, now: u64) {
        let epoch = self.link_epoch();
        let mut rt = self.supervisor_state();
        rt.online_at = now;
        if rt.epoch != epoch {
            rt.epoch = epoch;
            rt.armed_at = now;
        }
    }

    /// One run at `now`: load the open problems once, refresh the baselines once a day, read
    /// the sensors, step, push, store. A database that does not answer skips the run and
    /// keeps every problem as it is.
    pub async fn supervisor_tick(&self, now: u64) {
        let settings = self.supervisor_state().settings.clone();
        if !settings.enabled {
            return;
        }
        let Some(db) = self.supervisor_db() else {
            return;
        };
        if !self.supervisor_load(&db).await {
            tracing::warn!("supervisor run skipped: the database did not answer");
            return;
        }

        let sensors = self.supervised_sensors(&settings);
        let ids: Vec<String> = sensors.iter().map(|s| s.id.clone()).collect();
        let due = self
            .supervisor_state()
            .baselines_at
            .is_none_or(|at| now.saturating_sub(at) >= BASELINE_EVERY);
        if due {
            let since = now.saturating_sub(WINDOW_SECS);
            let stats = store::gap_stats(&db, &ids, since).await;
            let excluded = store::intervals(&db, since).await;
            let (stats, excluded) = match (stats, excluded) {
                (Ok(stats), Ok(excluded)) => (stats, excluded),
                (Err(error), _) | (_, Err(error)) => {
                    tracing::warn!(%error, "supervisor run skipped: the database did not answer");
                    return;
                }
            };
            let baselines: HashMap<String, Baseline> = stats
                .iter()
                .filter_map(|s| {
                    let ex = excluded.get(&s.entity_id).map_or(&[][..], Vec::as_slice);
                    baseline(s, now, ex).map(|b| (s.entity_id.clone(), b))
                })
                .collect();
            let learning = ids.iter().filter(|id| !baselines.contains_key(*id)).count();
            let mut rt = self.supervisor_state();
            rt.baselines = baselines;
            rt.baselines_at = Some(now);
            rt.learning = u32::try_from(learning).unwrap_or(u32::MAX);
        }
        let last_changes = match store::last_changes(&db, &ids).await {
            Ok(last) => last,
            Err(error) => {
                tracing::warn!(%error, "supervisor run skipped: the database did not answer");
                return;
            }
        };

        let link = self.connection().borrow().clone();
        let bridge = self.read().bridge().state;
        let (readings, ctx) = {
            let mut rt = self.supervisor_state();
            let offline: HashSet<&str> = sensors
                .iter()
                .filter(|s| s.offline)
                .map(|s| s.id.as_str())
                .collect();
            rt.unavailable_since
                .retain(|id, _| offline.contains(id.as_str()));
            for id in offline {
                rt.unavailable_since.entry(id.to_owned()).or_insert(now);
            }
            let readings: Vec<Reading> = sensors
                .into_iter()
                .map(|s| Reading {
                    last_change: last_changes.get(&s.id).copied(),
                    last_report: rt.last_report.get(&s.id).copied(),
                    unavailable_since: rt.unavailable_since.get(&s.id).copied(),
                    baseline: rt.baselines.get(&s.id).copied(),
                    entity_id: s.id,
                    name: s.name,
                    device_class: s.device_class,
                    unit: s.unit,
                    value: s.value,
                })
                .collect();
            rt.observe_bridge(bridge, now);
            let ctx = Ctx {
                now,
                armed_at: rt.armed_at,
                factor: sensitivity(settings.sensitivity).factor(),
                silent_paused: silent_paused(&link, bridge),
            };
            (without_ignored(readings, &settings), ctx)
        };

        let (step, _) = self.supervisor_step(&readings, &ctx);
        for p in &step.opened {
            if let Err(error) = store::insert(&db, p).await {
                tracing::warn!(%error, entity = %p.entity_id, "could not store the problem");
            }
        }
        for c in &step.closed {
            if let Err(error) = store::close(&db, &c.problem, c.at).await {
                tracing::warn!(%error, entity = %c.problem.entity_id, "could not close the problem");
            }
        }
        let purge = {
            let mut rt = self.supervisor_state();
            let due = rt
                .purged_at
                .is_none_or(|at| now.saturating_sub(at) >= BASELINE_EVERY);
            if due {
                rt.purged_at = Some(now);
            }
            due
        };
        if purge && let Err(error) = store::purge(&db, now.saturating_sub(KEEP_CLOSED)).await {
            tracing::warn!(%error, "could not purge the closed problems");
        }
    }

    /// Loads the stored open problems once; true when they are loaded. Turned off meanwhile:
    /// nothing is installed (the rows were deleted).
    pub(crate) async fn supervisor_load(&self, db: &crate::history::db::Db) -> bool {
        if self.supervisor_state().loaded {
            return true;
        }
        match store::load_open(db).await {
            Ok(open) => {
                let mut rt = self.supervisor_state();
                if rt.settings.enabled && !rt.loaded {
                    let ignored = rt.settings.ignored.clone();
                    rt.engine = Supervisor::new(
                        open.into_iter()
                            .filter(|p| !ignored.contains(&p.entity_id))
                            .collect(),
                    );
                    rt.loaded = true;
                }
                true
            }
            Err(error) => {
                tracing::warn!(%error, "could not load the open problems");
                false
            }
        }
    }

    /// Steps the engine with `readings` and sends the resulting pushes (per the settings).
    /// Returns the step, with `alert_id` set on what was announced, and the alerts sent.
    pub(crate) fn supervisor_step(&self, readings: &[Reading], ctx: &Ctx) -> (Step, Vec<Value>) {
        let (step, sent) = {
            let mut rt = self.supervisor_state();
            // The settings now, not the ones the run read before its database round: a Mute
            // or a switch-off meanwhile stands.
            if !rt.settings.enabled {
                return (Step::default(), Vec::new());
            }
            let readings = without_ignored(readings.to_vec(), &rt.settings);
            let mut step = rt.engine.step(&readings, ctx);
            let sent = alerts(
                &mut step,
                rt.settings.notify,
                rt.settings.notify_recovered,
                ctx.now,
            );
            (step, sent)
        };
        for alert in &sent {
            self.raise_system(Some(alert.clone()));
        }
        (step, sent)
    }

    /// The registry's numeric sensors (secret ones never). A sensor whose value is no longer
    /// a number stays supervised while it has a baseline or an open problem, so a sensor
    /// sending garbage is not dropped from its own checks.
    fn supervised_sensors(&self, settings: &SupervisorSettings) -> Vec<Sensor> {
        let known: HashSet<String> = {
            let rt = self.supervisor_state();
            rt.baselines
                .keys()
                .cloned()
                .chain(rt.engine.open().map(|p| p.entity_id.clone()))
                .collect()
        };
        let reg = self.read();
        reg.entities()
            .filter(|e| e.kind == Kind::Sensor)
            .filter(|e| !e.capabilities.generic.as_ref().is_some_and(|g| g.secret))
            .filter(|e| !settings.ignored.contains(&e.unique_id))
            .filter_map(|e| {
                let state = reg.state(&e.unique_id);
                let value = state.and_then(|s| match s.value {
                    StateValue::Number(x) if x.is_finite() => Some(x),
                    _ => None,
                });
                if value.is_none() && !known.contains(&e.unique_id) {
                    return None;
                }
                Some(Sensor {
                    id: e.unique_id.clone(),
                    name: e.name.clone(),
                    device_class: e.device_class.clone(),
                    unit: e.unit.clone(),
                    value,
                    offline: state.is_some_and(|s| s.availability == Availability::Offline),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
