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
mod tests {
    use super::*;
    use crate::doc::{Remote, SensorRule, default_sensor};

    fn settings() -> AlarmSettings {
        let all = BTreeSet::from([ArmMode::Away, ArmMode::Home, ArmMode::Night]);
        AlarmSettings {
            enabled: true,
            sensors: vec![
                SensorRule {
                    entity: "door".into(),
                    kind: SensorKind::Opening,
                    modes: all,
                    entry: Entry::Delayed,
                },
                SensorRule {
                    entity: "window".into(),
                    kind: SensorKind::Opening,
                    modes: BTreeSet::from([ArmMode::Away, ArmMode::Night]),
                    entry: Entry::Instant,
                },
                default_sensor("hall", SensorKind::Motion),
                default_sensor("kitchen", SensorKind::Motion),
            ],
            remotes: vec![Remote {
                entity: "remote.x".into(),
                signals: BTreeMap::from([("emergency".to_string(), RemoteAction::Sos)]),
            }],
            ..Default::default()
        }
    }

    const NONE: BTreeSet<String> = BTreeSet::new();

    fn step_on(
        rt: &mut Runtime,
        s: &AlarmSettings,
        on: &BTreeSet<String>,
        ev: Event,
        now: u64,
    ) -> Vec<Effect> {
        step(rt, s, &World { on }, ev, now)
    }

    fn go(rt: &mut Runtime, s: &AlarmSettings, ev: Event, now: u64) -> Vec<Effect> {
        step_on(rt, s, &NONE, ev, now)
    }

    fn sensor_ev(entity: &str, on: bool) -> Event {
        Event::Sensor {
            entity: entity.into(),
            on,
        }
    }

    fn armed(s: &AlarmSettings, mode: ArmMode) -> Runtime {
        let mut rt = Runtime::default();
        let ev = Event::Arm {
            mode,
            source: Source::Presence,
        };
        go(&mut rt, s, ev, 0);
        assert_eq!(rt.phase, Phase::Armed { mode });
        rt
    }

    #[test]
    fn manual_arm_goes_through_exit_delay() {
        let s = settings();
        let mut rt = Runtime::default();
        let ev = Event::Arm {
            mode: ArmMode::Away,
            source: Source::Command,
        };
        assert!(go(&mut rt, &s, ev, 0).is_empty());
        assert_eq!(
            rt.phase,
            Phase::Arming {
                mode: ArmMode::Away,
                until: 60
            }
        );
        let fx = go(&mut rt, &s, Event::Tick, 60);
        assert_eq!(
            rt.phase,
            Phase::Armed {
                mode: ArmMode::Away
            }
        );
        assert!(fx.contains(&Effect::Chirp));
    }

    #[test]
    fn zero_exit_delay_arms_at_once() {
        let mut s = settings();
        s.exit_delay_s = 0;
        let mut rt = Runtime::default();
        let ev = Event::Arm {
            mode: ArmMode::Home,
            source: Source::Command,
        };
        assert_eq!(
            go(&mut rt, &s, ev, 5),
            vec![
                Effect::Chirp,
                Effect::Armed {
                    mode: ArmMode::Home,
                    source: Some(Source::Command),
                    open: vec![],
                }
            ]
        );
        assert_eq!(
            rt.phase,
            Phase::Armed {
                mode: ArmMode::Home
            }
        );
    }

    #[test]
    fn presence_arm_skips_exit_delay() {
        let s = settings();
        let mut rt = Runtime::default();
        let ev = Event::Arm {
            mode: ArmMode::Away,
            source: Source::Presence,
        };
        let fx = go(&mut rt, &s, ev, 0);
        assert_eq!(
            rt.phase,
            Phase::Armed {
                mode: ArmMode::Away
            }
        );
        assert_eq!(rt.source, Some(Source::Presence));
        assert!(fx.contains(&Effect::Chirp));
    }

    #[test]
    fn arm_same_mode_is_noop_and_other_mode_rearms() {
        let s = settings();
        let mut rt = armed(&s, ArmMode::Away);
        let same = Event::Arm {
            mode: ArmMode::Away,
            source: Source::Command,
        };
        assert!(go(&mut rt, &s, same, 10).is_empty());
        assert_eq!(rt.since, 0);
        let other = Event::Arm {
            mode: ArmMode::Night,
            source: Source::Command,
        };
        go(&mut rt, &s, other, 10);
        assert!(matches!(
            rt.phase,
            Phase::Arming {
                mode: ArmMode::Night,
                until: 70
            }
        ));
    }

    #[test]
    fn arm_while_busy_is_noop() {
        let s = settings();
        let mut rt = armed(&s, ArmMode::Away);
        go(&mut rt, &s, sensor_ev("window", true), 5);
        let ev = Event::Arm {
            mode: ArmMode::Night,
            source: Source::Command,
        };
        assert!(go(&mut rt, &s, ev, 6).is_empty());
        assert!(matches!(rt.phase, Phase::Triggered { .. }));
    }

