//! On/off heaters: what one should do right now, and who heats the room in a decision.

use crate::decide::{ZoneInputs, clock, device_target, latest, live};
use crate::doc::{
    Decision, DeviceTarget, Heater, HeaterDrive, HeaterRun, Mode, Override, SafetyCause, Setpoint,
    Source, THERMOSTAT_BAND, ZoneState,
};
use crate::schedule::Reading;

/// One on/off heater as the hub sees it now.
#[derive(Clone, Debug, PartialEq)]
pub struct HeaterDevice {
    pub id: String,
    /// `None`: state unknown (counts as off).
    pub on: Option<bool>,
    pub available: bool,
    pub off_since: Option<u64>,
    pub on_since: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Want {
    On,
    Off,
    /// Wanted on, not allowed before this Unix time.
    Wait(u64),
}

/// What one heater should do. `chosen`: it is in the chosen set with a temperature setpoint.
/// `target`: the zone target. `reading`: the usable room reading. `bridge`: a hold heater
/// will be chosen again within its cool-down.
pub fn drive(
    h: &Heater,
    dev: &HeaterDevice,
    chosen: bool,
    target: Option<f64>,
    reading: Option<f64>,
    bridge: bool,
    now: u64,
) -> Want {
    let is_on = dev.on == Some(true);
    let wanted_on = if !chosen {
        h.drive == HeaterDrive::Hold && bridge && is_on
    } else if let Some(target) = target {
        match h.drive {
            HeaterDrive::Hold => true,
            HeaterDrive::Thermostat => thermostat_on(h, dev, target, reading, now),
        }
    } else {
        false
    };
    if !wanted_on {
        return Want::Off;
    }
    on_after_cool_down(h, dev, now)
}

/// On, unless the heater is off and its cool-down has not run out.
fn on_after_cool_down(h: &Heater, dev: &HeaterDevice, now: u64) -> Want {
    if dev.on != Some(true)
        && let Some(off_since) = dev.off_since
    {
        let ready = off_since + u64::from(h.min_off_min) * 60;
        if now < ready {
            return Want::Wait(ready);
        }
    }
    Want::On
}

fn thermostat_on(
    h: &Heater,
    dev: &HeaterDevice,
    target: f64,
    reading: Option<f64>,
    now: u64,
) -> bool {
    let Some(reading) = reading else {
        return false;
    };
    let is_on = dev.on == Some(true);
    if reading <= target - THERMOSTAT_BAND + 1e-9 {
        return true;
    }
    if reading >= target {
        // The band's own off: a minimum on-time may hold it.
        return is_on
            && dev
                .on_since
                .is_some_and(|since| now < since + u64::from(h.min_on_min) * 60);
    }
    is_on
}

/// Who heats the room: the chosen ids, whether the climate units outside them go off, and
/// whether the schedule's next blocks count (a `hold` heater bridging a short gap).
struct Who {
    ids: Vec<String>,
    units_follow: bool,
    schedule: bool,
}

impl Who {
    fn has(&self, id: &str) -> bool {
        self.ids.iter().any(|i| i == id)
    }
}

/// What one enabled heater does in this decision.
struct Step<'a> {
    heater: &'a Heater,
    dev: HeaterDevice,
    chosen: bool,
    hold: Option<&'a Override>,
    want: Want,
}

/// The post-pass worked out from the decision's source, mode and setpoint, before any change.
struct Plan<'a> {
    who: Who,
    unknown: Vec<String>,
    steps: Vec<Step<'a>>,
    unit_chosen: bool,
    /// Chosen climate units the zone lists that are unreachable: nothing heats in their place.
    units_down: Vec<String>,
    /// A chosen heater wants to heat and cannot (cooling down or unreachable).
    cannot_heat: bool,
    /// Some reachable heater is on.
    running: bool,
}

