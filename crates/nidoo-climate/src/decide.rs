//! The decision tree (spec §5.1, §5.4, §7): from a zone's inputs to what each device should do.
//!
//! Pure and deterministic. The hub gathers the inputs, calls [`decide`], and commands the devices.

use std::collections::BTreeMap;

use chrono::{Duration, NaiveDateTime};

use crate::doc::{
    Decision, DeviceTarget, HouseSettings, Mode, Override, RoomEco, SafetyCause, Setpoint, Source,
    ZoneSettings, ZoneState,
};
use crate::feedback::{CorrectionInput, FeedbackState, correct};
use crate::schedule::{Reading, ReadingSource};

/// Step used when a device does not advertise one.
const DEFAULT_STEP: f64 = 0.5;

/// What a climate device can do.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceCaps {
    pub hvac_modes: Vec<String>,
    /// The fan modes the device lists, empty without fan control.
    pub fan_modes: Vec<String>,
    pub min_temp: Option<f64>,
    pub max_temp: Option<f64>,
    pub temp_step: Option<f64>,
}

/// A climate device and its current target and mode.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub caps: DeviceCaps,
    pub target: Option<f64>,
    pub mode: Option<String>,
    pub fan_mode: Option<String>,
}

/// What the engine found on the devices before a safety action, to put back when it clears.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SafetyMemory {
    pub found: BTreeMap<String, DeviceTarget>,
}

impl SafetyMemory {
    /// Record each device's current target and mode.
    pub fn capture(devices: &[Device]) -> SafetyMemory {
        let found = devices
            .iter()
            .filter_map(|d| {
                Some((
                    d.id.clone(),
                    DeviceTarget {
                        setpoint: d.target,
                        mode: d.mode.clone()?,
                        held: false,
                    },
                ))
            })
            .collect();
        SafetyMemory { found }
    }
}

/// Everything `decide` reads for one zone.
pub struct ZoneInputs<'a> {
    pub zone: &'a str,
    pub settings: &'a ZoneSettings,
    pub house: &'a HouseSettings,
    pub mode: Option<Mode>,
    pub reading: Reading,
    pub outdoor_mean: Option<f64>,
    pub home: bool,
    pub holiday: bool,
    pub overrides: &'a [Override],
    pub devices: &'a [Device],
    pub local_now: NaiveDateTime,
    pub now: u64,
    pub safety_memory: Option<&'a SafetyMemory>,
    /// The cause of the zone's previous safety decision, so a limit that was crossed holds
    /// until the reading is clearly back inside it.
    pub previous_cause: Option<SafetyCause>,
    /// The zone's own eco, if set; one whose `until` has passed is ignored.
    pub room_eco: Option<&'a RoomEco>,
    /// The room thermometer's own value and age, whatever `reading` says.
    pub thermometer: Option<ReadingSource>,
    /// The thermometer correction's state from the previous tick.
    pub feedback: Option<&'a FeedbackState>,
    /// Nobody has been detected in the room for its presence settings' `empty_after`.
    pub room_empty: bool,
}

/// How far back inside the overheat or overcool limit the reading must be before that latched
/// cause clears.
pub const SAFETY_HYSTERESIS: f64 = 0.5;

/// How far above the frost limit frost protection heats, and so where a latched frost cause
/// clears.
pub const FROST_MARGIN: f64 = 2.0;

/// Clamp to the device's range and round to its step (0.5 when it has none).
pub fn fit(t: f64, caps: &DeviceCaps) -> f64 {
    let step = caps.temp_step.filter(|s| *s > 0.0).unwrap_or(DEFAULT_STEP);
    let mut v = (t / step).round() * step;
    if let Some(max) = caps.max_temp {
        v = v.min(max);
    }
    if let Some(min) = caps.min_temp {
        v = v.max(min);
    }
    v
}

pub(crate) fn mode_str(mode: Mode) -> &'static str {
    match mode {
        Mode::Heat => "heat",
        Mode::Cool => "cool",
    }
}

fn can(caps: &DeviceCaps, mode: &str) -> bool {
    caps.hvac_modes.iter().any(|m| m == mode)
}

/// What a device should be told to run `setpoint` in `mode`, or `None` when it cannot.
pub fn device_target(setpoint: Setpoint, mode: Mode, caps: &DeviceCaps) -> Option<DeviceTarget> {
    let m = mode_str(mode);
    match setpoint {
        Setpoint::Temp(t) => can(caps, m).then(|| DeviceTarget {
            setpoint: Some(fit(t, caps)),
            mode: m.to_string(),
            held: false,
        }),
        Setpoint::Off if can(caps, m) && can(caps, "off") => Some(DeviceTarget {
            setpoint: None,
            mode: "off".to_string(),
            held: false,
        }),
        Setpoint::Off => {
            let limit = match mode {
                Mode::Heat => caps.min_temp,
                Mode::Cool => caps.max_temp,
            }?;
            can(caps, m).then(|| DeviceTarget {
                setpoint: Some(fit(limit, caps)),
                mode: m.to_string(),
                held: false,
            })
        }
    }
}

fn temp(t: f64) -> String {
    format!("{t:.1} °C")
}

fn setpoint_text(sp: Setpoint) -> String {
    match sp {
        Setpoint::Temp(t) => temp(t),
        Setpoint::Off => "off".to_string(),
    }
}

/// The devices that can run `setpoint` in `mode`, with their targets.
fn targets(devices: &[Device], setpoint: Setpoint, mode: Mode) -> BTreeMap<String, DeviceTarget> {
    devices
        .iter()
        .filter_map(|d| Some((d.id.clone(), device_target(setpoint, mode, &d.caps)?)))
        .collect()
}

fn base_decision(inp: &ZoneInputs, source: Source, reason: String) -> Decision {
    let (reading, reading_age_s) = match inp.reading {
        Reading::Fresh { value, age_s, .. } => (Some(value), Some(age_s)),
        Reading::Stale { last } => (last, None),
        Reading::Missing => (None, None),
    };
    Decision {
        zone: inp.zone.to_string(),
        mode: inp.mode,
        setpoint: None,
        source,
        cause: None,
        reason,
        reading,
        reading_age_s,
        outdoor_mean: inp.outdoor_mean,
        home: inp.home,
        holiday: inp.holiday,
        applied: false,
        at: inp.now,
        entities: BTreeMap::new(),
        eco_restore: BTreeMap::new(),
        adjust: None,
    }
}

fn safety_decision(inp: &ZoneInputs, cause: SafetyCause, reason: String) -> Decision {
    let mut d = base_decision(inp, Source::Safety, reason);
    d.cause = Some(cause);
    d
}

/// Decide what the zone's devices should do now. `applied` is left `false` for the hub to set.
pub fn decide(inp: &ZoneInputs) -> Decision {
    let mut d = if inp.settings.state == ZoneState::Off {
        base_decision(inp, Source::Idle, "Zone is off".to_string())
    } else {
        safety(inp)
            .or_else(|| pending(inp))
            .or_else(|| zone_override(inp))
            .unwrap_or_else(|| rules(inp))
    };
    if matches!(
        inp.reading,
        Reading::Fresh {
            sensor_stale: true,
            ..
        }
    ) && inp.settings.state != ZoneState::Off
    {
        // The limit, not the age, so the reason stays the same from one pass to the next.
        let limit_min = inp
            .house
            .stale_after_min
            .max(inp.settings.feedback.quiet_limit_min);
        d.reason.push_str(&format!(
            "; the room thermometer has been silent for over {}, using the devices' own readings",
            duration(limit_min)
        ));
    }
    d
}