    #[test]
    fn instant_opening_triggers() {
        let s = settings();
        let mut rt = armed(&s, ArmMode::Away);
        let fx = go(&mut rt, &s, sensor_ev("window", true), 100);
        assert_eq!(
            rt.phase,
            Phase::Triggered {
                back_to: Some(ArmMode::Away),
                until: 280
            }
        );
        assert_eq!(
            fx,
            vec![
                Effect::SirensOn,
                Effect::Alert {
                    cause: vec!["window".into()]
                }
            ]
        );
    }

    #[test]
    fn delayed_opening_waits_entry_delay() {
        let s = settings();
        let mut rt = armed(&s, ArmMode::Away);
        go(&mut rt, &s, sensor_ev("door", true), 100);
        assert_eq!(
            rt.phase,
            Phase::Pending {
                mode: ArmMode::Away,
                until: 130
            }
        );
        let mut disarmed = rt.clone();
        let fx = go(
            &mut disarmed,
            &s,
            Event::Disarm {
                source: Source::Command,
            },
            110,
        );
        assert_eq!(disarmed.phase, Phase::Disarmed);
        assert!(!fx.contains(&Effect::SirensOn));
        let fx = go(&mut rt, &s, Event::Tick, 130);
        assert!(matches!(rt.phase, Phase::Triggered { .. }));
        assert_eq!(
            fx,
            vec![
                Effect::SirensOn,
                Effect::Alert {
                    cause: vec!["door".into()]
                }
            ]
        );
    }

    #[test]
    fn pending_ignores_motion_and_delayed_but_not_instant() {
        let s = settings();
        let mut rt = armed(&s, ArmMode::Away);
        go(&mut rt, &s, sensor_ev("door", true), 100);
        assert!(go(&mut rt, &s, sensor_ev("hall", true), 101).is_empty());
        assert!(go(&mut rt, &s, sensor_ev("kitchen", true), 102).is_empty());
        assert!(go(&mut rt, &s, sensor_ev("door", true), 103).is_empty());
        assert!(matches!(rt.phase, Phase::Pending { .. }));
        assert!(rt.motion_hits.is_empty());
        go(&mut rt, &s, sensor_ev("window", true), 104);
        assert!(matches!(rt.phase, Phase::Triggered { .. }));
    }

    #[test]
    fn two_motions_within_window_trigger() {
        let s = settings();
        let mut rt = armed(&s, ArmMode::Away);
        go(&mut rt, &s, sensor_ev("hall", true), 1000);
        let fx = go(&mut rt, &s, sensor_ev("kitchen", true), 1119);
        assert!(matches!(rt.phase, Phase::Triggered { .. }));
        assert_eq!(
            fx[1],
            Effect::Alert {
                cause: vec!["hall".into(), "kitchen".into()]
            }
        );

        let mut rt = armed(&s, ArmMode::Away);
        go(&mut rt, &s, sensor_ev("hall", true), 1000);
        go(&mut rt, &s, sensor_ev("kitchen", true), 1121);
        assert_eq!(
            rt.phase,
            Phase::Armed {
                mode: ArmMode::Away
            }
        );
        assert_eq!(rt.motion_hits, vec![(1121, "kitchen".to_string())]);
    }

    #[test]
    fn same_motion_sensor_twice_counts() {
        let s = settings();
        let mut rt = armed(&s, ArmMode::Away);
        go(&mut rt, &s, sensor_ev("hall", true), 10);
        go(&mut rt, &s, sensor_ev("hall", false), 20);
        let fx = go(&mut rt, &s, sensor_ev("hall", true), 30);
        assert!(matches!(rt.phase, Phase::Triggered { .. }));
        assert_eq!(
            fx[1],
            Effect::Alert {
                cause: vec!["hall".into()]
            }
        );
    }

    #[test]
    fn mode_filters_sensors() {
        let s = settings();
        let mut rt = armed(&s, ArmMode::Night);
        go(&mut rt, &s, sensor_ev("hall", true), 10);
        go(&mut rt, &s, sensor_ev("hall", true), 20);
        assert_eq!(
            rt.phase,
            Phase::Armed {
                mode: ArmMode::Night
            }
        );
        let mut rt = armed(&s, ArmMode::Home);
        assert!(go(&mut rt, &s, sensor_ev("window", true), 10).is_empty());
    }