impl Plan<'_> {
    /// The climate units heat in place of a chosen heater that cannot.
    fn fills_in(&self, inp: &ZoneInputs, d: &Decision) -> bool {
        let Some(sp @ Setpoint::Temp(_)) = d.setpoint else {
            return false;
        };
        self.cannot_heat
            && !self.unit_chosen
            && self.units_down.is_empty()
            && inp.settings.fill_in
            && inp.devices.iter().any(|u| {
                hand_hold(inp, &u.id).is_none() && device_target(sp, Mode::Heat, &u.caps).is_some()
            })
    }
}

/// No climate unit heats under this decision: the post-pass turns them all off (none chosen,
/// none filling in), so the room is driven by its on/off heaters. The thermometer correction
/// does not apply then: an offset learned while another heater drives the room is wrong for
/// the units.
pub(crate) fn units_rest(inp: &ZoneInputs, d: &Decision) -> bool {
    plan(inp, d).is_some_and(|p| p.who.units_follow && !p.unit_chosen && !p.fills_in(inp, d))
}

/// Rewrite `d` for a zone with on/off heaters: who heats, the switches' targets, the reason.
/// No-op unless `d.mode == Some(Mode::Heat)`, the zone is not off, the season is heat (or the
/// decision is frost protection), and the zone has an enabled heater or a `heated_by` anywhere.
pub(crate) fn apply(inp: &ZoneInputs, d: &mut Decision) {
    let Some(p) = plan(inp, d) else {
        return;
    };
    let fills_in = p.fills_in(inp, d);
    if p.who.units_follow {
        for u in inp.devices {
            if p.who.has(&u.id) || hand_hold(inp, &u.id).is_some() {
                continue;
            }
            match device_target(Setpoint::Off, Mode::Heat, &u.caps) {
                Some(t) => d.entities.insert(u.id.clone(), t),
                None => d.entities.remove(&u.id),
            };
        }
    }
    let mut notes: String = p
        .unknown
        .iter()
        .map(|id| format!("; ignoring unknown heater {id}"))
        .collect();
    for st in &p.steps {
        let id = st.heater.entity.as_str();
        let mut run = HeaterRun {
            off_since: st.dev.off_since,
            on_since: st.dev.on_since,
            wait_until: None,
        };
        if st.dev.available {
            match st.want {
                Want::On if !st.chosen && st.hold.is_none() => {
                    notes.push_str(&format!("; {id} stays on until its next block"));
                }
                Want::Wait(t) => {
                    run.wait_until = Some(t);
                    notes.push_str(&format!("; {id} cooling down until {}", clock(inp, t)));
                }
                Want::On | Want::Off => {}
            }
            if let Some(o) = st.hold {
                notes.push_str(&format!("; {id} held until {}", clock(inp, o.until)));
            }
            d.entities.insert(
                id.to_string(),
                DeviceTarget {
                    setpoint: None,
                    mode: if st.want == Want::On { "heat" } else { "off" }.to_string(),
                    held: st.hold.is_some(),
                },
            );
        } else if st.chosen {
            notes.push_str(&format!("; {id} unreachable"));
        }
        d.heaters.insert(id.to_string(), run);
    }
    for id in &p.units_down {
        notes.push_str(&format!("; {id} unreachable"));
    }
    if (p.cannot_heat || !p.units_down.is_empty()) && !p.unit_chosen {
        if fills_in && let Some(Setpoint::Temp(t)) = d.setpoint {
            // The units heat as they would without the heaters, correction included.
            let sp = Setpoint::Temp(t + d.adjust.unwrap_or(0.0));
            for u in inp.devices {
                if hand_hold(inp, &u.id).is_some() {
                    continue;
                }
                if let Some(target) = device_target(sp, Mode::Heat, &u.caps) {
                    d.entities.insert(u.id.clone(), target);
                }
            }
            notes.push_str("; the climate units heat meanwhile");
        } else if !p.running {
            notes.push_str("; nothing heats the room");
        }
    }
    d.reason.push_str(&notes);
}

