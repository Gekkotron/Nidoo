//! The alarm state machine: pure and deterministic, so the hub only feeds it events and clock ticks.
//!
//! `step` applies one [`Event`] to the persisted [`Runtime`] and returns the [`Effect`]s the hub must
//! carry out. `next_deadline` says when the hub must send the next [`Event::Tick`].

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::doc::{AlarmSettings, ArmMode, Entry, RemoteAction, SensorKind};

/// Where the alarm is in its cycle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum Phase {
    Disarmed,
    Arming {
        mode: ArmMode,
        until: u64,
    },
    Armed {
        mode: ArmMode,
    },
    Pending {
        mode: ArmMode,
        until: u64,
    },
    Triggered {
        back_to: Option<ArmMode>,
        until: u64,
    },
}

/// What asked for the last arm or disarm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum Source {
    Presence,
    Remote,
    Sos,
    Command,
}

/// The last state seen for a presence tracker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    Home,
    Away,
    Unknown,
}

/// Everything the engine persists between events (retained, §9).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Runtime {
    pub phase: Phase,
    pub source: Option<Source>,
    pub since: u64,
    pub motion_hits: Vec<(u64, String)>,
    pub open_at_arm: BTreeSet<String>,
    pub cause: Vec<String>,
    pub alert_ids: Vec<String>,
    pub trackers: BTreeMap<String, Presence>,
    /// The trackers whose last definite state was away: only these can come home (an
    /// unknown reading in between does not forget it, nor count as a departure).
    #[serde(default)]
    pub was_away: BTreeSet<String>,
    pub auto_arm_ready: bool,
    pub everyone_away_since: Option<u64>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            phase: Phase::Disarmed,
            source: None,
            since: 0,
            motion_hits: Vec::new(),
            open_at_arm: BTreeSet::new(),
            cause: Vec::new(),
            alert_ids: Vec::new(),
            trackers: BTreeMap::new(),
            was_away: BTreeSet::new(),
            auto_arm_ready: false,
            everyone_away_since: None,
        }
    }
}

impl Runtime {
    /// The published alarm state name.
    pub fn state_name(&self) -> &'static str {
        match self.phase {
            Phase::Disarmed => "disarmed",
            Phase::Arming { .. } => "arming",
            Phase::Armed { mode } => match mode {
                ArmMode::Away => "armed_away",
                ArmMode::Home => "armed_home",
                ArmMode::Night => "armed_night",
            },
            Phase::Pending { .. } => "pending",
            Phase::Triggered { .. } => "triggered",
        }
    }

    /// The arm mode this phase belongs to (for `Triggered`, the mode it returns to).
    pub fn mode(&self) -> Option<ArmMode> {
        match self.phase {
            Phase::Disarmed => None,
            Phase::Arming { mode, .. } | Phase::Armed { mode } | Phase::Pending { mode, .. } => {
                Some(mode)
            }
            Phase::Triggered { back_to, .. } => back_to,
        }
    }

    /// When the current phase ends by itself.
    pub fn deadline(&self) -> Option<u64> {
        match self.phase {
            Phase::Arming { until, .. }
            | Phase::Pending { until, .. }
            | Phase::Triggered { until, .. } => Some(until),
            Phase::Disarmed | Phase::Armed { .. } => None,
        }
    }

    fn enter(&mut self, phase: Phase, source: Option<Source>, now: u64) {
        self.phase = phase;
        if source.is_some() {
            self.source = source;
        }
        self.since = now;
    }
}

/// The watched sensors that are on, as the hub sees them right now.
pub struct World<'a> {
    pub on: &'a BTreeSet<String>,
}

/// Something that happened.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Arm {
        mode: ArmMode,
        source: Source,
    },
    Disarm {
        source: Source,
    },
    Sensor {
        entity: String,
        on: bool,
    },
    /// A watched sensor reported a definite value for the first time since it was unknown or
    /// absent (a restart, a reconnect, a newly watched entity): not a change, so an opening
    /// found open is only watched for closing and nothing else happens.
    SensorFound {
        entity: String,
        on: bool,
    },
    Signal {
        entity: String,
        signal: String,
    },
    Tracker {
        entity: String,
        seen: Presence,
    },
    Tick,
    Resume,
}