    #[test]
    fn open_window_at_arm_is_watched_for_closing() {
        let s = settings();
        let on = BTreeSet::from(["window".to_string()]);
        let mut rt = Runtime::default();
        let ev = Event::Arm {
            mode: ArmMode::Away,
            source: Source::Presence,
        };
        let fx = step_on(&mut rt, &s, &on, ev, 0);
        assert_eq!(rt.open_at_arm, BTreeSet::from(["window".to_string()]));
        assert_eq!(
            fx,
            vec![
                Effect::Chirp,
                Effect::Armed {
                    mode: ArmMode::Away,
                    source: Some(Source::Presence),
                    open: vec!["window".into()],
                }
            ]
        );
        let fx = go(&mut rt, &s, sensor_ev("window", false), 50);
        assert!(rt.open_at_arm.is_empty());
        assert_eq!(
            fx[1],
            Effect::Alert {
                cause: vec!["window".into()]
            }
        );
    }

    #[test]
    fn closing_during_exit_delay_is_not_an_event() {
        let s = settings();
        let on = BTreeSet::from(["window".to_string()]);
        let mut rt = Runtime::default();
        let ev = Event::Arm {
            mode: ArmMode::Away,
            source: Source::Command,
        };
        step_on(&mut rt, &s, &on, ev, 0);
        assert!(go(&mut rt, &s, sensor_ev("window", false), 30).is_empty());
        let fx = go(&mut rt, &s, Event::Tick, 60);
        assert_eq!(
            rt.phase,
            Phase::Armed {
                mode: ArmMode::Away
            }
        );
        assert!(rt.open_at_arm.is_empty());
        assert_eq!(
            fx,
            vec![
                Effect::Chirp,
                Effect::Armed {
                    mode: ArmMode::Away,
                    source: Some(Source::Command),
                    open: vec![],
                }
            ]
        );
    }

    #[test]
    fn ring_ends_and_rearms() {
        let s = settings();
        let mut rt = armed(&s, ArmMode::Away);
        go(&mut rt, &s, sensor_ev("window", true), 100);
        rt.alert_ids = vec!["alarm-1".into()];
        let on = BTreeSet::new();
        let fx = step_on(&mut rt, &s, &on, Event::Tick, 280);
        assert_eq!(fx, vec![Effect::SirensOff]);
        assert_eq!(
            rt.phase,
            Phase::Armed {
                mode: ArmMode::Away
            }
        );
        assert_eq!(rt.alert_ids, vec!["alarm-1".to_string()]);
        assert!(rt.motion_hits.is_empty());
        let fx = go(&mut rt, &s, sensor_ev("window", true), 300);
        assert_eq!(fx[0], Effect::SirensOn);
        assert!(matches!(fx[1], Effect::Alert { .. }));
        assert!(matches!(rt.phase, Phase::Triggered { .. }));
    }

    #[test]
    fn ring_end_watches_what_is_still_open() {
        let s = settings();
        let mut rt = armed(&s, ArmMode::Away);
        go(&mut rt, &s, sensor_ev("window", true), 100);
        let on = BTreeSet::from(["window".to_string()]);
        step_on(&mut rt, &s, &on, Event::Tick, 280);
        assert_eq!(rt.open_at_arm, BTreeSet::from(["window".to_string()]));
    }

    #[test]
    fn detections_while_ringing_are_ignored() {
        let s = settings();
        let mut rt = armed(&s, ArmMode::Away);
        go(&mut rt, &s, sensor_ev("window", true), 100);
        let before = rt.clone();
        assert!(go(&mut rt, &s, sensor_ev("hall", true), 110).is_empty());
        assert!(go(&mut rt, &s, sensor_ev("window", true), 111).is_empty());
        assert_eq!(rt, before);
    }

    #[test]
    fn sos_from_disarmed_returns_to_disarmed() {
        let s = settings();
        let mut rt = Runtime::default();
        let ev = Event::Signal {
            entity: "remote.x".into(),
            signal: "emergency".into(),
        };
        let fx = go(&mut rt, &s, ev, 0);
        assert_eq!(
            rt.phase,
            Phase::Triggered {
                back_to: None,
                until: 180
            }
        );
        assert_eq!(
            fx,
            vec![
                Effect::SirensOn,
                Effect::Alert {
                    cause: vec!["sos:remote.x".into()]
                }
            ]
        );
        assert_eq!(go(&mut rt, &s, Event::Tick, 180), vec![Effect::SirensOff]);
        assert_eq!(rt.phase, Phase::Disarmed);
    }

    #[test]
    fn sos_from_armed_returns_to_armed_and_unmapped_signal_is_ignored() {
        let s = settings();
        let mut rt = armed(&s, ArmMode::Night);
        let unmapped = Event::Signal {
            entity: "remote.x".into(),
            signal: "other".into(),
        };
        assert!(go(&mut rt, &s, unmapped, 5).is_empty());
        let ev = Event::Signal {
            entity: "remote.x".into(),
            signal: "emergency".into(),
        };
        go(&mut rt, &s, ev, 10);
        assert!(matches!(
            rt.phase,
            Phase::Triggered {
                back_to: Some(ArmMode::Night),
                ..
            }
        ));
        assert_eq!(rt.source, Some(Source::Sos));
    }