/// Work out the post-pass for `d`, or `None` when it leaves the decision as it is.
fn plan<'a>(inp: &'a ZoneInputs, d: &Decision) -> Option<Plan<'a>> {
    let s = inp.settings;
    if d.mode != Some(Mode::Heat) || s.state == ZoneState::Off {
        return None;
    }
    let frost = d.source == Source::Safety && d.cause == Some(SafetyCause::Frost);
    // Outside the heat season only frost protection drives an on/off heater: a heat boost or
    // override then runs on the climate units alone (a plug may drive something else then).
    if inp.mode != Some(Mode::Heat) && !frost {
        return None;
    }
    let heat = s.rules.get(Mode::Heat);
    let named = s.heated_by.is_some()
        || heat.is_some_and(|r| r.blocks.iter().any(|b| b.heated_by.is_some()));
    if !named && !s.heaters.iter().any(|h| h.enabled) {
        return None;
    }
    let mut unknown = Vec::new();
    let who = who(inp, d, &mut unknown)?;
    let unit_chosen = inp.devices.iter().any(|u| who.has(&u.id));
    let units_down: Vec<String> = who
        .ids
        .iter()
        .filter(|id| s.heater(id).is_none() && !inp.devices.iter().any(|u| u.id == **id))
        .cloned()
        .collect();
    let target = match d.setpoint {
        Some(Setpoint::Temp(t)) => Some(t),
        _ => None,
    };
    let reading = match inp.reading {
        Reading::Fresh { value, .. } => Some(value),
        Reading::Stale { .. } | Reading::Missing => None,
    };
    let mut steps = Vec::new();
    let (mut cannot_heat, mut running) = (false, false);
    // Overheat and stale-reading safety: every heater off, whatever a hand hold says.
    let safety_off = d.source == Source::Safety && d.cause != Some(SafetyCause::Frost);
    // Frost outside the heat season: a hold heater's plug may drive something else then, so it
    // gets nothing at all.
    let frost_off_season = frost && inp.mode != Some(Mode::Heat);
    for h in s.heaters.iter().filter(|h| h.enabled) {
        if frost_off_season && h.drive == HeaterDrive::Hold {
            continue;
        }
        let id = h.entity.as_str();
        let dev = inp
            .heaters
            .iter()
            .find(|x| x.id == id)
            .cloned()
            .unwrap_or_else(|| HeaterDevice {
                id: h.entity.clone(),
                on: None,
                available: false,
                off_since: None,
                on_since: None,
            });
        let chosen = who.has(id);
        let hold = hand_hold(inp, id).filter(|o| {
            if safety_off {
                return false;
            }
            if o.setpoint == Some(Setpoint::Off) {
                return true;
            }
            // A thermostat heater with no usable reading is off, even when turned on by hand.
            o.mode == Some(Mode::Heat) && (h.drive == HeaterDrive::Hold || reading.is_some())
        });
        let want = match hold {
            Some(o) if o.setpoint == Some(Setpoint::Off) => Want::Off,
            Some(_) => on_after_cool_down(h, &dev, inp.now),
            None => {
                let bridge = who.schedule
                    && !chosen
                    && h.drive == HeaterDrive::Hold
                    && dev.on == Some(true)
                    && heat.is_some_and(|r| {
                        let default_names = s
                            .heated_by
                            .as_ref()
                            .is_some_and(|ids| ids.iter().any(|i| i == id));
                        r.next_start_of(id, default_names, inp.local_now, h.min_off_min)
                            .is_some()
                    });
                drive(h, &dev, chosen, target, reading, bridge, inp.now)
            }
        };
        if dev.available {
            running |= want == Want::On;
            cannot_heat |= chosen && matches!(want, Want::Wait(_));
        } else {
            cannot_heat |= chosen && want != Want::Off;
        }
        steps.push(Step {
            heater: h,
            dev,
            chosen,
            hold,
            want,
        });
    }
    Some(Plan {
        who,
        unknown,
        steps,
        unit_chosen,
        units_down,
        cannot_heat,
        running,
    })
}

