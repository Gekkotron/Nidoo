//! Pure drive of one on/off heater: what it should do right now.

use crate::doc::{Heater, HeaterDrive, THERMOSTAT_BAND};

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
    if !is_on && let Some(off_since) = dev.off_since {
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