/// A duration in minutes as "45 min", "6 h" or "6 h 10 min".
fn duration(min: u32) -> String {
    match (min / 60, min % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

fn safety(inp: &ZoneInputs) -> Option<Decision> {
    let house = inp.house;
    match inp.reading {
        Reading::Fresh { value, .. }
            if value < house.frost
                || (latched(inp, SafetyCause::Frost) && value < house.frost + FROST_MARGIN) =>
        {
            let frost_target = house.frost + FROST_MARGIN;
            let sp = Setpoint::Temp(frost_target);
            let reason = if value < house.frost {
                format!("Frost protection: {} < {}", temp(value), temp(house.frost))
            } else {
                format!(
                    "Frost protection: {}, holding until {}",
                    temp(value),
                    temp(frost_target)
                )
            };
            let mut d = safety_decision(inp, SafetyCause::Frost, reason);
            d.mode = Some(Mode::Heat);
            d.setpoint = Some(sp);
            d.entities = inp
                .devices
                .iter()
                .filter_map(|dev| {
                    // A device that is off is not warming the room, whatever target it remembers.
                    let t = match dev.target {
                        Some(c) if dev.mode.as_deref() != Some("off") => c.max(frost_target),
                        _ => frost_target,
                    };
                    Some((
                        dev.id.clone(),
                        device_target(Setpoint::Temp(t), Mode::Heat, &dev.caps)?,
                    ))
                })
                .collect();
            Some(d)
        }
        Reading::Fresh { value, .. }
            if inp.mode == Some(Mode::Heat)
                && (value > house.overheat
                    || (latched(inp, SafetyCause::Overheat)
                        && value > house.overheat - SAFETY_HYSTERESIS)) =>
        {
            let base = inp
                .settings
                .rules
                .get(Mode::Heat)
                .map(|r| r.base)
                .and_then(|b| match b {
                    Setpoint::Temp(t) if t <= house.overheat => Some(Setpoint::Temp(t)),
                    _ => None,
                });
            // Latched inside the hysteresis band, the reading is no longer above the limit.
            let reason = if value > house.overheat {
                format!(
                    "Overheat protection: {} > {}",
                    temp(value),
                    temp(house.overheat)
                )
            } else {
                format!(
                    "Overheat protection: {}, holding until {}",
                    temp(value),
                    temp(house.overheat - SAFETY_HYSTERESIS)
                )
            };
            let mut d = safety_decision(inp, SafetyCause::Overheat, reason);
            // A device that is off is not heating, so overheat leaves it alone.
            let running: Vec<Device> = inp
                .devices
                .iter()
                .filter(|dev| dev.mode.as_deref() != Some("off"))
                .cloned()
                .collect();
            d.entities = match base {
                Some(sp) => {
                    d.setpoint = Some(sp);
                    targets(&running, sp, Mode::Heat)
                }
                None => running
                    .iter()
                    .filter_map(|dev| {
                        let sp = Setpoint::Temp(dev.caps.min_temp.unwrap_or(house.frost));
                        Some((dev.id.clone(), device_target(sp, Mode::Heat, &dev.caps)?))
                    })
                    .collect(),
            };
            if d.setpoint.is_none() {
                d.setpoint = d
                    .entities
                    .values()
                    .filter_map(|t| t.setpoint)
                    .min_by(f64::total_cmp)
                    .map(Setpoint::Temp);
            }
            Some(d)
        }
        Reading::Fresh { value, .. }
            if inp.mode == Some(Mode::Cool)
                && (value < house.overcool
                    || (latched(inp, SafetyCause::Overcool)
                        && value < house.overcool + SAFETY_HYSTERESIS)) =>
        {
            let reason = if value < house.overcool {
                format!(
                    "Overcool protection: {} < {}",
                    temp(value),
                    temp(house.overcool)
                )
            } else {
                format!(
                    "Overcool protection: {}, holding until {}",
                    temp(value),
                    temp(house.overcool + SAFETY_HYSTERESIS)
                )
            };
            let mut d = safety_decision(inp, SafetyCause::Overcool, reason);
            d.setpoint = Some(Setpoint::Off);
            d.entities = targets(inp.devices, Setpoint::Off, Mode::Cool);
            let stuck: Vec<&str> = inp
                .devices
                .iter()
                .filter(|dev| can(&dev.caps, "cool") && !d.entities.contains_key(&dev.id))
                .map(|dev| dev.id.as_str())
                .collect();
            if !stuck.is_empty() {
                d.reason
                    .push_str(&format!("; cannot stop cooling: {}", stuck.join(", ")));
            }
            Some(d)
        }
        Reading::Fresh { .. } => None,
        Reading::Stale { .. } | Reading::Missing => stale(inp),
    }
}

fn latched(inp: &ZoneInputs, cause: SafetyCause) -> bool {
    inp.previous_cause == Some(cause)
}

/// The stale-reading row: no usable reading, so never let a schedule, override or eco setpoint
/// through in an active zone.
fn stale(inp: &ZoneInputs) -> Option<Decision> {
    let what = match inp.reading {
        Reading::Stale { last: Some(v) } => format!("Reading is stale (last {})", temp(v)),
        Reading::Stale { last: None } => "Reading is stale".to_string(),
        _ => "No temperature reading".to_string(),
    };
    if inp.settings.state != ZoneState::Active {
        return Some(safety_decision(
            inp,
            SafetyCause::Stale,
            format!("{what}: not acting until approved rules exist"),
        ));
    }
    let house = inp.house;
    let rules = inp.mode.and_then(|m| inp.settings.rules.get(m));
    let (Some(mode), Some(rules)) = (inp.mode, rules) else {
        return Some(safety_decision(
            inp,
            SafetyCause::Stale,
            format!("{what}: no rules to fall back on, not acting"),
        ));
    };
    let base = rules.base;
    let mut d = safety_decision(
        inp,
        SafetyCause::Stale,
        format!("{what}: holding the safer of the current target and base"),
    );
    match mode {
        Mode::Heat => {
            let base_t = match base {
                Setpoint::Temp(b) => b,
                _ => house.frost,
            };
            d.setpoint = Some(Setpoint::Temp(base_t.max(house.frost)));
            d.entities = inp
                .devices
                .iter()
                .filter(|dev| dev.mode.as_deref() != Some("off"))
                .filter_map(|dev| {
                    let t = dev
                        .target
                        .map_or(base_t, |c| c.min(base_t))
                        .max(house.frost);
                    Some((
                        dev.id.clone(),
                        device_target(Setpoint::Temp(t), Mode::Heat, &dev.caps)?,
                    ))
                })
                .collect();
        }
        Mode::Cool => {
            d.setpoint = Some(base);
            d.entities = inp
                .devices
                .iter()
                .filter(|dev| dev.mode.as_deref() != Some("off"))
                .filter_map(|dev| {
                    let sp = match (base, dev.target) {
                        (Setpoint::Temp(b), Some(c)) => Setpoint::Temp(c.max(b)),
                        (Setpoint::Temp(b), None) => Setpoint::Temp(b),
                        _ => Setpoint::Off,
                    };
                    Some((dev.id.clone(), device_target(sp, Mode::Cool, &dev.caps)?))
                })
                .collect();
        }
    }
    Some(d)
}

/// A pending zone acts only on safety: restore what safety changed, or just show what it would do.
fn pending(inp: &ZoneInputs) -> Option<Decision> {
    if inp.settings.state != ZoneState::Pending {
        return None;
    }
    if let Some(mem) = inp.safety_memory {
        let mut d = base_decision(
            inp,
            Source::Idle,
            "Safety cleared: restoring previous settings".to_string(),
        );
        d.entities = mem.found.clone();
        return Some(d);
    }
    let choice = choose(inp, inp.mode);
    let mut d = base_decision(inp, Source::Idle, "Waiting for approved rules".to_string());
    if let Some(sp) = choice.setpoint {
        d.setpoint = Some(sp);
        d.reason
            .push_str(&format!("; would set {}", setpoint_text(sp)));
    }
    Some(d)
}

fn live(o: &Override, now: u64) -> bool {
    o.until > now
}

fn latest<'a>(it: impl Iterator<Item = &'a Override>) -> Option<&'a Override> {
    it.max_by_key(|o| o.until)
}

fn clock(inp: &ZoneInputs, until: u64) -> String {
    let secs = i64::try_from(until.saturating_sub(inp.now)).unwrap_or(i64::MAX / 2);
    (inp.local_now + Duration::seconds(secs))
        .format("%H:%M")
        .to_string()
}

/// A whole-zone override: every device follows it.
fn zone_override(inp: &ZoneInputs) -> Option<Decision> {
    let ov = latest(
        inp.overrides
            .iter()
            .filter(|o| o.entity.is_none() && live(o, inp.now)),
    )?;
    let mode = ov.mode.or(inp.mode)?;
    if let Some(Setpoint::Temp(t)) = ov.setpoint {
        let sp = Setpoint::Temp(t);
        let mut d = base_decision(
            inp,
            Source::Boost,
            format!(
                "Boost to {} until {}",
                setpoint_text(sp),
                clock(inp, ov.until)
            ),
        );
        d.mode = Some(mode);
        d.setpoint = Some(sp);
        d.entities = targets(inp.devices, sp, mode);
        apply_correction(inp, &mut d, mode, t);
        hold_devices(inp, &mut d, Some(sp));
        return Some(d);
    }
    let sp = ov.setpoint.or_else(|| choose(inp, Some(mode)).setpoint)?;
    let mut d = base_decision(
        inp,
        Source::Override,
        format!(
            "Override to {} until {}",
            setpoint_text(sp),
            clock(inp, ov.until)
        ),
    );
    d.mode = Some(mode);
    d.setpoint = Some(sp);
    d.entities = targets(inp.devices, sp, mode);
    hold_devices(inp, &mut d, Some(sp));
    Some(d)
}

/// What the holiday, away and schedule rows choose.
struct Choice {
    source: Source,
    setpoint: Option<Setpoint>,
    reason: String,
}