/// Something the hub must do.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    SirensOn,
    SirensOff,
    Chirp,
    Alert {
        cause: Vec<String>,
    },
    ResolveAlerts,
    /// The alarm is now armed in `mode`, at the command of `source`; `open` lists the watched
    /// openings that were open at that moment (watched for closing, §4.3).
    Armed {
        mode: ArmMode,
        source: Option<Source>,
        open: Vec<String>,
    },
    /// The alarm went from an armed, arming, pending or ringing state to disarmed.
    Disarmed {
        source: Source,
    },
}

/// The action a remote's signal is mapped to.
fn remote_action(s: &AlarmSettings, entity: &str, signal: &str) -> Option<RemoteAction> {
    let remote = s.remotes.iter().find(|r| r.entity == entity)?;
    remote.signals.get(signal).copied()
}

/// A hit is inside the window while `now < at + window_s`.
fn hit_alive(at: u64, now: u64, s: &AlarmSettings) -> bool {
    now < at + u64::from(s.motion.window_s)
}

/// The watched openings of `mode` that are on right now.
fn open_openings(s: &AlarmSettings, world: &World, mode: ArmMode) -> BTreeSet<String> {
    s.sensors
        .iter()
        .filter(|r| r.kind == SensorKind::Opening && r.modes.contains(&mode))
        .filter(|r| world.on.contains(&r.entity))
        .map(|r| r.entity.clone())
        .collect()
}

/// Enters `Armed`: what is open now is watched for closing (§4.3), the panel chirps and the
/// phones get an "armed" push.
fn enter_armed(
    rt: &mut Runtime,
    s: &AlarmSettings,
    world: &World,
    mode: ArmMode,
    source: Option<Source>,
    now: u64,
) -> Vec<Effect> {
    rt.enter(Phase::Armed { mode }, source, now);
    rt.motion_hits.clear();
    rt.open_at_arm = open_openings(s, world, mode);
    vec![
        Effect::Chirp,
        Effect::Armed {
            mode,
            source: rt.source,
            open: rt.open_at_arm.iter().cloned().collect(),
        },
    ]
}

fn trigger(
    rt: &mut Runtime,
    s: &AlarmSettings,
    back_to: Option<ArmMode>,
    cause: Vec<String>,
    source: Option<Source>,
    now: u64,
) -> Vec<Effect> {
    rt.enter(
        Phase::Triggered {
            back_to,
            until: now + u64::from(s.siren_s),
        },
        source,
        now,
    );
    rt.motion_hits.clear();
    rt.cause = cause.clone();
    vec![Effect::SirensOn, Effect::Alert { cause }]
}

/// The siren period is over: back to the state before the trigger, without a chirp. The alert
/// stays open until a disarm.
pub fn return_from_ring(
    rt: &mut Runtime,
    s: &AlarmSettings,
    world: &World,
    back_to: Option<ArmMode>,
    now: u64,
) -> Vec<Effect> {
    match back_to {
        Some(mode) => {
            rt.enter(Phase::Armed { mode }, None, now);
            rt.open_at_arm = open_openings(s, world, mode);
        }
        None => {
            rt.enter(Phase::Disarmed, None, now);
            rt.open_at_arm.clear();
        }
    }
    rt.motion_hits.clear();
    vec![Effect::SirensOff]
}

