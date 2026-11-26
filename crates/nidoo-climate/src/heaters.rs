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

/// Rewrite `d` for a zone with on/off heaters: who heats, the switches' targets, the reason.
/// No-op unless `d.mode == Some(Mode::Heat)`, the zone is not off, and the zone has an
/// enabled heater or a `heated_by` anywhere.
pub(crate) fn apply(inp: &ZoneInputs, d: &mut Decision) {
    let s = inp.settings;
    if d.mode != Some(Mode::Heat) || s.state == ZoneState::Off {
        return;
    }
    let heat = s.rules.get(Mode::Heat);
    let named = s.heated_by.is_some()
        || heat.is_some_and(|r| r.blocks.iter().any(|b| b.heated_by.is_some()));
    if !named && !s.heaters.iter().any(|h| h.enabled) {
        return;
    }
    let mut unknown = Vec::new();
    let Some(who) = who(inp, d, &mut unknown) else {
        return;
    };
    let chosen = |id: &str| who.ids.iter().any(|i| i == id);
    let unit_chosen = inp.devices.iter().any(|u| chosen(&u.id));
    if who.units_follow {
        for u in inp.devices {
            if chosen(&u.id) || hand_hold(inp, &u.id).is_some() {
                continue;
            }
            match device_target(Setpoint::Off, Mode::Heat, &u.caps) {
                Some(t) => d.entities.insert(u.id.clone(), t),
                None => d.entities.remove(&u.id),
            };
        }
    }

    let target = match d.setpoint {
        Some(Setpoint::Temp(t)) => Some(t),
        _ => None,
    };
    let reading = match inp.reading {
        Reading::Fresh { value, .. } => Some(value),
        Reading::Stale { .. } | Reading::Missing => None,
    };
    let mut notes: String = unknown
        .iter()
        .map(|id| format!("; ignoring unknown heater {id}"))
        .collect();
    // A chosen heater wants to heat and cannot; some heater is on.
    let mut cannot_heat = false;
    let mut running = false;
    for h in s.heaters.iter().filter(|h| h.enabled) {
        let id = h.entity.as_str();
        let unseen = HeaterDevice {
            id: h.entity.clone(),
            on: None,
            available: false,
            off_since: None,
            on_since: None,
        };
        let dev = inp.heaters.iter().find(|x| x.id == id).unwrap_or(&unseen);
        let is_chosen = chosen(id);
        let hold = hand_hold(inp, id)
            .filter(|o| o.setpoint == Some(Setpoint::Off) || o.mode == Some(Mode::Heat));
        let want = match hold {
            Some(o) if o.setpoint == Some(Setpoint::Off) => Want::Off,
            Some(_) => on_after_cool_down(h, dev, inp.now),
            None => {
                let bridge = who.schedule
                    && !is_chosen
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
                drive(h, dev, is_chosen, target, reading, bridge, inp.now)
            }
        };
        let mut run = HeaterRun {
            off_since: dev.off_since,
            on_since: dev.on_since,
            wait_until: None,
        };
        if dev.available {
            match want {
                Want::On => {
                    running = true;
                    if !is_chosen && hold.is_none() {
                        notes.push_str(&format!("; {id} stays on until its next block"));
                    }
                }
                Want::Wait(t) => {
                    run.wait_until = Some(t);
                    cannot_heat |= is_chosen;
                    notes.push_str(&format!("; {id} cooling down until {}", clock(inp, t)));
                }
                Want::Off => {}
            }
            if let Some(o) = hold {
                notes.push_str(&format!("; {id} held until {}", clock(inp, o.until)));
            }
            d.entities.insert(
                h.entity.clone(),
                DeviceTarget {
                    setpoint: None,
                    mode: if want == Want::On { "heat" } else { "off" }.to_string(),
                    held: hold.is_some(),
                },
            );
        } else if is_chosen {
            cannot_heat |= want != Want::Off;
            notes.push_str(&format!("; {id} unreachable"));
        }
        d.heaters.insert(h.entity.clone(), run);
    }

    if cannot_heat && !unit_chosen {
        let mut filled = false;
        if s.fill_in
            && let Some(sp @ Setpoint::Temp(_)) = d.setpoint
        {
            for u in inp.devices {
                if hand_hold(inp, &u.id).is_some() {
                    continue;
                }
                if let Some(t) = device_target(sp, Mode::Heat, &u.caps) {
                    d.entities.insert(u.id.clone(), t);
                    filled = true;
                }
            }
        }
        if filled {
            notes.push_str("; the climate units heat meanwhile");
        } else if !running {
            notes.push_str("; nothing heats the room");
        }
    }
    d.reason.push_str(&notes);
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

/// The ids of `list` that can heat: an enabled heater of the zone or one of its climate units.
/// A disabled heater is dropped; anything else is added to `unknown`. `None` when none is left.
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
            None if inp.devices.iter().any(|u| u.id == *id) => kept.push(id.clone()),
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