fn choose(inp: &ZoneInputs, mode: Option<Mode>) -> Choice {
    let idle = |reason: &str| Choice {
        source: Source::Idle,
        setpoint: None,
        reason: reason.to_string(),
    };
    let Some(mode) = mode else {
        return idle("No heating or cooling season");
    };
    if inp.holiday {
        let sp = match mode {
            Mode::Heat => Setpoint::Temp(inp.house.frost),
            Mode::Cool => Setpoint::Off,
        };
        return Choice {
            source: Source::Holiday,
            setpoint: Some(sp),
            reason: format!("Holiday: {}", setpoint_text(sp)),
        };
    }
    let (scheduled, block) = match inp.settings.rules.get(mode) {
        None => return idle(&format!("No rules for {} mode", mode_str(mode))),
        Some(r) => r.at(inp.local_now),
    };
    let room_eco = inp.room_eco.filter(|e| e.until > inp.now);
    if room_eco.is_some() || !inp.home {
        let eco = inp.house.eco.setpoint(mode);
        let sp = eco_setpoint(mode, eco, scheduled);
        let mut reason = match room_eco {
            Some(e) if inp.home => format!(
                "Eco until {}: {}",
                day_clock(inp, e.until),
                setpoint_text(sp)
            ),
            Some(e) => format!(
                "Eco until {} (nobody home): {}",
                day_clock(inp, e.until),
                setpoint_text(sp)
            ),
            None => format!("Nobody home: eco {}", setpoint_text(sp)),
        };
        if sp != Setpoint::Temp(eco) {
            reason.push_str(match mode {
                Mode::Heat => "; schedule already lower",
                Mode::Cool => "; schedule already higher",
            });
        }
        return Choice {
            source: if room_eco.is_some() {
                Source::Eco
            } else {
                Source::Away
            },
            setpoint: Some(sp),
            reason,
        };
    }
    if let (Some(_), Setpoint::Temp(t)) = (block, scheduled)
        && let Some(choice) = setback(inp, mode, t)
    {
        return choice;
    }
    let reason = match block {
        Some(i) => format!("Schedule block {}: {}", i + 1, setpoint_text(scheduled)),
        None => format!("Schedule base: {}", setpoint_text(scheduled)),
    };
    Choice {
        source: Source::Schedule,
        setpoint: Some(scheduled),
        reason,
    }
}

/// The presence setback of a block at `scheduled` (spec §7.2), while the room is empty: lower in
/// heat (never under the base or frost), higher in cool (never over the base), and never past
/// the block itself. `None` when it is off, the room is occupied, or the limits leave the block
/// as it is.
fn setback(inp: &ZoneInputs, mode: Mode, scheduled: f64) -> Option<Choice> {
    let p = &inp.settings.presence;
    if !p.enabled || !inp.room_empty {
        return None;
    }
    let base = match inp.settings.rules.get(mode).map(|r| r.base) {
        Some(Setpoint::Temp(b)) => Some(b),
        _ => None,
    };
    let (wanted, sign) = match mode {
        Mode::Heat => (scheduled - p.setback, '−'),
        Mode::Cool => (scheduled + p.setback, '+'),
    };
    let (target, limit) = match mode {
        Mode::Heat => {
            let floor = base.map_or(inp.house.frost, |b| b.max(inp.house.frost));
            if wanted >= floor {
                (wanted, None)
            } else if base.is_some_and(|b| b >= inp.house.frost) {
                (floor, Some("base"))
            } else {
                (floor, Some("frost"))
            }
        }
        Mode::Cool => match base {
            Some(b) if wanted > b => (b, Some("base")),
            _ => (wanted, None),
        },
    };
    // A block already under the base (heat) or over it (cool) is never moved the other way.
    let target = match mode {
        Mode::Heat => target.min(scheduled),
        Mode::Cool => target.max(scheduled),
    };
    if (target - scheduled).abs() <= 0.01 {
        return None;
    }
    let detail = match limit {
        Some(word) => word.to_owned(),
        None => format!("{scheduled:.1} {sign} {:.1}", p.setback),
    };
    Some(Choice {
        source: Source::Empty,
        setpoint: Some(Setpoint::Temp(target)),
        reason: format!(
            "No one in the room for {} min: {} ({detail})",
            p.empty_after_min,
            temp(target)
        ),
    })
}

/// What eco holds in `mode` given what the schedule asks for: never more than the schedule
/// (the lower of the two in heat, the higher in cool), and a schedule `off` stays `off`.
pub fn eco_setpoint(mode: Mode, eco: f64, schedule: Setpoint) -> Setpoint {
    match (mode, schedule) {
        (_, Setpoint::Off) => Setpoint::Off,
        (Mode::Heat, Setpoint::Temp(t)) => Setpoint::Temp(t.min(eco)),
        (Mode::Cool, Setpoint::Temp(t)) => Setpoint::Temp(t.max(eco)),
    }
}

/// `until` as the house's weekday, date and clock time ("Sun 1 Nov 00:00"): eco can end days away.
fn day_clock(inp: &ZoneInputs, until: u64) -> String {
    let secs = i64::try_from(until.saturating_sub(inp.now)).unwrap_or(i64::MAX / 2);
    (inp.local_now + Duration::seconds(secs))
        .format("%a %-d %b %H:%M")
        .to_string()
}

/// Holiday, away and schedule, then per-device overrides on top.
fn rules(inp: &ZoneInputs) -> Decision {
    let choice = choose(inp, inp.mode);
    let mut d = base_decision(inp, choice.source, choice.reason);
    if let (Some(mode), Some(sp)) = (inp.mode, choice.setpoint) {
        d.setpoint = Some(sp);
        d.entities = targets(inp.devices, sp, mode);
        if let (Source::Schedule | Source::Empty, Setpoint::Temp(target)) = (choice.source, sp) {
            apply_correction(inp, &mut d, mode, target);
        }
    }
    hold_devices(inp, &mut d, choice.setpoint);
    d
}

/// The room-thermometer correction (spec §6): shift the unheld devices' setpoint by the offset.
fn apply_correction(inp: &ZoneInputs, d: &mut Decision, mode: Mode, target: f64) {
    let fb = &inp.settings.feedback;
    if !fb.enabled {
        return;
    }
    let unheld = || {
        inp.devices.iter().filter(|dev| {
            !inp.overrides
                .iter()
                .any(|o| o.entity.as_deref() == Some(dev.id.as_str()) && live(o, inp.now))
        })
    };
    let step = unheld()
        .map(|dev| {
            dev.caps
                .temp_step
                .filter(|s| *s > 0.0)
                .unwrap_or(DEFAULT_STEP)
        })
        .fold(fb.step, f64::max);
    let base = match inp.settings.rules.get(mode).map(|r| r.base) {
        Some(Setpoint::Temp(b)) => Some(b),
        _ => None,
    };
    let c = correct(&CorrectionInput {
        settings: fb,
        mode,
        target,
        base,
        frost: inp.house.frost,
        thermometer: inp.thermometer,
        step,
        frozen: unheld().next().is_none(),
        previous: inp.feedback,
        now: inp.now,
    });
    d.entities = targets(inp.devices, Setpoint::Temp(target + c.offset), mode);
    d.adjust = (c.offset != 0.0).then_some(c.offset);
    if let Some(note) = c.note {
        d.reason.push_str(&format!("; {note}"));
    }
}