    #[test]
    fn double_disarm_is_harmless() {
        let s = settings();
        let mut rt = armed(&s, ArmMode::Away);
        go(&mut rt, &s, sensor_ev("window", true), 10);
        rt.alert_ids = vec!["alarm-1".into()];
        let ev = Event::Disarm {
            source: Source::Command,
        };
        assert_eq!(
            go(&mut rt, &s, ev.clone(), 20),
            vec![
                Effect::SirensOff,
                Effect::ResolveAlerts,
                Effect::Disarmed {
                    source: Source::Command
                }
            ]
        );
        assert_eq!(rt.phase, Phase::Disarmed);
        assert!(rt.cause.is_empty());
        rt.alert_ids.clear();
        assert!(go(&mut rt, &s, ev, 30).is_empty());
    }

    #[test]
    fn disarm_sets_auto_arm_ready_from_trackers() {
        let mut s = settings();
        s.presence.trackers = vec!["tracker.a".into()];
        let mut rt = armed(&s, ArmMode::Away);
        rt.trackers.insert("tracker.a".into(), Presence::Home);
        let ev = Event::Disarm {
            source: Source::Command,
        };
        go(&mut rt, &s, ev, 5);
        assert!(rt.auto_arm_ready);
    }

    #[test]
    fn removed_sensor_is_forgotten() {
        let s = settings();
        let mut rt = armed(&s, ArmMode::Away);
        rt.open_at_arm.insert("window".into());
        rt.motion_hits.push((0, "hall".into()));
        let bare = AlarmSettings {
            enabled: true,
            ..Default::default()
        };
        go(&mut rt, &bare, Event::Tick, 1);
        assert!(rt.open_at_arm.is_empty());
        assert!(rt.motion_hits.is_empty());
        assert!(go(&mut rt, &bare, sensor_ev("window", true), 2).is_empty());
    }

    #[test]
    fn state_names() {
        let mut rt = Runtime::default();
        assert_eq!(rt.state_name(), "disarmed");
        let m = ArmMode::Home;
        for (phase, name) in [
            (Phase::Arming { mode: m, until: 1 }, "arming"),
            (Phase::Armed { mode: m }, "armed_home"),
            (
                Phase::Armed {
                    mode: ArmMode::Away,
                },
                "armed_away",
            ),
            (
                Phase::Armed {
                    mode: ArmMode::Night,
                },
                "armed_night",
            ),
            (Phase::Pending { mode: m, until: 1 }, "pending"),
            (
                Phase::Triggered {
                    back_to: None,
                    until: 1,
                },
                "triggered",
            ),
        ] {
            rt.phase = phase;
            assert_eq!(rt.state_name(), name);
        }
    }

    #[test]
    fn deadlines_follow_the_phase() {
        let s = settings();
        let mut rt = armed(&s, ArmMode::Away);
        assert_eq!(next_deadline(&rt, &s), None);
        go(&mut rt, &s, sensor_ev("hall", true), 100);
        assert_eq!(next_deadline(&rt, &s), Some(220));
        go(&mut rt, &s, sensor_ev("door", true), 105);
        assert_eq!(rt.deadline(), Some(135));
        assert_eq!(next_deadline(&rt, &s), Some(135));
    }

    #[test]
    fn next_deadline_moves_forward_after_a_tick() {
        let s = settings();
        let mut rt = armed(&s, ArmMode::Away);
        go(&mut rt, &s, sensor_ev("hall", true), 100);
        let at = next_deadline(&rt, &s).unwrap();
        go(&mut rt, &s, Event::Tick, at);
        assert!(rt.motion_hits.is_empty());
        assert_eq!(next_deadline(&rt, &s), None);

        // Same through a phase deadline with a stale hit alongside.
        let mut rt = armed(&s, ArmMode::Away);
        go(&mut rt, &s, sensor_ev("hall", true), 100);
        go(&mut rt, &s, sensor_ev("door", true), 110);
        for _ in 0..4 {
            let Some(at) = next_deadline(&rt, &s) else {
                return;
            };
            go(&mut rt, &s, Event::Tick, at);
            assert!(next_deadline(&rt, &s).is_none_or(|d| d > at));
        }
    }