/// The chosen set for the decision's source (spec "Who heats"); `None` leaves it untouched.
fn who(inp: &ZoneInputs, d: &Decision, unknown: &mut Vec<String>) -> Option<Who> {
    let s = inp.settings;
    let units = || inp.devices.iter().map(|u| u.id.clone()).collect::<Vec<_>>();
    let nobody = Who {
        ids: Vec::new(),
        units_follow: false,
        schedule: false,
    };
    Some(match d.source {
        Source::Schedule | Source::Empty | Source::Boost | Source::Override => {
            let block = s
                .rules
                .get(Mode::Heat)
                .and_then(|r| r.heated_by_at(inp.local_now));
            let ids = match known(inp, block, unknown) {
                Some(ids) => ids,
                None => known(inp, s.heated_by.as_deref(), unknown).unwrap_or_else(units),
            };
            Who {
                ids,
                units_follow: true,
                schedule: true,
            }
        }
        Source::Eco | Source::Away | Source::Holiday => {
            let mut ids = known(inp, s.heated_by.as_deref(), unknown).unwrap_or_else(units);
            ids.retain(|id| s.heater(id).is_none_or(|h| h.drive != HeaterDrive::Hold));
            if ids.is_empty() {
                // Only hold heaters in the default: the climate units hold the eco target.
                ids = units();
            }
            Who {
                ids,
                units_follow: true,
                schedule: false,
            }
        }
        Source::Safety if d.cause == Some(SafetyCause::Frost) => {
            let mut ids = units();
            ids.extend(
                s.heaters
                    .iter()
                    .filter(|h| h.enabled && h.drive == HeaterDrive::Thermostat)
                    .map(|h| h.entity.clone()),
            );
            Who { ids, ..nobody }
        }
        Source::Safety => nobody,
        Source::Idle if s.state == ZoneState::Pending => nobody,
        Source::Idle => return None,
    })
}

/// The ids of `list` that can heat: an enabled heater of the zone or one of its climate units,
/// reachable or not (an unreachable one stays chosen, so nothing else heats in its place). A
/// disabled heater is dropped; anything else is added to `unknown`. `None` when none is left.
fn known(
    inp: &ZoneInputs,
    list: Option<&[String]>,
    unknown: &mut Vec<String>,
) -> Option<Vec<String>> {
    let mut kept = Vec::new();
    for id in list? {
        match inp.settings.heater(id) {
            Some(h) if h.enabled => kept.push(id.clone()),
            Some(_) => {}
            None if inp.devices.iter().any(|u| u.id == *id) || inp.unit_ids.contains(id) => {
                kept.push(id.clone());
            }
            None if !unknown.contains(id) => unknown.push(id.clone()),
            None => {}
        }
    }
    (!kept.is_empty()).then_some(kept)
}