fn arm(
    rt: &mut Runtime,
    s: &AlarmSettings,
    world: &World,
    mode: ArmMode,
    source: Source,
    now: u64,
) -> Vec<Effect> {
    match rt.phase {
        Phase::Armed { mode: current } if current == mode => return vec![],
        Phase::Arming { .. } | Phase::Pending { .. } | Phase::Triggered { .. } => return vec![],
        Phase::Disarmed | Phase::Armed { .. } => {}
    }
    rt.cause.clear();
    if source == Source::Presence || s.exit_delay_s == 0 {
        return enter_armed(rt, s, world, mode, Some(source), now);
    }
    rt.motion_hits.clear();
    rt.open_at_arm.clear();
    rt.enter(
        Phase::Arming {
            mode,
            until: now + u64::from(s.exit_delay_s),
        },
        Some(source),
        now,
    );
    vec![]
}

fn disarm(rt: &mut Runtime, source: Source, now: u64) -> Vec<Effect> {
    let mut fx = Vec::new();
    if matches!(rt.phase, Phase::Triggered { .. }) {
        fx.push(Effect::SirensOff);
    }
    if !rt.alert_ids.is_empty() {
        fx.push(Effect::ResolveAlerts);
    }
    if rt.phase != Phase::Disarmed {
        fx.push(Effect::Disarmed { source });
    }
    rt.enter(Phase::Disarmed, Some(source), now);
    rt.motion_hits.clear();
    rt.open_at_arm.clear();
    rt.cause.clear();
    rt.auto_arm_ready = rt.trackers.values().any(|p| *p == Presence::Home);
    fx
}

/// An opening event (it went on, or a watched-for-closing one went off), per its `entry` (§4.2).
fn opening(
    rt: &mut Runtime,
    s: &AlarmSettings,
    entity: &str,
    entry: Entry,
    now: u64,
) -> Vec<Effect> {
    match (rt.phase.clone(), entry) {
        (Phase::Armed { mode }, Entry::Instant) | (Phase::Pending { mode, .. }, Entry::Instant) => {
            trigger(rt, s, Some(mode), vec![entity.to_string()], None, now)
        }
        (Phase::Armed { mode }, Entry::Delayed) => {
            rt.cause = vec![entity.to_string()];
            rt.enter(
                Phase::Pending {
                    mode,
                    until: now + u64::from(s.entry_delay_s),
                },
                None,
                now,
            );
            vec![]
        }
        _ => vec![],
    }
}

fn sensor(rt: &mut Runtime, s: &AlarmSettings, entity: &str, on: bool, now: u64) -> Vec<Effect> {
    let Some(mode) = (match rt.phase {
        Phase::Armed { mode } | Phase::Pending { mode, .. } => Some(mode),
        _ => None,
    }) else {
        return vec![];
    };
    let Some(rule) = s.sensor(entity) else {
        return vec![];
    };
    if !rule.modes.contains(&mode) {
        return vec![];
    }
    match (rule.kind, on) {
        (SensorKind::Opening, true) => opening(rt, s, entity, rule.entry, now),
        (SensorKind::Opening, false) => {
            if rt.open_at_arm.remove(entity) {
                opening(rt, s, entity, rule.entry, now)
            } else {
                vec![]
            }
        }
        (SensorKind::Motion, true) => {
            if matches!(rt.phase, Phase::Pending { .. }) {
                return vec![];
            }
            rt.motion_hits.push((now, entity.to_string()));
            rt.motion_hits.retain(|(at, _)| hit_alive(*at, now, s));
            if rt.motion_hits.len() < usize::from(s.motion.count) {
                return vec![];
            }
            let mut cause: Vec<String> = Vec::new();
            for (_, e) in &rt.motion_hits {
                if !cause.contains(e) {
                    cause.push(e.clone());
                }
            }
            trigger(rt, s, Some(mode), cause, None, now)
        }
        (SensorKind::Motion, false) => vec![],
    }
}