    #[test]
    fn auto_arm_deadline_only_when_it_could_fire() {
        let mut s = settings();
        let mut rt = Runtime {
            auto_arm_ready: true,
            everyone_away_since: Some(1000),
            ..Default::default()
        };
        assert_eq!(next_deadline(&rt, &s), None, "no trackers");
        s.presence.trackers = vec!["tracker.a".into()];
        assert_eq!(next_deadline(&rt, &s), Some(1120));
        rt.auto_arm_ready = false;
        assert_eq!(next_deadline(&rt, &s), None);
        rt.auto_arm_ready = true;
        s.presence.arm = false;
        assert_eq!(next_deadline(&rt, &s), None);
        s.presence.arm = true;
        rt.phase = Phase::Armed {
            mode: ArmMode::Away,
        };
        assert_eq!(next_deadline(&rt, &s), None);
    }

    fn trackers_settings() -> AlarmSettings {
        let mut s = settings();
        s.presence.trackers = vec!["tracker.a".into(), "tracker.b".into()];
        s.remotes[0].signals.extend([
            ("arm_all_zones".to_string(), RemoteAction::Away),
            ("arm_day_zones".to_string(), RemoteAction::Home),
            ("arm_night_zones".to_string(), RemoteAction::Night),
            ("disarm".to_string(), RemoteAction::Disarm),
        ]);
        s
    }

    fn seen(entity: &str, seen: Presence) -> Event {
        Event::Tracker {
            entity: entity.into(),
            seen,
        }
    }

    fn ready_runtime() -> Runtime {
        Runtime {
            auto_arm_ready: true,
            ..Default::default()
        }
    }

    #[test]
    fn everyone_away_arms_after_grace() {
        let s = trackers_settings();
        let mut rt = ready_runtime();
        let t = 1000;
        go(&mut rt, &s, seen("tracker.a", Presence::Away), t);
        assert_eq!(rt.everyone_away_since, None);
        go(&mut rt, &s, seen("tracker.b", Presence::Away), t + 10);
        assert_eq!(rt.everyone_away_since, Some(t + 10));
        assert_eq!(next_deadline(&rt, &s), Some(t + 130));
        assert!(go(&mut rt, &s, Event::Tick, t + 129).is_empty());
        assert_eq!(rt.phase, Phase::Disarmed);
        let fx = go(&mut rt, &s, Event::Tick, t + 130);
        assert_eq!(
            rt.phase,
            Phase::Armed {
                mode: ArmMode::Away
            }
        );
        assert_eq!(rt.source, Some(Source::Presence));
        assert_eq!(
            fx,
            vec![
                Effect::Chirp,
                Effect::Armed {
                    mode: ArmMode::Away,
                    source: Some(Source::Presence),
                    open: vec![],
                }
            ]
        );
        assert!(!rt.auto_arm_ready);
        assert_eq!(next_deadline(&rt, &s), None);
    }

    #[test]
    fn unknown_tracker_blocks_auto_arm() {
        let s = trackers_settings();
        let mut rt = ready_runtime();
        go(&mut rt, &s, seen("tracker.a", Presence::Away), 0);
        go(&mut rt, &s, seen("tracker.b", Presence::Unknown), 0);
        go(&mut rt, &s, Event::Tick, 1000);
        assert_eq!(rt.phase, Phase::Disarmed);
        go(&mut rt, &s, seen("tracker.b", Presence::Away), 2000);
        go(&mut rt, &s, Event::Tick, 2119);
        assert_eq!(rt.phase, Phase::Disarmed);
        go(&mut rt, &s, Event::Tick, 2120);
        assert!(matches!(rt.phase, Phase::Armed { .. }));
    }

    #[test]
    fn disarm_while_away_stays_disarmed() {
        let s = trackers_settings();
        let mut rt = ready_runtime();
        go(&mut rt, &s, seen("tracker.a", Presence::Away), 0);
        go(&mut rt, &s, seen("tracker.b", Presence::Away), 0);
        go(&mut rt, &s, Event::Tick, 120);
        assert!(matches!(rt.phase, Phase::Armed { .. }));
        let ev = Event::Disarm {
            source: Source::Command,
        };
        go(&mut rt, &s, ev, 200);
        assert!(!rt.auto_arm_ready);
        assert_eq!(next_deadline(&rt, &s), None);
        for t in (300..3900).step_by(60) {
            go(&mut rt, &s, Event::Tick, t);
        }
        assert_eq!(rt.phase, Phase::Disarmed);
        go(&mut rt, &s, seen("tracker.a", Presence::Home), 4000);
        assert!(rt.auto_arm_ready);
        go(&mut rt, &s, seen("tracker.a", Presence::Away), 4100);
        go(&mut rt, &s, Event::Tick, 4219);
        assert_eq!(rt.phase, Phase::Disarmed);
        go(&mut rt, &s, Event::Tick, 4220);
        assert!(matches!(rt.phase, Phase::Armed { .. }));
    }

