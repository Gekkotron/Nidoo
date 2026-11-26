//! Learning runs (spec §6.2–§6.4): read a zone's target history and the engine's own commands
//! from the history database, run the pure learner per mode, and turn the outcome into a
//! proposal, "nothing to change", "not enough days" or "already rejected".
//!
//! The database reads are the only impure part; everything that builds the learner's input or
//! interprets its outcome is a plain function, covered without a database.

use std::sync::Arc;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use chrono_tz::Tz;
use nidoo_climate::decide::DeviceCaps;
use nidoo_climate::doc::{
    HhMm, Mode, Proposal, ProposalKind, Rejection, Rules, Source, Window, ZoneSettings, ZoneState,
};
use nidoo_climate::learn::{
    DeviceHistory, EngineCommand, LearnInput, LearnOutcome, TargetSample, diff, differs,
    fingerprint, learn,
};
use nidoo_climate::view::LearnStatus;
use nidoo_core::history::Micros;

use super::inputs::{local_time, zone_entity_ids};
use super::proposals;
use crate::clock;
use crate::history::climate::ClimateDecisionRow;
use crate::hub::Hub;

const DAY_S: u64 = 86_400;
/// Half-hour steps tried to get past a local time that does not exist (spring forward).
const GAP_STEPS: i64 = 4;

/// The two modes a zone can learn, with the hvac mode name a device lists for each.
const MODES: [(Mode, &str); 2] = [(Mode::Heat, "heat"), (Mode::Cool, "cool")];

// ---------------------------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------------------------

/// Unix seconds of a local wall-clock time. A time that does not exist (the clocks jumped
/// forward) moves on half an hour at a time; an ambiguous one is its first occurrence.
pub(crate) fn local_to_unix(tz: Tz, local: NaiveDateTime) -> Option<u64> {
    let instant = (0..=GAP_STEPS).find_map(|step| {
        tz.from_local_datetime(&(local + Duration::minutes(30 * step)))
            .earliest()
    })?;
    u64::try_from(instant.timestamp()).ok()
}

/// The first instant strictly after `after` (Unix seconds) at which the house clock reads
/// `at`; `24:00` is the next midnight.
pub(crate) fn next_learn_at(tz: Tz, after: u64, at: HhMm) -> u64 {
    let today = local_time(tz, after).date();
    for offset in 0..=3 {
        let date = today + Duration::days(offset);
        let local = date.and_time(NaiveTime::MIN) + Duration::minutes(i64::from(at.0));
        if let Some(instant) = local_to_unix(tz, local)
            && instant > after
        {
            return instant;
        }
    }
    // Unreachable for a real zone; one day on is a safe nightly fallback.
    after + DAY_S
}

/// A history time as the house's wall clock.
fn micros_local(tz: Tz, at: Micros) -> NaiveDateTime {
    DateTime::from_timestamp_micros(at)
        .unwrap_or(DateTime::UNIX_EPOCH)
        .with_timezone(&tz)
        .naive_local()
}

/// A device's `(time, target, mode)` rows as the learner's samples, sorted by local time. Rows
/// that share a time collapse to the last one: the history query returns the row in force at
/// the window's start twice when a change landed exactly there, and the later row is the newer.
pub(crate) fn samples_from_rows(
    tz: Tz,
    rows: Vec<(Micros, Option<f64>, Option<String>)>,
) -> Vec<TargetSample> {
    let mut samples: Vec<TargetSample> = rows
        .into_iter()
        .map(|(at, target, mode)| TargetSample {
            at: micros_local(tz, at),
            target,
            mode,
        })
        .collect();
    // Stable: equal times keep the order the database gave them.
    samples.sort_by_key(|s| s.at);
    let mut out: Vec<TargetSample> = Vec::with_capacity(samples.len());
    for sample in samples {
        match out.last_mut() {
            Some(last) if last.at == sample.at => *last = sample,
            _ => out.push(sample),
        }
    }
    out
}