/// A sensor first seen with a value while armed (or pending): an opening found open is watched
/// for closing (§4.3), quietly. Anything else is only recorded by the engine.
fn sensor_found(rt: &mut Runtime, s: &AlarmSettings, entity: &str, on: bool) -> Vec<Effect> {
    let Some(mode) = rt
        .mode()
        .filter(|_| matches!(rt.phase, Phase::Armed { .. } | Phase::Pending { .. }))
    else {
        return vec![];
    };
    if let Some(rule) = s.sensor(entity)
        && on
        && rule.kind == SensorKind::Opening
        && rule.modes.contains(&mode)
    {
        rt.open_at_arm.insert(entity.to_string());
    }
    vec![]
}

fn sos(rt: &mut Runtime, s: &AlarmSettings, remote: &str, now: u64) -> Vec<Effect> {
    let back_to = match rt.phase {
        // Already ringing: no second ring until it has returned to armed.
        Phase::Triggered { .. } => return vec![],
        Phase::Armed { mode } | Phase::Pending { mode, .. } => Some(mode),
        Phase::Disarmed | Phase::Arming { .. } => None,
    };
    trigger(
        rt,
        s,
        back_to,
        vec![format!("sos:{remote}")],
        Some(Source::Sos),
        now,
    )
}

/// True when the alarm is turned on with at least one tracker home: presence may arm right away
/// once everyone leaves. With everyone away it is false (§6.1, edge not level).
pub fn initial_ready(trackers: &BTreeMap<String, Presence>) -> bool {
    trackers.values().any(|p| *p == Presence::Home)
}

fn tracker(
    rt: &mut Runtime,
    s: &AlarmSettings,
    entity: &str,
    seen: Presence,
    now: u64,
) -> Vec<Effect> {
    if !s.presence.trackers.iter().any(|t| t == entity) {
        return vec![];
    }
    let previous = rt.trackers.insert(entity.to_string(), seen);
    // A homecoming needs a definite away before it; Home after Unknown after Home is a flap.
    // Any new reading of home (a tracker never seen, or one back from unknown) still means
    // someone is home: it makes presence ready to arm but does not disarm anything.
    let was_away = rt.was_away.contains(entity) || previous == Some(Presence::Away);
    let came_home = seen == Presence::Home && was_away;
    let reads_home = seen == Presence::Home && previous != Some(Presence::Home);
    match seen {
        Presence::Away => {
            rt.was_away.insert(entity.to_string());
        }
        Presence::Home => {
            rt.was_away.remove(entity);
        }
        Presence::Unknown if was_away => {
            rt.was_away.insert(entity.to_string());
        }
        Presence::Unknown => {}
    }
    let mut fx = vec![];
    if reads_home {
        rt.auto_arm_ready = true;
    }
    if came_home && s.presence.disarm && !matches!(rt.phase, Phase::Disarmed) {
        fx = disarm(rt, Source::Presence, now);
    }
    let all_away = s
        .presence
        .trackers
        .iter()
        .all(|t| rt.trackers.get(t) == Some(&Presence::Away));
    if !all_away {
        rt.everyone_away_since = None;
    } else if rt.everyone_away_since.is_none() {
        rt.everyone_away_since = Some(now);
    }
    fx
}

/// Whether presence would arm now; `next_deadline` mirrors the same conditions.
fn auto_arm_due(rt: &Runtime, s: &AlarmSettings, now: u64) -> bool {
    s.presence.arm
        && !s.presence.trackers.is_empty()
        && rt.auto_arm_ready
        && matches!(rt.phase, Phase::Disarmed)
        && rt
            .everyone_away_since
            .is_some_and(|t| t + u64::from(s.presence.arm_after_s) <= now)
}