    #[test]
    fn enable_with_everyone_away_does_not_arm() {
        let away = BTreeMap::from([
            ("tracker.a".to_string(), Presence::Away),
            ("tracker.b".to_string(), Presence::Away),
        ]);
        assert!(!initial_ready(&away));
        let mixed = BTreeMap::from([
            ("tracker.a".to_string(), Presence::Away),
            ("tracker.b".to_string(), Presence::Home),
        ]);
        assert!(initial_ready(&mixed));
        assert!(!initial_ready(&BTreeMap::new()));
    }

    #[test]
    fn tracker_home_disarms() {
        let s = trackers_settings();
        let mut rt = armed(&s, ArmMode::Away);
        go(&mut rt, &s, seen("tracker.a", Presence::Away), 5);
        go(&mut rt, &s, sensor_ev("window", true), 10);
        rt.alert_ids = vec!["alarm-1".into()];
        let fx = go(&mut rt, &s, seen("tracker.a", Presence::Home), 20);
        assert_eq!(
            fx,
            vec![
                Effect::SirensOff,
                Effect::ResolveAlerts,
                Effect::Disarmed {
                    source: Source::Presence
                }
            ]
        );
        assert_eq!(rt.phase, Phase::Disarmed);
        assert_eq!(rt.source, Some(Source::Presence));
        assert!(rt.auto_arm_ready);
    }

    #[test]
    fn tracker_home_noop_when_already_home() {
        let s = trackers_settings();
        let mut rt = Runtime::default();
        go(&mut rt, &s, seen("tracker.a", Presence::Home), 0);
        let ev = Event::Arm {
            mode: ArmMode::Night,
            source: Source::Command,
        };
        go(&mut rt, &s, ev, 1);
        go(&mut rt, &s, Event::Tick, 61);
        assert_eq!(
            rt.phase,
            Phase::Armed {
                mode: ArmMode::Night
            }
        );
        assert!(go(&mut rt, &s, seen("tracker.a", Presence::Home), 70).is_empty());
        assert_eq!(
            rt.phase,
            Phase::Armed {
                mode: ArmMode::Night
            }
        );
    }

    #[test]
    fn home_after_unknown_makes_presence_ready_without_disarming() {
        let s = trackers_settings();
        let mut rt = Runtime::default();
        go(&mut rt, &s, seen("tracker.a", Presence::Home), 0);
        go(&mut rt, &s, seen("tracker.b", Presence::Away), 0);
        go(&mut rt, &s, seen("tracker.a", Presence::Unknown), 10);
        let ev = Event::Disarm {
            source: Source::Command,
        };
        go(&mut rt, &s, ev, 20);
        assert!(!rt.auto_arm_ready, "nobody reads home at the disarm");
        // The phone comes back online at home: not a homecoming, but someone is home.
        assert!(go(&mut rt, &s, seen("tracker.a", Presence::Home), 30).is_empty());
        assert!(rt.auto_arm_ready);
        go(&mut rt, &s, seen("tracker.a", Presence::Away), 100);
        go(&mut rt, &s, Event::Tick, 220);
        assert!(matches!(rt.phase, Phase::Armed { .. }));
    }

    #[test]
    fn unlisted_tracker_is_ignored_and_stale_ones_dropped() {
        let s = trackers_settings();
        let mut rt = Runtime::default();
        go(&mut rt, &s, seen("tracker.zzz", Presence::Home), 0);
        assert!(rt.trackers.is_empty());
        rt.trackers.insert("tracker.old".into(), Presence::Home);
        go(&mut rt, &s, Event::Tick, 1);
        assert!(rt.trackers.is_empty());
    }

    #[test]
    fn presence_switches_off() {
        let mut s = trackers_settings();
        s.presence.arm = false;
        let mut rt = ready_runtime();
        go(&mut rt, &s, seen("tracker.a", Presence::Away), 0);
        go(&mut rt, &s, seen("tracker.b", Presence::Away), 0);
        assert_eq!(next_deadline(&rt, &s), None);
        go(&mut rt, &s, Event::Tick, 10_000);
        assert_eq!(rt.phase, Phase::Disarmed);

        let mut s = trackers_settings();
        s.presence.disarm = false;
        let mut rt = armed(&s, ArmMode::Away);
        go(&mut rt, &s, seen("tracker.a", Presence::Home), 5);
        assert_eq!(
            rt.phase,
            Phase::Armed {
                mode: ArmMode::Away
            }
        );
        assert!(rt.auto_arm_ready);
    }