/// The engine's applied decisions as the learner's engine commands. Rows with no device (a
/// decision that commanded nothing) or a source this code does not know are skipped.
pub(crate) fn commands_from_rows(tz: Tz, rows: &[ClimateDecisionRow]) -> Vec<EngineCommand> {
    rows.iter()
        .filter(|row| !row.entity_id.is_empty())
        .filter_map(|row| {
            let source =
                serde_json::from_value::<Source>(serde_json::Value::String(row.source.clone()))
                    .ok()?;
            Some(EngineCommand {
                entity: row.entity_id.clone(),
                at: micros_local(tz, row.at),
                setpoint: row.setpoint,
                mode: row.mode.clone(),
                source,
                adjust: row.adjust,
            })
        })
        .collect()
}

/// What the learner may not go beyond for a group of devices: the tightest limits, the
/// coarsest step.
pub(crate) fn merge_caps(caps: &[DeviceCaps]) -> DeviceCaps {
    let mut hvac_modes: Vec<String> = caps.iter().flat_map(|c| c.hvac_modes.clone()).collect();
    hvac_modes.sort();
    hvac_modes.dedup();
    DeviceCaps {
        hvac_modes,
        fan_modes: Vec::new(),
        min_temp: caps.iter().filter_map(|c| c.min_temp).reduce(f64::max),
        max_temp: caps.iter().filter_map(|c| c.max_temp).reduce(f64::min),
        temp_step: caps.iter().filter_map(|c| c.temp_step).reduce(f64::max),
    }
}

/// The learning window: the `days` whole local days before today, as the first day and the
/// Unix seconds it spans.
pub(crate) fn window_of(tz: Tz, now: u64, days: u32) -> Option<(NaiveDate, Window)> {
    let today = local_time(tz, now).date();
    let first = today - Duration::days(i64::from(days));
    let from = local_to_unix(tz, first.and_time(NaiveTime::MIN))?;
    let to = local_to_unix(tz, today.and_time(NaiveTime::MIN))?;
    Some((first, Window { from, to, days }))
}

/// What a run concluded, before any document is written.
#[derive(Debug, PartialEq)]
pub(crate) enum Verdict {
    NotEnough {
        days: u32,
        needed: u32,
    },
    UpToDate,
    /// The rules this run would propose were rejected and are still remembered.
    Rejected,
    /// The pending proposal is already these rules.
    AlreadyPending,
    Propose {
        kind: ProposalKind,
        rules: Rules,
        changes: Vec<String>,
        fingerprint: String,
    },
}

/// Turns the per-mode outcomes into a verdict. Modes that were not learned keep the approved
/// rules. A zone without rules gets an `Initial` proposal; otherwise only a learned mode that
/// differs from the approved one (or has none yet) proposes a `Change`.
pub(crate) fn verdict(
    settings: &ZoneSettings,
    runs: &[(Mode, LearnOutcome)],
    pending: Option<&Proposal>,
    now: u64,
) -> Verdict {
    let mut candidate = settings.rules.clone();
    let mut learned = false;
    let mut changed = false;
    let mut short: Option<(u32, u32)> = None;
    for (mode, outcome) in runs {
        match outcome {
            LearnOutcome::NotEnough { days, needed } => {
                let best = short.map_or(*days, |(d, _)| d.max(*days));
                short = Some((best, *needed));
            }
            LearnOutcome::Rules(rules) => {
                learned = true;
                // A mode that only jitters within tolerance keeps its approved rules: no
                // edit slips in unseen, and the fingerprint stays the same from run to run.
                let moved = settings
                    .rules
                    .get(*mode)
                    .is_none_or(|approved| differs(approved, rules));
                if moved {
                    changed = true;
                    match mode {
                        Mode::Heat => candidate.heat = Some(rules.clone()),
                        Mode::Cool => candidate.cool = Some(rules.clone()),
                    }
                }
            }
        }
    }
    if !learned {
        return match short {
            Some((days, needed)) => Verdict::NotEnough { days, needed },
            None => Verdict::UpToDate,
        };
    }
    let initial = settings.rules.is_empty();
    if !initial && !changed {
        return Verdict::UpToDate;
    }
    let fingerprint = fingerprint(&candidate);
    if settings
        .rejected
        .iter()
        .any(|r: &Rejection| r.until > now && r.fingerprint == fingerprint)
    {
        return Verdict::Rejected;
    }
    if pending.is_some_and(|p| p.fingerprint == fingerprint) {
        return Verdict::AlreadyPending;
    }
    let (kind, old) = if initial {
        (ProposalKind::Initial, None)
    } else {
        (ProposalKind::Change, Some(&settings.rules))
    };
    let changes = diff(old, &candidate);
    Verdict::Propose {
        kind,
        rules: candidate,
        changes,
        fingerprint,
    }
}

