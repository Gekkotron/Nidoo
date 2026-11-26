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
use crate::heaters::{self, HeaterDevice};
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
    /// Every climate unit the zone lists, reachable or not (`devices` holds the reachable ones).
    pub unit_ids: &'a [String],
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
    /// The zone's on/off heaters, enabled or not, reachable or not.
    pub heaters: &'a [HeaterDevice],
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
        heaters: BTreeMap::new(),
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
    heaters::apply(inp, &mut d);
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

pub(crate) fn live(o: &Override, now: u64) -> bool {
    o.until > now
}

pub(crate) fn latest<'a>(it: impl Iterator<Item = &'a Override>) -> Option<&'a Override> {
    it.max_by_key(|o| o.until)
}

pub(crate) fn clock(inp: &ZoneInputs, until: u64) -> String {
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
    if !fb.enabled || heaters::units_rest(inp, d) {
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
mod tests;