/// Boot: picks up the retained phase after a restart (§9).
fn resume(rt: &mut Runtime, s: &AlarmSettings, world: &World, now: u64) -> Vec<Effect> {
    match rt.phase.clone() {
        Phase::Arming { mode, until } if now >= until => enter_armed(rt, s, world, mode, None, now),
        Phase::Pending { mode, until } if now >= until => {
            let cause = rt.cause.clone();
            let ring_until = until + u64::from(s.siren_s);
            if now < ring_until {
                rt.enter(
                    Phase::Triggered {
                        back_to: Some(mode),
                        until: ring_until,
                    },
                    None,
                    now,
                );
                rt.motion_hits.clear();
                vec![Effect::SirensOn, Effect::Alert { cause }]
            } else {
                // The ring was missed, the alert is still raised.
                return_from_ring(rt, s, world, Some(mode), now);
                vec![Effect::Alert { cause }]
            }
        }
        Phase::Triggered { until, .. } if until > now => vec![Effect::SirensOn],
        Phase::Triggered { back_to, .. } => return_from_ring(rt, s, world, back_to, now),
        _ => vec![],
    }
}

fn tick(rt: &mut Runtime, s: &AlarmSettings, world: &World, now: u64) -> Vec<Effect> {
    rt.motion_hits.retain(|(at, _)| hit_alive(*at, now, s));
    if auto_arm_due(rt, s, now) {
        let fx = arm(rt, s, world, ArmMode::Away, Source::Presence, now);
        rt.auto_arm_ready = false;
        return fx;
    }
    match rt.phase.clone() {
        Phase::Arming { mode, until } if now >= until => enter_armed(rt, s, world, mode, None, now),
        Phase::Pending { mode, until } if now >= until => {
            let cause = rt.cause.clone();
            trigger(rt, s, Some(mode), cause, None, now)
        }
        Phase::Triggered { back_to, until } if now >= until => {
            return_from_ring(rt, s, world, back_to, now)
        }
        _ => vec![],
    }
}

/// Applies `ev` at time `now` (seconds) and returns what the hub must do.
pub fn step(
    rt: &mut Runtime,
    s: &AlarmSettings,
    world: &World,
    ev: Event,
    now: u64,
) -> Vec<Effect> {
    // Settings drift: forget sensors that are no longer watched.
    rt.open_at_arm.retain(|e| s.sensor(e).is_some());
    rt.motion_hits.retain(|(_, e)| s.sensor(e).is_some());
    rt.trackers.retain(|e, _| s.presence.trackers.contains(e));
    rt.was_away.retain(|e| s.presence.trackers.contains(e));
    match ev {
        Event::Arm { mode, source } => arm(rt, s, world, mode, source, now),
        Event::Disarm { source } => disarm(rt, source, now),
        Event::Sensor { entity, on } => sensor(rt, s, &entity, on, now),
        Event::SensorFound { entity, on } => sensor_found(rt, s, &entity, on),
        Event::Signal { entity, signal } => match remote_action(s, &entity, &signal) {
            Some(RemoteAction::Sos) => sos(rt, s, &entity, now),
            Some(RemoteAction::Disarm) => disarm(rt, Source::Remote, now),
            Some(action) => {
                let mode = match action {
                    RemoteAction::Away => ArmMode::Away,
                    RemoteAction::Home => ArmMode::Home,
                    _ => ArmMode::Night,
                };
                arm(rt, s, world, mode, Source::Remote, now)
            }
            None => vec![],
        },
        Event::Tracker { entity, seen } => tracker(rt, s, &entity, seen, now),
        Event::Resume => resume(rt, s, world, now),
        Event::Tick => tick(rt, s, world, now),
    }
}

/// The earliest time the hub must send a `Tick`; never a time a `Tick` would not act on.
pub fn next_deadline(rt: &Runtime, s: &AlarmSettings) -> Option<u64> {
    let auto_arm = (s.presence.arm
        && !s.presence.trackers.is_empty()
        && rt.auto_arm_ready
        && matches!(rt.phase, Phase::Disarmed))
    .then(|| {
        rt.everyone_away_since
            .map(|t| t + u64::from(s.presence.arm_after_s))
    })
    .flatten();
    let hit_expiry = rt
        .motion_hits
        .iter()
        .map(|(at, _)| at + u64::from(s.motion.window_s))
        .min();
    [rt.deadline(), auto_arm, hit_expiry]
        .into_iter()
        .flatten()
        .min()
}

#[cfg(test)]
mod tests;