/// `p-<YYYYMMDD-HHMMSS local>-<zone>`.
pub(crate) fn proposal_id(tz: Tz, now: u64, zone: &str) -> String {
    format!("p-{}-{zone}", local_time(tz, now).format("%Y%m%d-%H%M%S"))
}

fn micros(secs: u64) -> Micros {
    i64::try_from(secs)
        .unwrap_or(i64::MAX / 1_000_000)
        .saturating_mul(1_000_000)
}

// ---------------------------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------------------------

/// Learns one zone at `now` and publishes a proposal when there is one. `HistoryOff` without a
/// reachable history database.
pub(crate) async fn learn_zone(hub: &Arc<Hub>, zone: &str, now: u64) -> LearnStatus {
    let settings = hub.zone_settings(zone);
    if settings.state == ZoneState::Off {
        return LearnStatus::Off;
    }
    if in_eco(hub, zone, now) {
        return eco_paused(hub, zone);
    }
    let Some(history) = hub.history().cloned() else {
        return history_off(hub, zone);
    };
    let Ok(db) = history.db() else {
        return history_off(hub, zone);
    };
    let house = hub.climate_store().house.clone();
    let tz = hub.zone();
    let Some((first_day, window)) = window_of(tz, now, house.learn_days) else {
        return LearnStatus::UpToDate;
    };
    let (from, to) = (micros(window.from), micros(window.to));

    // Only devices that list a mode the learner knows are read.
    // The ids first: `zone_entity_ids` reads the registry itself, and a read is never nested.
    let entity_ids = zone_entity_ids(hub, zone, &settings);
    let learnable: Vec<(String, DeviceCaps)> = {
        let reg = hub.read();
        entity_ids
            .into_iter()
            .filter_map(|id| {
                let c = &reg.entity(&id)?.capabilities;
                let caps = DeviceCaps {
                    hvac_modes: c.hvac_modes.clone(),
                    fan_modes: c.fan_modes.clone(),
                    min_temp: c.min_temp,
                    max_temp: c.max_temp,
                    temp_step: c.temp_step,
                };
                let known = MODES
                    .iter()
                    .any(|(_, name)| caps.hvac_modes.iter().any(|m| m == name));
                known.then_some((id, caps))
            })
            .collect()
    };
    let mut histories: Vec<(DeviceHistory, DeviceCaps)> = Vec::new();
    for (id, caps) in learnable {
        match db.climate_targets(&id, from, to).await {
            Ok(rows) => histories.push((
                DeviceHistory {
                    entity: id,
                    samples: samples_from_rows(tz, rows),
                },
                caps,
            )),
            Err(err) => {
                tracing::warn!(zone, entity = %id, error = %err, "climate learning could not read history");
                return history_off(hub, zone);
            }
        }
    }
    let ids: Vec<String> = histories.iter().map(|(h, _)| h.entity.clone()).collect();
    let engine = match db.climate_commands(&ids, from, to).await {
        Ok(rows) => commands_from_rows(tz, &rows),
        Err(err) => {
            tracing::warn!(zone, error = %err, "climate learning could not read decisions");
            return history_off(hub, zone);
        }
    };

    let mut outcomes: Vec<(Mode, LearnOutcome)> = Vec::new();
    for (mode, name) in MODES {
        let (devices, caps): (Vec<DeviceHistory>, Vec<DeviceCaps>) = histories
            .iter()
            .filter(|(_, caps)| caps.hvac_modes.iter().any(|m| m == name))
            .cloned()
            .unzip();
        if devices.is_empty() {
            continue;
        }
        let merged = merge_caps(&caps);
        let input = LearnInput {
            mode,
            devices: &devices,
            engine: &engine,
            approved: settings.rules.get(mode),
            first_day,
            days: house.learn_days,
            min_days: house.learn_min_days,
            support: house.learn_support,
            frost: house.frost,
            caps: &merged,
        };
        outcomes.push((mode, learn(&input)));
    }
    conclude(hub, zone, &outcomes, window, now).await
}