/// Per-device overrides (hand changes) hold their device whatever the zone does (§7).
fn hold_devices(inp: &ZoneInputs, d: &mut Decision, fallback: Option<Setpoint>) {
    let zone_mode = d.mode.or(inp.mode);
    for dev in inp.devices {
        let Some(ov) = latest(
            inp.overrides
                .iter()
                .filter(|o| o.entity.as_deref() == Some(dev.id.as_str()) && live(o, inp.now)),
        ) else {
            continue;
        };
        let held = match (ov.mode.or(zone_mode), ov.setpoint.or(fallback)) {
            (Some(mode), Some(sp)) => device_target(sp, mode, &dev.caps).map(|mut t| {
                t.held = true;
                t
            }),
            _ if !d.entities.contains_key(&dev.id) => continue,
            _ => None,
        };
        match held {
            Some(t) => {
                d.entities.insert(dev.id.clone(), t);
                d.reason
                    .push_str(&format!("; {} held until {}", dev.id, clock(inp, ov.until)));
            }
            None => {
                match &dev.mode {
                    Some(mode) => d.entities.insert(
                        dev.id.clone(),
                        DeviceTarget {
                            setpoint: dev.target,
                            mode: mode.clone(),
                            held: true,
                        },
                    ),
                    None => d.entities.remove(&dev.id),
                };
                d.reason.push_str(&format!(
                    "; {} held as it is until {}",
                    dev.id,
                    clock(inp, ov.until)
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{Block, Day, HhMm, ModeRules, OverrideOrigin, RoomEco, Rules, ZoneSettings};

    const NOW: u64 = 1_000_000;

    fn caps(modes: &[&str]) -> DeviceCaps {
        DeviceCaps {
            hvac_modes: modes.iter().map(|m| m.to_string()).collect(),
            fan_modes: vec![],
            min_temp: Some(5.0),
            max_temp: Some(30.0),
            temp_step: Some(0.5),
        }
    }

    fn dev(id: &str, modes: &[&str], target: Option<f64>) -> Device {
        Device {
            id: id.into(),
            caps: caps(modes),
            target,
            mode: Some("heat".into()),
            fan_mode: None,
        }
    }

    fn trv() -> Device {
        dev("trv", &["off", "heat"], Some(21.0))
    }

    fn fresh(v: f64) -> Reading {
        Reading::Fresh {
            value: v,
            age_s: 30,
            from_sensor: true,
            sensor_stale: false,
        }
    }

    /// Wednesday 10:00 local.
    fn local() -> NaiveDateTime {
        NaiveDateTime::parse_from_str("2026-10-21 10:00", "%Y-%m-%d %H:%M").unwrap()
    }

    fn all_days() -> Vec<Day> {
        vec![
            Day::Mon,
            Day::Tue,
            Day::Wed,
            Day::Thu,
            Day::Fri,
            Day::Sat,
            Day::Sun,
        ]
    }

    fn mode_rules(base: Setpoint) -> ModeRules {
        ModeRules {
            base,
            blocks: vec![Block {
                days: all_days(),
                from: HhMm(420),
                to: HhMm(1320),
                setpoint: Setpoint::Temp(20.5),
                evidence: None,
            }],
        }
    }

    fn active(base: Setpoint) -> ZoneSettings {
        ZoneSettings {
            state: ZoneState::Active,
            rules: Rules {
                heat: Some(mode_rules(base)),
                cool: Some(mode_rules(Setpoint::Temp(26.0))),
            },
            ..Default::default()
        }
    }

    struct Case {
        settings: ZoneSettings,
        house: HouseSettings,
        mode: Option<Mode>,
        reading: Reading,
        home: bool,
        holiday: bool,
        overrides: Vec<Override>,
        devices: Vec<Device>,
        memory: Option<SafetyMemory>,
        room_eco: Option<RoomEco>,
        previous: Option<SafetyCause>,
        thermometer: Option<ReadingSource>,
        feedback: Option<FeedbackState>,
        room_empty: bool,
    }

    impl Case {
        fn new() -> Self {
            Case {
                settings: active(Setpoint::Temp(17.0)),
                house: HouseSettings::default(),
                mode: Some(Mode::Heat),
                reading: fresh(19.0),
                home: true,
                holiday: false,
                overrides: vec![],
                devices: vec![trv()],
                memory: None,
                previous: None,
                room_eco: None,
                thermometer: None,
                feedback: None,
                room_empty: false,
            }
        }

        fn run(&self) -> Decision {
            decide(&ZoneInputs {
                zone: "z",
                settings: &self.settings,
                house: &self.house,
                mode: self.mode,
                reading: self.reading,
                outdoor_mean: Some(8.0),
                home: self.home,
                holiday: self.holiday,
                overrides: &self.overrides,
                devices: &self.devices,
                local_now: local(),
                now: NOW,
                safety_memory: self.memory.as_ref(),
                previous_cause: self.previous,
                room_eco: self.room_eco.as_ref(),
                thermometer: self.thermometer,
                feedback: self.feedback.as_ref(),
                room_empty: self.room_empty,
            })
        }
    }

    fn target(sp: Option<f64>, mode: &str, held: bool) -> DeviceTarget {
        DeviceTarget {
            setpoint: sp,
            mode: mode.into(),
            held,
        }
    }

    fn zone_ov(sp: f64, until: u64) -> Override {
        Override {
            entity: None,
            setpoint: Some(Setpoint::Temp(sp)),
            mode: None,
            until,
            origin: OverrideOrigin::User,
        }
    }

    /// Heat, block 19 °C, correction on, room thermometer at `r` and `age_s` old.
    fn corrected(r: f64, age_s: u64) -> Case {
        let mut c = Case::new();
        c.settings.rules.heat.as_mut().unwrap().blocks[0].setpoint = Setpoint::Temp(19.0);
        c.settings.feedback.enabled = true;
        c.thermometer = Some(ReadingSource { value: r, age_s });
        c
    }

    fn dev_step(id: &str, step: f64) -> Device {
        let mut d = dev(id, &["off", "heat"], Some(19.0));
        d.caps.temp_step = Some(step);
        d
    }

    fn dev_ov(id: &str, sp: f64) -> Override {
        Override {
            entity: Some(id.into()),
            setpoint: Some(Setpoint::Temp(sp)),
            mode: None,
            until: NOW + 3600,
            origin: OverrideOrigin::User,
        }
    }

    /// Heat over a `base` base, block 19 °C at 10:00, presence on and the room empty.
    fn empty_room(base: f64) -> Case {
        let mut c = Case::new();
        let heat = c.settings.rules.heat.as_mut().unwrap();
        heat.base = Setpoint::Temp(base);
        heat.blocks[0].setpoint = Setpoint::Temp(19.0);
        c.settings.presence.enabled = true;
        c.room_empty = true;
        c
    }

    #[test]
    fn empty_room_lowers_a_block() {
        let d = empty_room(15.0).run();
        assert_eq!(d.source, Source::Empty);
        assert_eq!(d.setpoint, Some(Setpoint::Temp(17.0)));
        assert_eq!(d.entities["trv"].setpoint, Some(17.0));
        assert_eq!(
            d.reason,
            "No one in the room for 30 min: 17.0 °C (19.0 − 2.0)"
        );
    }

    #[test]
    fn empty_room_keeps_the_base() {
        let mut c = empty_room(15.0);
        // The block starts at 11:00: at 10:00 the room is on its base.
        c.settings.rules.heat.as_mut().unwrap().blocks[0].from = HhMm(660);
        let d = c.run();
        assert_eq!(d.source, Source::Schedule);
        assert_eq!(d.setpoint, Some(Setpoint::Temp(15.0)));
    }

    #[test]
    fn setback_stops_at_the_base() {
        let d = empty_room(18.0).run();
        assert_eq!(d.source, Source::Empty);
        assert_eq!(d.setpoint, Some(Setpoint::Temp(18.0)));
        assert!(d.reason.ends_with("18.0 °C (base)"), "{}", d.reason);
    }

    #[test]
    fn setback_is_off_by_default() {
        let mut c = empty_room(15.0);
        c.settings.presence.enabled = false;
        let d = c.run();
        assert_eq!(d.source, Source::Schedule);
        assert_eq!(d.setpoint, Some(Setpoint::Temp(19.0)));
    }

    #[test]
    fn away_wins_over_setback() {
        let mut c = empty_room(15.0);
        c.home = false;
        assert_eq!(c.run().source, Source::Away);
    }

    #[test]
    fn cool_setback_raises() {
        let mut c = Case::new();
        c.mode = Some(Mode::Cool);
        c.devices = vec![dev("ac", &["off", "cool"], Some(25.0))];
        let cool = c.settings.rules.cool.as_mut().unwrap();
        cool.base = Setpoint::Temp(28.0);
        cool.blocks[0].setpoint = Setpoint::Temp(25.0);
        c.settings.presence.enabled = true;
        c.room_empty = true;
        let d = c.run();
        assert_eq!(d.source, Source::Empty);
        assert_eq!(d.setpoint, Some(Setpoint::Temp(27.0)));
        assert_eq!(
            d.reason,
            "No one in the room for 30 min: 27.0 °C (25.0 + 2.0)"
        );
        c.settings.rules.cool.as_mut().unwrap().base = Setpoint::Temp(26.0);
        let d = c.run();
        assert_eq!(d.setpoint, Some(Setpoint::Temp(26.0)));
        assert!(d.reason.ends_with("26.0 °C (base)"), "{}", d.reason);
    }

    #[test]
    fn setback_stops_at_frost() {
        let mut c = empty_room(15.0);
        let heat = c.settings.rules.heat.as_mut().unwrap();
        heat.base = Setpoint::Off;
        heat.blocks[0].setpoint = Setpoint::Temp(8.0);
        let d = c.run();
        assert_eq!(d.setpoint, Some(Setpoint::Temp(7.0)));
        assert!(d.reason.ends_with("7.0 °C (frost)"), "{}", d.reason);
    }

    #[test]
    fn setback_never_heats_past_the_block() {
        // A night block under the base: the empty room keeps its block, never the base.
        let mut c = empty_room(20.0);
        c.settings.rules.heat.as_mut().unwrap().blocks[0].setpoint = Setpoint::Temp(17.0);
        let d = c.run();
        assert_eq!(d.source, Source::Schedule);
        assert_eq!(d.setpoint, Some(Setpoint::Temp(17.0)));
        // A block under frost is not raised either.
        let mut c = empty_room(15.0);
        let heat = c.settings.rules.heat.as_mut().unwrap();
        heat.base = Setpoint::Off;
        heat.blocks[0].setpoint = Setpoint::Temp(6.0);
        let d = c.run();
        assert_eq!(d.source, Source::Schedule);
        assert_eq!(d.setpoint, Some(Setpoint::Temp(6.0)));
    }

    #[test]
    fn setback_never_cools_past_the_block() {
        let mut c = Case::new();
        c.mode = Some(Mode::Cool);
        c.devices = vec![dev("ac", &["off", "cool"], Some(26.0))];
        let cool = c.settings.rules.cool.as_mut().unwrap();
        cool.base = Setpoint::Temp(24.0);
        cool.blocks[0].setpoint = Setpoint::Temp(26.0);
        c.settings.presence.enabled = true;
        c.room_empty = true;
        let d = c.run();
        assert_eq!(d.source, Source::Schedule);
        assert_eq!(d.setpoint, Some(Setpoint::Temp(26.0)));
    }

    #[test]
    fn correction_runs_against_the_setback_target() {
        let mut c = empty_room(15.0);
        c.settings.feedback.enabled = true;
        // Just over the setback's band (17.5): one step, not a jump.
        c.thermometer = Some(ReadingSource {
            value: 17.6,
            age_s: 30,
        });
        let d = c.run();
        assert_eq!(d.source, Source::Empty);
        assert_eq!(d.setpoint, Some(Setpoint::Temp(17.0)));
        assert_eq!(d.entities["trv"].setpoint, Some(16.5));
        assert_eq!(d.adjust, Some(-0.5));
    }

    #[test]
    fn correction_lowers_unheld_devices() {
        let d = corrected(19.6, 30).run();
        assert_eq!(d.source, Source::Schedule);
        assert_eq!(d.setpoint, Some(Setpoint::Temp(19.0)));
        assert_eq!(d.entities["trv"].setpoint, Some(18.5));
        assert_eq!(d.adjust, Some(-0.5));
        assert!(
            d.reason.contains("above 19.5 °C: AC 18.5 °C"),
            "{}",
            d.reason
        );
    }

    #[test]
    fn correction_jumps_by_the_gap_within_the_base() {
        // 2.6 over the target: a -2.5 jump, held by the rules' 17 °C base.
        let d = corrected(21.6, 30).run();
        assert_eq!(d.adjust, Some(-2.0));
        assert_eq!(d.entities["trv"].setpoint, Some(17.0));
    }

    #[test]
    fn correction_off_by_default() {
        let mut c = corrected(21.6, 30);
        c.settings.feedback.enabled = false;
        let d = c.run();
        assert_eq!(d.entities["trv"].setpoint, Some(19.0));
        assert_eq!(d.adjust, None);
    }

    #[test]
    fn boost_runs_correction() {
        let mut c = corrected(21.6, 30);
        c.overrides = vec![zone_ov(21.0, NOW + 3600)];
        let d = c.run();
        assert_eq!(d.source, Source::Boost);
        assert_eq!(d.setpoint, Some(Setpoint::Temp(21.0)));
        assert!(d.adjust.is_some());
        assert!(d.entities["trv"].setpoint < Some(21.0));
        assert!(
            d.reason.starts_with("Boost to 21.0 °C until 11:00"),
            "{}",
            d.reason
        );
    }

    #[test]
    fn boost_without_feedback_sends_target() {
        let mut c = Case::new();
        c.overrides = vec![zone_ov(22.0, NOW + 3600)];
        let d = c.run();
        assert_eq!(d.source, Source::Boost);
        assert_eq!(d.entities["trv"], target(Some(22.0), "heat", false));
        assert_eq!(d.adjust, None);
    }

    #[test]
    fn mode_only_override_is_unchanged() {
        let mut c = Case::new();
        let mut off = zone_ov(0.0, NOW + 3600);
        off.setpoint = Some(Setpoint::Off);
        c.overrides = vec![off];
        let d = c.run();
        assert_eq!(d.source, Source::Override);
        assert_eq!(d.adjust, None);
    }

    #[test]
    fn boost_beats_away_eco_and_empty() {
        let mut away = corrected(19.0, 30);
        away.home = false;
        let mut eco = corrected(19.0, 30);
        eco.room_eco = eco_until(NOW + 7200);
        for mut c in [away, eco, empty_room(15.0)] {
            assert_ne!(c.run().source, Source::Boost);
            c.overrides = vec![zone_ov(22.0, NOW + 3600)];
            assert_eq!(c.run().source, Source::Boost);
        }
    }

    #[test]
    fn stale_reading_beats_boost() {
        let mut c = corrected(21.0, 30);
        c.reading = Reading::Missing;
        c.overrides = vec![zone_ov(25.0, NOW + 3600)];
        let d = c.run();
        assert_eq!(d.source, Source::Safety);
        assert_eq!(d.entities["trv"].setpoint, Some(17.0));
    }

    #[test]
    fn held_device_under_boost() {
        let mut c = corrected(21.6, 30);
        c.devices = vec![dev_step("a", 0.5), dev_step("b", 0.5)];
        c.overrides = vec![zone_ov(22.0, NOW + 3600), dev_ov("a", 20.0)];
        let d = c.run();
        assert_eq!(d.source, Source::Boost);
        assert_eq!(d.entities["a"], target(Some(20.0), "heat", true));
        assert_eq!(
            d.entities["b"].setpoint,
            Some(22.0 + d.adjust.unwrap_or(0.0))
        );
        assert!(!d.entities["b"].held);
    }

    #[test]
    fn held_device_keeps_its_value() {
        let mut c = corrected(19.6, 30);
        c.devices = vec![dev_step("a", 0.5), dev_step("b", 0.5)];
        c.overrides = vec![dev_ov("a", 20.0)];
        let d = c.run();
        assert_eq!(d.entities["a"].setpoint, Some(20.0));
        assert!(d.entities["a"].held);
        assert_eq!(d.entities["b"].setpoint, Some(18.5));
    }

    #[test]
    fn all_held_freezes() {
        let mut c = corrected(22.0, 30);
        c.devices = vec![dev_step("a", 0.5), dev_step("b", 0.5)];
        c.overrides = vec![dev_ov("a", 20.0), dev_ov("b", 20.0)];
        c.feedback = Some(FeedbackState {
            target: 19.0,
            offset: -0.5,
            since: NOW - 7200,
            last_step_at: Some(NOW - 7200),
            reached: false,
        });
        let d = c.run();
        assert_eq!(d.adjust, Some(-0.5));
        assert_eq!(d.entities["a"].setpoint, Some(20.0));
        assert_eq!(d.entities["b"].setpoint, Some(20.0));
    }

    #[test]
    fn coarse_device_sets_the_step() {
        // 0.6 over the target: one 1 °C step (a 0.5 °C step would give -0.5).
        let mut c = corrected(19.6, 30);
        c.devices = vec![dev_step("a", 0.5), dev_step("b", 1.0)];
        let d = c.run();
        assert_eq!(d.adjust, Some(-1.0));
        assert_eq!(d.entities["a"].setpoint, Some(18.0));
        assert_eq!(d.entities["b"].setpoint, Some(18.0));
    }

    #[test]
    fn silent_thermometer_sends_the_schedule() {
        let d = corrected(21.6, 7 * 3600).run();
        assert_eq!(d.entities["trv"].setpoint, Some(19.0));
        assert_eq!(d.adjust, None);
        assert!(d.reason.contains("silent: no correction"), "{}", d.reason);
    }

    #[test]
    fn quiet_thermometer_reads_the_room_and_still_corrects() {
        let mut c = corrected(19.6, 2 * 3600);
        c.reading = Reading::Fresh {
            value: 19.6,
            age_s: 2 * 3600,
            from_sensor: true,
            sensor_stale: false,
        };
        let d = c.run();
        assert_eq!(d.entities["trv"].setpoint, Some(18.5));
        assert!(!d.reason.contains("silent for"), "{}", d.reason);
    }

    #[test]
    fn off_zone_does_nothing() {
        let mut c = Case::new();
        c.settings.state = ZoneState::Off;
        c.reading = fresh(2.0);
        let d = c.run();
        assert_eq!(d.source, Source::Idle);
        assert!(d.entities.is_empty());
        assert_eq!(d.reason, "Zone is off");
        assert!(!d.applied);
        assert_eq!(d.at, NOW);
    }

    #[test]
    fn frost_beats_override_in_active_zone() {
        let mut c = Case::new();
        c.devices = vec![dev("trv", &["off", "heat"], Some(5.0))];
        c.reading = fresh(6.5);
        c.overrides = vec![zone_ov(22.0, NOW + 600)];
        let d = c.run();
        assert_eq!(d.source, Source::Safety);
        assert_eq!(d.reason, "Frost protection: 6.5 °C < 7.0 °C");
        assert_eq!(d.entities["trv"], target(Some(9.0), "heat", false));
        assert_eq!(d.reading, Some(6.5));
        assert_eq!(d.reading_age_s, Some(30));
    }

    #[test]
    fn every_safety_decision_names_its_cause() {
        let mut c = Case::new();
        c.reading = fresh(6.5);
        assert_eq!(c.run().cause, Some(SafetyCause::Frost));
        c.reading = fresh(31.0);
        assert_eq!(c.run().cause, Some(SafetyCause::Overheat));
        c.mode = Some(Mode::Cool);
        c.reading = fresh(15.0);
        assert_eq!(c.run().cause, Some(SafetyCause::Overcool));
        c.reading = Reading::Missing;
        assert_eq!(c.run().cause, Some(SafetyCause::Stale));
        c.reading = fresh(19.0);
        assert_eq!(c.run().cause, None);
    }

    #[test]
    fn frost_holds_until_the_frost_target() {
        let mut c = Case::new();
        c.devices = vec![dev("trv", &["off", "heat"], Some(5.0))];
        c.previous = Some(SafetyCause::Frost);
        for (reading, safety) in [
            (6.9, true),
            (7.0, true),
            (7.5, true),
            (8.99, true),
            (9.0, false),
        ] {
            c.reading = fresh(reading);
            assert_eq!(c.run().source == Source::Safety, safety, "{reading}");
        }
        // Not latched: the limit itself.
        c.previous = None;
        c.reading = fresh(7.0);
        assert_ne!(c.run().source, Source::Safety);
        c.reading = fresh(6.99);
        assert_eq!(c.run().source, Source::Safety);
        c.previous = Some(SafetyCause::Frost);
        c.reading = fresh(8.0);
        assert_eq!(
            c.run().reason,
            "Frost protection: 8.0 °C, holding until 9.0 °C"
        );
        // Another latched cause does not hold frost.
        c.previous = Some(SafetyCause::Stale);
        c.reading = fresh(7.2);
        assert_ne!(c.run().source, Source::Safety);
    }

    #[test]
    fn overheat_and_overcool_hold_with_hysteresis() {
        let mut c = Case::new();
        c.previous = Some(SafetyCause::Overheat);
        for (reading, safety) in [(30.1, true), (29.6, true), (29.5, false)] {
            c.reading = fresh(reading);
            assert_eq!(c.run().source == Source::Safety, safety, "{reading}");
        }
        c.mode = Some(Mode::Cool);
        c.previous = Some(SafetyCause::Overcool);
        for (reading, safety) in [(15.9, true), (16.4, true), (16.5, false)] {
            c.reading = fresh(reading);
            assert_eq!(c.run().source == Source::Safety, safety, "{reading}");
        }
    }

    #[test]
    fn latched_reasons_say_what_holds_them() {
        let mut c = Case::new();
        c.previous = Some(SafetyCause::Overheat);
        c.reading = fresh(30.1);
        assert!(c.run().reason.contains("> 30"), "{}", c.run().reason);
        c.reading = fresh(29.6);
        let reason = c.run().reason;
        assert!(reason.contains("holding until 29.5"), "{reason}");
        assert!(!reason.contains('>'), "{reason}");
        c.mode = Some(Mode::Cool);
        c.previous = Some(SafetyCause::Overcool);
        c.reading = fresh(16.4);
        let reason = c.run().reason;
        assert!(reason.contains("holding until 16.5"), "{reason}");
        assert!(!reason.contains('<'), "{reason}");
    }

    #[test]
    fn frost_acts_in_pending_zone() {
        let mut c = Case::new();
        c.settings.state = ZoneState::Pending;
        c.devices = vec![dev("trv", &["off", "heat"], Some(5.0))];
        c.reading = fresh(6.5);
        let d = c.run();
        assert_eq!(d.source, Source::Safety);
        assert_eq!(d.entities["trv"], target(Some(9.0), "heat", false));
    }

    #[test]
    fn overheat_uses_base_or_device_min() {
        let mut c = Case::new();
        c.reading = fresh(31.0);
        let d = c.run();
        assert_eq!(d.source, Source::Safety);
        assert_eq!(d.entities["trv"], target(Some(17.0), "heat", false));
        c.settings = active(Setpoint::Temp(31.0));
        let d = c.run();
        assert_eq!(d.entities["trv"], target(Some(5.0), "heat", false));
        c.settings = active(Setpoint::Off);
        let d = c.run();
        assert_eq!(d.entities["trv"], target(Some(5.0), "heat", false));
    }

    #[test]
    fn frost_on_an_off_device_targets_frost_plus_two() {
        let mut c = Case::new();
        c.reading = fresh(6.5);
        let mut off = dev("off", &["off", "heat"], Some(21.0));
        off.mode = Some("off".into());
        c.devices = vec![off];
        assert_eq!(c.run().entities["off"], target(Some(9.0), "heat", false));
    }

    #[test]
    fn overheat_leaves_off_devices_alone() {
        let mut c = Case::new();
        c.reading = fresh(31.0);
        let mut off = dev("off", &["off", "heat"], Some(5.0));
        off.mode = Some("off".into());
        c.devices = vec![trv(), off.clone()];
        let d = c.run();
        assert_eq!(d.source, Source::Safety);
        assert!(d.entities.contains_key("trv"));
        assert!(!d.entities.contains_key("off"));
        c.settings = active(Setpoint::Off);
        let d = c.run();
        assert!(d.entities.contains_key("trv"));
        assert!(!d.entities.contains_key("off"));
    }

    #[test]
    fn off_never_touches_a_device_that_cannot_run_the_mode() {
        // A heating radiator in a cooling zone.
        let radiator = || dev("rad", &["heat", "off"], Some(20.0));
        let mut c = Case::new();
        c.mode = Some(Mode::Cool);
        c.devices = vec![radiator()];
        c.settings.rules.cool.as_mut().unwrap().blocks[0].setpoint = Setpoint::Off;
        let d = c.run();
        assert_eq!(d.source, Source::Schedule);
        assert!(d.entities.is_empty(), "cool Off block: {:?}", d.entities);
        c.holiday = true;
        let d = c.run();
        assert_eq!(d.source, Source::Holiday);
        assert!(d.entities.is_empty(), "holiday in cool: {:?}", d.entities);
        c.holiday = false;
        c.reading = fresh(15.0);
        let d = c.run();
        assert_eq!(d.cause, Some(SafetyCause::Overcool));
        assert!(d.entities.is_empty(), "overcool: {:?}", d.entities);
        assert!(!d.reason.contains("cannot stop cooling"), "{}", d.reason);
        c.settings.rules.cool.as_mut().unwrap().base = Setpoint::Off;
        c.reading = Reading::Missing;
        let d = c.run();
        assert_eq!(d.cause, Some(SafetyCause::Stale));
        assert!(d.entities.is_empty(), "stale in cool: {:?}", d.entities);

        // A cooling-only AC in a heating zone whose base is off.
        let mut c = Case::new();
        let mut ac = dev("ac", &["cool", "off"], Some(24.0));
        ac.mode = Some("cool".into());
        c.devices = vec![ac];
        let heat = c.settings.rules.heat.as_mut().unwrap();
        heat.blocks.clear();
        heat.base = Setpoint::Off;
        let d = c.run();
        assert_eq!(d.source, Source::Schedule);
        assert!(d.entities.is_empty(), "heat Off base: {:?}", d.entities);
        assert_eq!(
            device_target(Setpoint::Off, Mode::Heat, &caps(&["cool", "off"])),
            None
        );
    }

    #[test]
    fn overheat_ignored_in_cool_mode() {
        let mut c = Case::new();
        c.mode = Some(Mode::Cool);
        c.devices = vec![dev("ac", &["off", "cool"], Some(25.0))];
        c.reading = fresh(31.0);
        assert_eq!(c.run().source, Source::Schedule);
    }

    #[test]
    fn overcool_stops_cooling() {
        let mut c = Case::new();
        c.mode = Some(Mode::Cool);
        c.devices = vec![dev("ac", &["off", "cool"], Some(24.0))];
        c.reading = fresh(15.0);
        let d = c.run();
        assert_eq!(d.source, Source::Safety);
        assert_eq!(d.entities["ac"], target(None, "off", false));
    }

    #[test]
    fn overcool_without_off_uses_max_temp() {
        let mut c = Case::new();
        c.mode = Some(Mode::Cool);
        c.devices = vec![dev("ac", &["cool"], Some(24.0))];
        c.reading = fresh(15.0);
        let d = c.run();
        assert_eq!(d.entities["ac"], target(Some(30.0), "cool", false));
    }

    #[test]
    fn stale_active_heat_takes_lower_of_target_and_base() {
        let mut c = Case::new();
        c.reading = Reading::Stale { last: Some(19.0) };
        let d = c.run(); // target 21, base 17
        assert_eq!(d.source, Source::Safety);
        assert_eq!(d.entities["trv"], target(Some(17.0), "heat", false));
        assert!(d.reason.contains("stale"));
        c.devices = vec![dev("trv", &["off", "heat"], Some(15.0))];
        assert_eq!(c.run().entities["trv"].setpoint, Some(15.0));
        c.devices = vec![dev("trv", &["off", "heat"], Some(3.0))];
        assert_eq!(c.run().entities["trv"].setpoint, Some(7.0));
    }

    #[test]
    fn stale_active_heat_base_off_falls_back_to_frost() {
        let mut c = Case::new();
        c.settings = active(Setpoint::Off);
        c.reading = Reading::Missing;
        let d = c.run();
        assert_eq!(d.entities["trv"], target(Some(7.0), "heat", false));
    }

    #[test]
    fn stale_active_cool_takes_higher() {
        let mut c = Case::new();
        c.mode = Some(Mode::Cool);
        c.reading = Reading::Stale { last: None };
        c.devices = vec![dev("ac", &["off", "cool"], Some(22.0))];
        let d = c.run(); // base 26
        assert_eq!(d.entities["ac"], target(Some(26.0), "cool", false));
        c.devices = vec![dev("ac", &["off", "cool"], Some(27.0))];
        assert_eq!(c.run().entities["ac"].setpoint, Some(27.0));
    }

    #[test]
    fn stale_never_lets_schedule_override_or_away_through() {
        let mut c = Case::new();
        c.reading = Reading::Missing;
        c.overrides = vec![zone_ov(25.0, NOW + 600)];
        c.home = false;
        c.holiday = true;
        let d = c.run();
        assert_eq!(d.source, Source::Safety);
        assert_eq!(d.entities["trv"].setpoint, Some(17.0));
    }

    #[test]
    fn stale_pending_only_notifies() {
        let mut c = Case::new();
        c.settings.state = ZoneState::Pending;
        c.reading = Reading::Missing;
        let d = c.run();
        assert_eq!(d.source, Source::Safety);
        assert!(d.entities.is_empty());
        assert!(d.reason.contains("No temperature reading"));
    }

    #[test]
    fn stale_with_season_off_commands_nothing() {
        let mut c = Case::new();
        c.mode = None;
        c.reading = Reading::Missing;
        let d = c.run();
        assert_eq!(d.source, Source::Safety);
        assert!(d.entities.is_empty());
    }

    fn device_ov(id: &str, sp: f64) -> Override {
        Override {
            entity: Some(id.into()),
            setpoint: Some(Setpoint::Temp(sp)),
            mode: Some(Mode::Heat),
            until: NOW + 600,
            origin: OverrideOrigin::Hand,
        }
    }

    #[test]
    fn stale_no_season_beats_overrides() {
        let mut c = Case::new();
        c.mode = None;
        c.reading = Reading::Stale { last: Some(19.0) };
        let mut zone = zone_ov(25.0, NOW + 600);
        zone.mode = Some(Mode::Heat);
        c.overrides = vec![zone];
        let d = c.run();
        assert_eq!(d.source, Source::Safety);
        assert!(d.entities.is_empty());
        c.overrides = vec![device_ov("trv", 25.0)];
        let d = c.run();
        assert_eq!(d.source, Source::Safety);
        assert!(d.entities.is_empty());
    }

    #[test]
    fn stale_without_rules_for_mode_only_notifies() {
        let mut c = Case::new();
        c.settings.rules.heat = None;
        c.reading = Reading::Missing;
        let d = c.run();
        assert_eq!(d.source, Source::Safety);
        assert!(d.entities.is_empty());
    }

    #[test]
    fn stale_leaves_off_devices_alone() {
        let mut c = Case::new();
        c.reading = Reading::Missing;
        let mut off = dev("off", &["off", "heat"], Some(5.0));
        off.mode = Some("off".into());
        c.devices = vec![trv(), off];
        let d = c.run();
        assert!(d.entities.contains_key("trv"));
        assert!(!d.entities.contains_key("off"));
    }

    #[test]
    fn capture_skips_devices_with_unknown_mode() {
        let mut unknown = dev("u", &["heat"], Some(20.0));
        unknown.mode = None;
        let m = SafetyMemory::capture(&[trv(), unknown]);
        assert!(m.found.contains_key("trv"));
        assert!(!m.found.contains_key("u"));
    }

    #[test]
    fn device_override_holds_under_zone_override() {
        let mut c = Case::new();
        c.devices = vec![trv(), dev("trv2", &["off", "heat"], Some(21.0))];
        c.overrides = vec![zone_ov(22.0, NOW + 3600), device_ov("trv2", 24.0)];
        let d = c.run();
        assert_eq!(d.source, Source::Boost);
        assert_eq!(d.entities["trv"], target(Some(22.0), "heat", false));
        assert_eq!(d.entities["trv2"], target(Some(24.0), "heat", true));
    }

    #[test]
    fn zone_override_mode_without_setpoint_uses_its_own_mode_rules() {
        let mut c = Case::new();
        c.devices = vec![dev("ac", &["off", "cool"], Some(24.0))];
        c.overrides = vec![Override {
            entity: None,
            setpoint: None,
            mode: Some(Mode::Cool),
            until: NOW + 600,
            origin: OverrideOrigin::User,
        }];
        let d = c.run(); // heat season, cool rules block 20.5
        assert_eq!(d.source, Source::Override);
        assert_eq!(d.mode, Some(Mode::Cool));
        assert_eq!(d.entities["ac"], target(Some(20.5), "cool", false));
    }

    #[test]
    fn frost_never_lowers_a_warmer_device() {
        let mut c = Case::new();
        c.reading = fresh(6.5);
        c.devices = vec![
            dev("hot", &["heat"], Some(21.0)),
            dev("cold", &["heat"], Some(3.0)),
        ];
        let d = c.run();
        assert_eq!(d.entities["hot"].setpoint, Some(21.0));
        assert_eq!(d.entities["cold"].setpoint, Some(9.0));
        assert_eq!(d.reason, "Frost protection: 6.5 °C < 7.0 °C");
    }

    #[test]
    fn overcool_names_devices_it_cannot_stop() {
        let mut c = Case::new();
        c.mode = Some(Mode::Cool);
        c.reading = fresh(15.0);
        let mut stuck = dev("stuck", &["cool"], Some(24.0));
        stuck.caps.max_temp = None;
        c.devices = vec![dev("ac", &["off", "cool"], Some(24.0)), stuck];
        let d = c.run();
        assert!(d.entities.contains_key("ac"));
        assert!(!d.entities.contains_key("stuck"));
        assert!(
            d.reason.contains("cannot stop cooling: stuck"),
            "{}",
            d.reason
        );
    }

    #[test]
    fn pending_restores_after_safety() {
        let mut c = Case::new();
        c.settings.state = ZoneState::Pending;
        c.memory = Some(SafetyMemory {
            found: [("trv".to_string(), target(Some(19.0), "heat", false))].into(),
        });
        let d = c.run();
        assert_eq!(d.source, Source::Idle);
        assert_eq!(d.entities["trv"], target(Some(19.0), "heat", false));
        assert_eq!(d.reason, "Safety cleared: restoring previous settings");
    }

    #[test]
    fn pending_shows_would_do() {
        let mut c = Case::new();
        c.settings.state = ZoneState::Pending;
        let d = c.run();
        assert_eq!(d.source, Source::Idle);
        assert_eq!(d.setpoint, Some(Setpoint::Temp(20.5)));
        assert!(d.entities.is_empty());
        assert!(d.reason.starts_with("Waiting for approved rules"));
        c.settings.rules = Rules::default();
        let d = c.run();
        assert_eq!(d.setpoint, None);
        assert!(d.entities.is_empty());
    }

    #[test]
    fn pending_ignores_overrides() {
        let mut c = Case::new();
        c.settings.state = ZoneState::Pending;
        c.overrides = vec![zone_ov(25.0, NOW + 600)];
        let d = c.run();
        assert_eq!(d.source, Source::Idle);
        assert!(d.entities.is_empty());
    }

    #[test]
    fn zone_override_wins_over_schedule() {
        let mut c = Case::new();
        c.overrides = vec![zone_ov(22.0, NOW + 3600)];
        let d = c.run();
        assert_eq!(d.source, Source::Boost);
        assert_eq!(d.entities["trv"], target(Some(22.0), "heat", false));
        assert_eq!(d.reason, "Boost to 22.0 °C until 11:00");
    }

    #[test]
    fn expired_override_is_ignored() {
        let mut c = Case::new();
        c.overrides = vec![zone_ov(22.0, NOW)];
        let d = c.run();
        assert_eq!(d.source, Source::Schedule);
        assert_eq!(d.entities["trv"].setpoint, Some(20.5));
    }

    #[test]
    fn device_override_holds_only_that_device() {
        let mut c = Case::new();
        c.devices = vec![trv(), dev("trv2", &["off", "heat"], Some(21.0))];
        c.overrides = vec![Override {
            entity: Some("trv2".into()),
            setpoint: Some(Setpoint::Temp(23.0)),
            mode: None,
            until: NOW + 3600,
            origin: OverrideOrigin::Hand,
        }];
        let d = c.run();
        assert_eq!(d.source, Source::Schedule);
        assert_eq!(d.entities["trv"], target(Some(20.5), "heat", false));
        assert_eq!(d.entities["trv2"], target(Some(23.0), "heat", true));
        assert!(
            d.reason.ends_with("; trv2 held until 11:00"),
            "{}",
            d.reason
        );
    }

    #[test]
    fn device_override_the_device_cannot_run_holds_it_as_it_is() {
        let mut c = Case::new();
        c.overrides = vec![Override {
            entity: Some("trv".into()),
            setpoint: Some(Setpoint::Temp(24.0)),
            mode: Some(Mode::Cool),
            until: NOW + 3600,
            origin: OverrideOrigin::Hand,
        }];
        let d = c.run();
        assert_eq!(d.entities["trv"], target(Some(21.0), "heat", true));
        assert!(
            d.reason.ends_with("; trv held as it is until 11:00"),
            "{}",
            d.reason
        );
    }

    #[test]
    fn device_override_the_device_cannot_run_leaves_an_unknown_device_alone() {
        let mut c = Case::new();
        let mut unknown = trv();
        unknown.mode = None;
        c.devices = vec![unknown];
        c.overrides = vec![Override {
            entity: Some("trv".into()),
            setpoint: Some(Setpoint::Temp(24.0)),
            mode: Some(Mode::Cool),
            until: NOW + 3600,
            origin: OverrideOrigin::Hand,
        }];
        let d = c.run();
        assert!(!d.entities.contains_key("trv"));
        assert!(
            d.reason.ends_with("; trv held as it is until 11:00"),
            "{}",
            d.reason
        );
    }

    #[test]
    fn holiday_heat_is_frost() {
        let mut c = Case::new();
        c.holiday = true;
        let d = c.run();
        assert_eq!(d.source, Source::Holiday);
        assert_eq!(d.entities["trv"].setpoint, Some(7.0));
        c.mode = Some(Mode::Cool);
        c.devices = vec![dev("ac", &["off", "cool"], Some(24.0))];
        let d = c.run();
        assert_eq!(d.source, Source::Holiday);
        assert_eq!(d.entities["ac"], target(None, "off", false));
    }

    #[test]
    fn away_uses_eco_setpoint() {
        let mut c = Case::new();
        c.home = false;
        let d = c.run();
        assert_eq!(d.source, Source::Away);
        assert_eq!(d.entities["trv"].setpoint, Some(16.0));
        assert!(!d.home);
    }

    #[test]
    fn schedule_block_and_base() {
        let c = Case::new();
        let d = c.run();
        assert_eq!(d.source, Source::Schedule);
        assert_eq!(d.setpoint, Some(Setpoint::Temp(20.5)));
        assert_eq!(d.entities["trv"], target(Some(20.5), "heat", false));
        let mut c = Case::new();
        c.settings.rules.heat.as_mut().unwrap().blocks.clear();
        let d = c.run();
        assert_eq!(d.setpoint, Some(Setpoint::Temp(17.0)));
        assert_eq!(d.entities["trv"].setpoint, Some(17.0));
    }

    #[test]
    fn rules_off_setpoint_turns_device_off() {
        let mut c = Case::new();
        c.settings.rules.heat.as_mut().unwrap().blocks[0].setpoint = Setpoint::Off;
        let d = c.run();
        assert_eq!(d.entities["trv"], target(None, "off", false));
    }

    #[test]
    fn device_without_mode_is_left_out() {
        let mut c = Case::new();
        c.devices = vec![trv(), dev("ac", &["off", "cool"], Some(24.0))];
        let d = c.run();
        assert!(d.entities.contains_key("trv"));
        assert!(!d.entities.contains_key("ac"));
    }

    #[test]
    fn fit_clamps_and_rounds() {
        let c = caps(&["heat"]);
        assert_eq!(fit(30.3, &c), 30.0);
        assert_eq!(fit(20.26, &c), 20.5);
        assert_eq!(fit(20.24, &c), 20.0);
        assert_eq!(fit(2.0, &c), 5.0);
        let none = DeviceCaps {
            temp_step: None,
            ..c
        };
        assert_eq!(fit(20.26, &none), 20.5);
    }

    #[test]
    fn device_target_off_without_off_mode() {
        let heat_only = caps(&["heat"]);
        assert_eq!(
            device_target(Setpoint::Off, Mode::Heat, &heat_only),
            Some(target(Some(5.0), "heat", false))
        );
        assert_eq!(
            device_target(Setpoint::Temp(20.0), Mode::Cool, &heat_only),
            None
        );
        assert_eq!(device_target(Setpoint::Off, Mode::Cool, &heat_only), None);
    }

    #[test]
    fn no_mode_is_idle() {
        let mut c = Case::new();
        c.mode = None;
        let d = c.run();
        assert_eq!(d.source, Source::Idle);
        assert!(d.entities.is_empty());
    }

    #[test]
    fn no_rules_for_mode_is_idle() {
        let mut c = Case::new();
        c.settings.rules.heat = None;
        let d = c.run();
        assert_eq!(d.source, Source::Idle);
        assert!(d.entities.is_empty());
    }

    #[test]
    fn silent_sensor_is_mentioned_in_reason() {
        let mut c = Case::new();
        c.reading = Reading::Fresh {
            value: 19.0,
            age_s: 10,
            from_sensor: false,
            sensor_stale: true,
        };
        let d = c.run();
        assert_eq!(d.source, Source::Schedule);
        assert!(
            d.reason.ends_with(
                "; the room thermometer has been silent for over 6 h, using the devices' own readings"
            ),
            "{}",
            d.reason
        );
    }

    #[test]
    fn duration_reads_in_hours_and_minutes() {
        assert_eq!(duration(45), "45 min");
        assert_eq!(duration(360), "6 h");
        assert_eq!(duration(370), "6 h 10 min");
    }

    #[test]
    fn safety_memory_captures_devices() {
        let m = SafetyMemory::capture(&[trv()]);
        assert_eq!(m.found["trv"], target(Some(21.0), "heat", false));
    }

    fn eco_until(until: u64) -> Option<RoomEco> {
        Some(RoomEco {
            until,
            set_at: NOW - 60,
        })
    }

    #[test]
    fn room_eco_lowers_to_eco() {
        let mut c = Case::new();
        c.room_eco = eco_until(NOW + 3600);
        let d = c.run();
        assert_eq!(d.source, Source::Eco);
        assert_eq!(d.setpoint, Some(Setpoint::Temp(16.0)));
        assert_eq!(d.entities["trv"], target(Some(16.0), "heat", false));
        assert_eq!(d.reason, "Eco until Wed 21 Oct 11:00: 16.0 °C");
    }

    #[test]
    fn room_eco_and_away_together() {
        let mut c = Case::new();
        c.room_eco = eco_until(NOW + 3600);
        c.home = false;
        let d = c.run();
        assert_eq!(d.source, Source::Eco);
        assert_eq!(
            d.reason,
            "Eco until Wed 21 Oct 11:00 (nobody home): 16.0 °C"
        );
    }

    #[test]
    fn away_reason_names_eco() {
        let mut c = Case::new();
        c.home = false;
        assert_eq!(c.run().reason, "Nobody home: eco 16.0 °C");
    }

    #[test]
    fn eco_never_raises_heat() {
        let mut c = Case::new();
        c.settings.rules.heat.as_mut().unwrap().blocks.clear();
        c.settings.rules.heat.as_mut().unwrap().base = Setpoint::Temp(14.0);
        c.room_eco = eco_until(NOW + 3600);
        let d = c.run();
        assert_eq!(d.source, Source::Eco);
        assert_eq!(d.setpoint, Some(Setpoint::Temp(14.0)));
        assert_eq!(
            d.reason,
            "Eco until Wed 21 Oct 11:00: 14.0 °C; schedule already lower"
        );
    }

    #[test]
    fn eco_never_raises_cool() {
        let mut c = Case::new();
        c.mode = Some(Mode::Cool);
        c.devices = vec![Device {
            mode: Some("cool".into()),
            ..dev("ac", &["off", "cool"], Some(26.0))
        }];
        c.settings.rules.cool.as_mut().unwrap().blocks.clear();
        c.settings.rules.cool.as_mut().unwrap().base = Setpoint::Temp(30.0);
        c.home = false;
        let d = c.run();
        assert_eq!(d.source, Source::Away);
        assert_eq!(d.setpoint, Some(Setpoint::Temp(30.0)));
        assert!(
            d.reason.ends_with("; schedule already higher"),
            "{}",
            d.reason
        );
        c.settings.rules.cool.as_mut().unwrap().base = Setpoint::Temp(25.0);
        assert_eq!(c.run().setpoint, Some(Setpoint::Temp(28.0)));
    }

    #[test]
    fn eco_keeps_schedule_off() {
        let mut c = Case::new();
        c.settings.rules.heat.as_mut().unwrap().blocks[0].setpoint = Setpoint::Off;
        c.room_eco = eco_until(NOW + 3600);
        let d = c.run();
        assert_eq!(d.source, Source::Eco);
        assert_eq!(d.entities["trv"], target(None, "off", false));
    }

    #[test]
    fn eco_without_rules_for_mode_stays_idle() {
        let mut c = Case::new();
        c.settings.rules.heat = None;
        c.home = false;
        c.room_eco = eco_until(NOW + 3600);
        let d = c.run();
        assert_eq!(d.source, Source::Idle);
        assert!(d.entities.is_empty());
    }

    #[test]
    fn expired_room_eco_is_ignored() {
        let mut c = Case::new();
        c.room_eco = eco_until(NOW);
        let d = c.run();
        assert_eq!(d.source, Source::Schedule);
        assert_eq!(d.entities["trv"].setpoint, Some(20.5));
    }

    #[test]
    fn override_and_holiday_win_over_eco() {
        let mut c = Case::new();
        c.room_eco = eco_until(NOW + 3600);
        c.overrides = vec![zone_ov(22.0, NOW + 1800)];
        assert_eq!(c.run().source, Source::Boost);
        c.overrides.clear();
        c.holiday = true;
        assert_eq!(c.run().source, Source::Holiday);
    }

    #[test]
    fn hand_hold_survives_eco() {
        let mut c = Case::new();
        c.room_eco = eco_until(NOW + 3600);
        c.overrides = vec![Override {
            entity: Some("trv".into()),
            setpoint: Some(Setpoint::Temp(22.0)),
            mode: None,
            until: NOW + 1800,
            origin: OverrideOrigin::Hand,
        }];
        let d = c.run();
        assert_eq!(d.source, Source::Eco);
        assert_eq!(d.entities["trv"], target(Some(22.0), "heat", true));
    }

    #[test]
    fn eco_setpoint_rules() {
        let t = Setpoint::Temp;
        assert_eq!(eco_setpoint(Mode::Heat, 16.0, t(20.0)), t(16.0));
        assert_eq!(eco_setpoint(Mode::Heat, 16.0, t(14.0)), t(14.0));
        assert_eq!(eco_setpoint(Mode::Heat, 16.0, Setpoint::Off), Setpoint::Off);
        assert_eq!(eco_setpoint(Mode::Cool, 28.0, t(24.0)), t(28.0));
        assert_eq!(eco_setpoint(Mode::Cool, 28.0, t(30.0)), t(30.0));
    }
}