    #[test]
    fn remote_signals_map_to_actions() {
        let s = trackers_settings();
        let sig = |signal: &str| Event::Signal {
            entity: "remote.x".into(),
            signal: signal.into(),
        };
        let mut rt = Runtime::default();
        go(&mut rt, &s, sig("arm_all_zones"), 0);
        assert_eq!(
            rt.phase,
            Phase::Arming {
                mode: ArmMode::Away,
                until: 60
            }
        );
        assert_eq!(rt.source, Some(Source::Remote));
        go(&mut rt, &s, sig("disarm"), 5);
        assert_eq!(rt.phase, Phase::Disarmed);
        go(&mut rt, &s, sig("arm_day_zones"), 10);
        assert!(matches!(
            rt.phase,
            Phase::Arming {
                mode: ArmMode::Home,
                ..
            }
        ));
        go(&mut rt, &s, sig("disarm"), 11);
        go(&mut rt, &s, sig("arm_night_zones"), 12);
        assert!(matches!(
            rt.phase,
            Phase::Arming {
                mode: ArmMode::Night,
                ..
            }
        ));
        let before = rt.clone();
        assert!(go(&mut rt, &s, sig("nope"), 13).is_empty());
        let unknown = Event::Signal {
            entity: "remote.other".into(),
            signal: "disarm".into(),
        };
        assert!(go(&mut rt, &s, unknown, 14).is_empty());
        assert_eq!(rt, before);
    }

    #[test]
    fn resume_cases() {
        let s = settings();
        let away = ArmMode::Away;
        let with = |phase: Phase| Runtime {
            phase,
            cause: vec!["door".into()],
            ..Default::default()
        };
        // Arming past its end.
        let mut rt = with(Phase::Arming {
            mode: away,
            until: 100,
        });
        assert_eq!(
            go(&mut rt, &s, Event::Resume, 150),
            vec![
                Effect::Chirp,
                Effect::Armed {
                    mode: ArmMode::Away,
                    source: None,
                    open: vec![],
                }
            ]
        );
        assert_eq!(rt.phase, Phase::Armed { mode: away });
        // Arming / Pending not yet due: the hub resumes the countdown.
        let mut rt = with(Phase::Arming {
            mode: away,
            until: 100,
        });
        assert!(go(&mut rt, &s, Event::Resume, 50).is_empty());
        let mut rt = with(Phase::Pending {
            mode: away,
            until: 100,
        });
        assert!(go(&mut rt, &s, Event::Resume, 50).is_empty());
        assert!(matches!(rt.phase, Phase::Pending { .. }));
        // Pending past its end, ring still due.
        let alert = Effect::Alert {
            cause: vec!["door".into()],
        };
        let mut rt = with(Phase::Pending {
            mode: away,
            until: 100,
        });
        assert_eq!(
            go(&mut rt, &s, Event::Resume, 200),
            vec![Effect::SirensOn, alert.clone()]
        );
        assert_eq!(
            rt.phase,
            Phase::Triggered {
                back_to: Some(away),
                until: 280
            }
        );
        // Pending deadline missed long ago: re-armed, alert still raised, no chirp.
        let mut rt = with(Phase::Pending {
            mode: away,
            until: 100,
        });
        assert_eq!(go(&mut rt, &s, Event::Resume, 280), vec![alert]);
        assert_eq!(rt.phase, Phase::Armed { mode: away });
        // Triggered still ringing.
        let mut rt = with(Phase::Triggered {
            back_to: Some(away),
            until: 300,
        });
        assert_eq!(go(&mut rt, &s, Event::Resume, 200), vec![Effect::SirensOn]);
        assert!(matches!(rt.phase, Phase::Triggered { .. }));
        // Triggered over.
        assert_eq!(go(&mut rt, &s, Event::Resume, 300), vec![Effect::SirensOff]);
        assert_eq!(rt.phase, Phase::Armed { mode: away });
        let mut rt = with(Phase::Triggered {
            back_to: None,
            until: 10,
        });
        go(&mut rt, &s, Event::Resume, 20);
        assert_eq!(rt.phase, Phase::Disarmed);
        // Idle phases.
        assert!(go(&mut rt, &s, Event::Resume, 30).is_empty());
        let mut rt = with(Phase::Armed { mode: away });
        assert!(go(&mut rt, &s, Event::Resume, 30).is_empty());
    }

    #[test]
    fn auto_arm_deadline_is_acted_on_by_a_tick() {
        let s = trackers_settings();
        let mut rt = ready_runtime();
        go(&mut rt, &s, seen("tracker.a", Presence::Away), 0);
        go(&mut rt, &s, seen("tracker.b", Presence::Away), 0);
        let at = next_deadline(&rt, &s).unwrap();
        go(&mut rt, &s, Event::Tick, at);
        assert!(matches!(rt.phase, Phase::Armed { .. }));
        assert!(next_deadline(&rt, &s).is_none_or(|d| d > at));
    }