/// The run could not read history: the view must stop showing what an earlier run said.
fn history_off(hub: &Hub, zone: &str) -> LearnStatus {
    hub.climate_runtime()
        .learn_status
        .insert(zone.to_owned(), LearnStatus::HistoryOff);
    LearnStatus::HistoryOff
}

/// The zone holds a room eco at `now`: its days are not its habits, so nothing is learned.
fn in_eco(hub: &Hub, zone: &str, now: u64) -> bool {
    hub.climate_store()
        .zones
        .get(zone)
        .and_then(|docs| docs.eco.as_ref())
        .is_some_and(|eco| eco.until > now)
}

/// The run stopped for the zone's eco: the view says so instead of what an earlier run said.
fn eco_paused(hub: &Hub, zone: &str) -> LearnStatus {
    hub.climate_runtime()
        .learn_status
        .insert(zone.to_owned(), LearnStatus::Eco);
    LearnStatus::Eco
}

/// Applies the outcomes under the engine's pass lock, against the settings as they are now: an
/// approval or rejection that landed during the database reads is not undone by a stale run.
async fn conclude(
    hub: &Arc<Hub>,
    zone: &str,
    outcomes: &[(Mode, LearnOutcome)],
    window: Window,
    now: u64,
) -> LearnStatus {
    let Some(climate) = &hub.climate else {
        return LearnStatus::Off;
    };
    let _pass = climate.pass.lock().await;
    let settings = hub.zone_settings(zone);
    if settings.state == ZoneState::Off {
        return LearnStatus::Off;
    }
    // An eco started during the database reads.
    if in_eco(hub, zone, now) {
        return eco_paused(hub, zone);
    }
    let pending = hub
        .climate_store()
        .zones
        .get(zone)
        .and_then(|docs| docs.proposal.clone());
    let status = match verdict(&settings, outcomes, pending.as_ref(), now) {
        Verdict::NotEnough { days, needed } => LearnStatus::NotEnough { days, needed },
        Verdict::UpToDate => LearnStatus::UpToDate,
        Verdict::Rejected => LearnStatus::Rejected,
        Verdict::AlreadyPending => LearnStatus::Proposed,
        Verdict::Propose {
            kind,
            rules,
            changes,
            fingerprint,
        } => {
            let proposal = Proposal {
                id: proposal_id(hub.zone(), now, zone),
                zone: zone.to_owned(),
                kind,
                created: now,
                window,
                rules,
                changes,
                fingerprint,
            };
            proposals::publish(hub, zone, proposal, now);
            LearnStatus::Proposed
        }
    };
    hub.climate_runtime()
        .learn_status
        .insert(zone.to_owned(), status.clone());
    status
}

/// Clears a zone's "learning now" mark when dropped, so a cancelled run cannot leave it set.
struct RunGuard {
    hub: Arc<Hub>,
    zone: String,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        self.hub.climate_runtime().learning_now.remove(&self.zone);
    }
}

/// Starts a learning run of `zone` without waiting for it; nothing starts while one is
/// already running for that zone.
pub(crate) fn spawn_zone_learning(hub: &Arc<Hub>, zone: &str) {
    if hub.climate.is_none() || tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let Some(guard) = claim_run(hub, zone) else {
        return;
    };
    tokio::spawn(async move { run_claimed(guard).await });
}

/// Marks `zone` as being learned; `None` when a run of it is already in progress.
fn claim_run(hub: &Arc<Hub>, zone: &str) -> Option<RunGuard> {
    hub.climate_runtime()
        .learning_now
        .insert(zone.to_owned())
        .then(|| RunGuard {
            hub: Arc::clone(hub),
            zone: zone.to_owned(),
        })
}

async fn run_claimed(guard: RunGuard) {
    let status = learn_zone(&guard.hub, &guard.zone, clock::now()).await;
    tracing::debug!(zone = %guard.zone, ?status, "climate learning run");
}

/// Learns every zone, one after another, in the background; a zone already being learned is
/// skipped.
pub(crate) fn spawn_learning(hub: &Arc<Hub>) {
    if hub.climate.is_none() || tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let hub = Arc::clone(hub);
    tokio::spawn(async move {
        for zone in hub.climate_zone_ids() {
            if let Some(guard) = claim_run(&hub, &zone) {
                run_claimed(guard).await;
            }
        }
    });
}

#[cfg(test)]
mod tests;