/// The live per-device override of `id`, if any.
fn hand_hold<'a>(inp: &'a ZoneInputs, id: &str) -> Option<&'a Override> {
    latest(
        inp.overrides
            .iter()
            .filter(|o| o.entity.as_deref() == Some(id) && live(o, inp.now)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{Heater, HeaterDrive};

    const NOW: u64 = 1_800_000_000;
    fn th() -> Heater {
        Heater {
            entity: "rad".into(),
            ..Heater::default()
        }
    }
    fn hold(min_off: u32) -> Heater {
        Heater {
            entity: "stove".into(),
            drive: HeaterDrive::Hold,
            min_off_min: min_off,
            ..Heater::default()
        }
    }
    fn dev(on: bool) -> HeaterDevice {
        HeaterDevice {
            id: "x".into(),
            on: Some(on),
            available: true,
            off_since: None,
            on_since: None,
        }
    }

    #[test]
    fn thermostat_switches_on_below_the_band() {
        assert_eq!(
            drive(&th(), &dev(false), true, Some(20.0), Some(19.7), false, NOW),
            Want::On
        );
        assert_eq!(
            drive(&th(), &dev(false), true, Some(20.0), Some(19.8), false, NOW),
            Want::Off
        );
        assert_eq!(
            drive(&th(), &dev(true), true, Some(20.0), Some(19.8), false, NOW),
            Want::On
        );
        assert_eq!(
            drive(&th(), &dev(true), true, Some(20.0), Some(20.0), false, NOW),
            Want::Off
        );
    }
    #[test]
    fn thermostat_without_a_reading_is_off_even_inside_min_on() {
        let h = Heater {
            min_on_min: 10,
            ..th()
        };
        let d = HeaterDevice {
            on_since: Some(NOW - 60),
            ..dev(true)
        };
        assert_eq!(drive(&h, &d, true, Some(20.0), None, false, NOW), Want::Off);
    }
    #[test]
    fn thermostat_min_on_delays_the_off() {
        let h = Heater {
            min_on_min: 10,
            ..th()
        };
        let d = HeaterDevice {
            on_since: Some(NOW - 60),
            ..dev(true)
        };
        assert_eq!(
            drive(&h, &d, true, Some(20.0), Some(20.5), false, NOW),
            Want::On
        );
        let d = HeaterDevice {
            on_since: Some(NOW - 601),
            ..dev(true)
        };
        assert_eq!(
            drive(&h, &d, true, Some(20.0), Some(20.5), false, NOW),
            Want::Off
        );
    }
    #[test]
    fn not_chosen_is_off_and_min_on_does_not_keep_it() {
        let h = Heater {
            min_on_min: 10,
            ..th()
        };
        let d = HeaterDevice {
            on_since: Some(NOW - 60),
            ..dev(true)
        };
        assert_eq!(
            drive(&h, &d, false, Some(20.0), Some(15.0), false, NOW),
            Want::Off
        );
    }
    #[test]
    fn hold_is_on_whatever_the_reading() {
        assert_eq!(
            drive(
                &hold(15),
                &dev(false),
                true,
                Some(20.0),
                Some(25.0),
                false,
                NOW
            ),
            Want::On
        );
        assert_eq!(
            drive(&hold(15), &dev(false), true, Some(20.0), None, false, NOW),
            Want::On
        );
    }
    #[test]
    fn hold_bridges_a_short_gap_only_while_on() {
        assert_eq!(
            drive(&hold(15), &dev(true), false, None, None, true, NOW),
            Want::On
        );
        assert_eq!(
            drive(&hold(15), &dev(false), false, None, None, true, NOW),
            Want::Off
        );
    }
    #[test]
    fn cool_down_makes_it_wait() {
        let d = HeaterDevice {
            off_since: Some(NOW - 300),
            ..dev(false)
        };
        assert_eq!(
            drive(&hold(15), &d, true, Some(20.0), None, false, NOW),
            Want::Wait(NOW - 300 + 900)
        );
        let d = HeaterDevice {
            off_since: Some(NOW - 900),
            ..dev(false)
        };
        assert_eq!(
            drive(&hold(15), &d, true, Some(20.0), None, false, NOW),
            Want::On
        );
    }
    #[test]
    fn unknown_off_clock_means_no_cool_down_pending() {
        assert_eq!(
            drive(&hold(15), &dev(false), true, Some(20.0), None, false, NOW),
            Want::On
        );
    }
    #[test]
    fn an_unknown_state_counts_as_off_and_a_running_heater_never_waits() {
        let d = HeaterDevice {
            on: None,
            off_since: Some(NOW - 60),
            ..dev(false)
        };
        assert_eq!(
            drive(&hold(15), &d, true, Some(20.0), None, false, NOW),
            Want::Wait(NOW - 60 + 900)
        );
        let d = HeaterDevice {
            off_since: Some(NOW - 60),
            ..dev(true)
        };
        assert_eq!(
            drive(&hold(15), &d, true, Some(20.0), None, false, NOW),
            Want::On
        );
    }
    #[test]
    fn a_setpoint_off_zone_is_off() {
        assert_eq!(
            drive(&hold(0), &dev(true), true, None, Some(15.0), false, NOW),
            Want::Off
        );
    }
}