    #[test]
    fn runtime_roundtrips_json() {
        let rt = Runtime {
            phase: Phase::Triggered {
                back_to: Some(ArmMode::Away),
                until: 280,
            },
            source: Some(Source::Remote),
            since: 100,
            motion_hits: vec![(90, "hall".into())],
            open_at_arm: BTreeSet::from(["window".to_string()]),
            cause: vec!["window".into()],
            alert_ids: vec!["alarm-1".into()],
            trackers: BTreeMap::from([("tracker.a".to_string(), Presence::Away)]),
            was_away: BTreeSet::from(["tracker.a".to_string()]),
            auto_arm_ready: true,
            everyone_away_since: Some(5),
        };
        let json = serde_json::to_string(&rt).unwrap();
        assert!(json.contains(r#""phase":"triggered""#));
        assert_eq!(serde_json::from_str::<Runtime>(&json).unwrap(), rt);
    }

    fn found_ev(entity: &str, on: bool) -> Event {
        Event::SensorFound {
            entity: entity.into(),
            on,
        }
    }

    #[test]
    fn found_open_opening_is_watched_for_closing_without_trigger() {
        let s = settings();
        let mut rt = armed(&s, ArmMode::Away);
        let fx = go(&mut rt, &s, found_ev("door", true), 10);
        assert!(fx.is_empty());
        assert_eq!(
            rt.phase,
            Phase::Armed {
                mode: ArmMode::Away
            }
        );
        assert_eq!(rt.open_at_arm, BTreeSet::from(["door".to_string()]));
        // Closing it later triggers per its entry (delayed: pending).
        go(&mut rt, &s, sensor_ev("door", false), 20);
        assert!(matches!(rt.phase, Phase::Pending { .. }));
    }

    #[test]
    fn found_open_instant_opening_triggers_on_closing() {
        let s = settings();
        let mut rt = armed(&s, ArmMode::Away);
        assert!(go(&mut rt, &s, found_ev("window", true), 10).is_empty());
        assert!(matches!(rt.phase, Phase::Armed { .. }));
        let fx = go(&mut rt, &s, sensor_ev("window", false), 20);
        assert!(matches!(rt.phase, Phase::Triggered { .. }));
        assert!(fx.contains(&Effect::SirensOn));
    }

    #[test]
    fn found_off_and_found_motion_and_unwatched_modes_do_nothing() {
        let s = settings();
        let mut rt = armed(&s, ArmMode::Home);
        let before = rt.clone();
        go(&mut rt, &s, found_ev("door", false), 10);
        go(&mut rt, &s, found_ev("hall", true), 10);
        // The window is not watched in home mode.
        go(&mut rt, &s, found_ev("window", true), 10);
        go(&mut rt, &s, found_ev("unknown", true), 10);
        assert_eq!(rt, before);
        let mut away = armed(&s, ArmMode::Away);
        go(&mut away, &s, found_ev("hall", true), 10);
        assert!(away.motion_hits.is_empty());
        let mut off = Runtime::default();
        go(&mut off, &s, found_ev("door", true), 10);
        assert!(off.open_at_arm.is_empty());
    }

    #[test]
    fn home_unknown_home_is_not_a_homecoming() {
        let s = trackers_settings();
        let mut rt = Runtime::default();
        go(&mut rt, &s, seen("tracker.a", Presence::Home), 0);
        let ev = Event::Arm {
            mode: ArmMode::Night,
            source: Source::Command,
        };
        go(&mut rt, &s, ev, 1);
        go(&mut rt, &s, Event::Tick, 61);
        go(&mut rt, &s, seen("tracker.a", Presence::Unknown), 70);
        assert!(go(&mut rt, &s, seen("tracker.a", Presence::Home), 80).is_empty());
        assert_eq!(
            rt.phase,
            Phase::Armed {
                mode: ArmMode::Night
            }
        );
    }

    #[test]
    fn away_unknown_home_disarms() {
        let s = trackers_settings();
        let mut rt = Runtime::default();
        go(&mut rt, &s, seen("tracker.a", Presence::Away), 0);
        go(&mut rt, &s, seen("tracker.a", Presence::Unknown), 5);
        let mut armed_rt = armed(&s, ArmMode::Away);
        armed_rt.trackers = rt.trackers.clone();
        armed_rt.was_away = rt.was_away.clone();
        go(&mut armed_rt, &s, seen("tracker.a", Presence::Home), 10);
        assert_eq!(armed_rt.phase, Phase::Disarmed);
        assert!(armed_rt.auto_arm_ready);
    }

    #[test]
    fn never_seen_home_sets_ready_without_disarming() {
        let s = trackers_settings();
        let mut rt = armed(&s, ArmMode::Away);
        assert!(!rt.auto_arm_ready);
        assert!(go(&mut rt, &s, seen("tracker.a", Presence::Home), 10).is_empty());
        assert!(matches!(rt.phase, Phase::Armed { .. }));
        assert!(rt.auto_arm_ready);
    }
}
