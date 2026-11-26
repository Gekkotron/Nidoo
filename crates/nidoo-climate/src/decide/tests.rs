use super::*;
use crate::doc::{
    Block, Day, Heater, HeaterDrive, HeaterRun, HhMm, ModeRules, OverrideOrigin, RoomEco, Rules,
    ZoneSettings,
};

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
            heated_by: None,
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
    heaters: Vec<HeaterDevice>,
    /// The climate units the zone lists; `None`: the ids of `devices`.
    units: Option<Vec<String>>,
    local: NaiveDateTime,
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
            heaters: vec![],
            units: None,
            local: local(),
        }
    }

    /// Add an on/off heater to the zone, `dev` being how the hub sees it.
    fn heater(mut self, h: Heater, dev: HeaterDevice) -> Self {
        self.heaters.push(HeaterDevice {
            id: h.entity.clone(),
            ..dev
        });
        self.settings.heaters.push(h);
        self
    }

    /// Replace the heat blocks by one block, every day, from `from_h` to `to_h` o'clock.
    fn heat_block_by(self, from_h: u16, to_h: u16, sp: f64, ids: &[&str]) -> Self {
        self.heat_blocks(vec![block_by(from_h * 60, to_h * 60, sp, ids)])
    }

    fn heat_blocks(mut self, blocks: Vec<Block>) -> Self {
        self.settings.rules.heat.as_mut().unwrap().blocks = blocks;
        self
    }

    /// The room default.
    fn heated_by(mut self, ids: &[&str]) -> Self {
        self.settings.heated_by = Some(ids.iter().map(|i| i.to_string()).collect());
        self
    }

    /// The same Wednesday, at `h:m` local.
    fn at_hm(mut self, h: u32, m: u32) -> Self {
        self.local = local().date().and_hms_opt(h, m, 0).unwrap();
        self
    }

    fn mode(mut self, mode: Mode) -> Self {
        self.mode = Some(mode);
        self
    }

    fn reading(mut self, r: Reading) -> Self {
        self.reading = r;
        self
    }

    fn run(&self) -> Decision {
        let units = self
            .units
            .clone()
            .unwrap_or_else(|| self.devices.iter().map(|d| d.id.clone()).collect());
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
            unit_ids: &units,
            local_now: self.local,
            now: NOW,
            safety_memory: self.memory.as_ref(),
            previous_cause: self.previous,
            room_eco: self.room_eco.as_ref(),
            thermometer: self.thermometer,
            feedback: self.feedback.as_ref(),
            room_empty: self.room_empty,
            heaters: &self.heaters,
        })
    }
}

/// A block every day from minute `from` to minute `to`, heated by `ids`.
fn block_by(from: u16, to: u16, sp: f64, ids: &[&str]) -> Block {
    Block {
        days: all_days(),
        from: HhMm(from),
        to: HhMm(to),
        setpoint: Setpoint::Temp(sp),
        evidence: None,
        heated_by: Some(ids.iter().map(|i| i.to_string()).collect()),
    }
}

/// A hold heater with a 15 min cool-down.
fn stove() -> Heater {
    Heater {
        entity: "stove".into(),
        drive: HeaterDrive::Hold,
        min_off_min: 15,
        ..Heater::default()
    }
}

/// A thermostat heater.
fn rad() -> Heater {
    Heater {
        entity: "rad".into(),
        ..Heater::default()
    }
}

/// A reachable switch, off, with no clocks.
fn off() -> HeaterDevice {
    HeaterDevice {
        id: String::new(),
        on: Some(false),
        available: true,
        off_since: None,
        on_since: None,
    }
}

fn on() -> HeaterDevice {
    HeaterDevice {
        on: Some(true),
        ..off()
    }
}

fn off_since(t: u64) -> HeaterDevice {
    HeaterDevice {
        off_since: Some(t),
        ..off()
    }
}

/// A hand change on a switch.
fn switch_ov(id: &str, setpoint: Option<Setpoint>, mode: Option<Mode>) -> Override {
    Override {
        entity: Some(id.into()),
        setpoint,
        mode,
        until: NOW + 3600,
        origin: OverrideOrigin::Hand,
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

fn run_of(off_since: Option<u64>, wait_until: Option<u64>) -> HeaterRun {
    HeaterRun {
        off_since,
        on_since: None,
        wait_until,
    }
}

#[test]
fn zone_without_heaters_is_unchanged() {
    let expected = Decision {
        zone: "z".into(),
        mode: Some(Mode::Heat),
        setpoint: Some(Setpoint::Temp(20.5)),
        source: Source::Schedule,
        cause: None,
        reason: "Schedule block 1: 20.5 °C".into(),
        reading: Some(19.0),
        reading_age_s: Some(30),
        outdoor_mean: Some(8.0),
        home: true,
        holiday: false,
        applied: false,
        at: NOW,
        entities: BTreeMap::from([("trv".into(), target(Some(20.5), "heat", false))]),
        eco_restore: BTreeMap::new(),
        adjust: None,
        heaters: BTreeMap::new(),
    };
    assert_eq!(Case::new().run(), expected);
    // A disabled heater and no `heated_by`: the engine leaves the room as it was.
    let disabled = Heater {
        enabled: false,
        ..stove()
    };
    assert_eq!(Case::new().heater(disabled, on()).run(), expected);
}

#[test]
fn block_names_the_stove_unit_goes_off() {
    let d = Case::new()
        .heater(stove(), off())
        .heat_block_by(9, 18, 20.0, &["stove"])
        .at_hm(10, 0)
        .run();
    assert_eq!(d.entities["stove"], target(None, "heat", false));
    assert_eq!(d.entities["trv"], target(None, "off", false));
    assert_eq!(d.heaters["stove"], HeaterRun::default());
    assert_eq!(d.reason, "Schedule block 1: 20.0 °C");
}

#[test]
fn base_uses_the_room_default() {
    let block = Block {
        heated_by: None,
        ..block_by(540, 1080, 20.0, &[])
    };
    let d = Case::new()
        .heater(rad(), off())
        .heated_by(&["rad"])
        .heat_blocks(vec![block])
        .reading(fresh(16.0))
        .at_hm(20, 0)
        .run();
    assert_eq!(d.entities["rad"], target(None, "heat", false));
    assert_eq!(d.entities["trv"], target(None, "off", false));
    assert_eq!(d.reason, "Schedule base: 17.0 °C");
}

#[test]
fn no_default_means_the_units_heat_and_heaters_are_off() {
    let mut c = Case::new()
        .heater(rad(), off())
        .reading(fresh(12.0))
        .at_hm(23, 0);
    c.settings.rules.heat.as_mut().unwrap().base = Setpoint::Temp(15.0);
    let d = c.run();
    assert_eq!(d.entities["trv"], target(Some(15.0), "heat", false));
    assert_eq!(d.entities["rad"], target(None, "off", false));
    assert_eq!(d.reason, "Schedule base: 15.0 °C");
}

#[test]
fn eco_drops_hold_heaters() {
    let mut c = Case::new()
        .heater(stove(), on())
        .heated_by(&["stove", "trv"]);
    c.home = false;
    let d = c.run();
    assert_eq!(d.source, Source::Away);
    assert_eq!(d.entities["stove"], target(None, "off", false));
    assert_eq!(d.entities["trv"], target(Some(16.0), "heat", false));
    assert_eq!(d.reason, "Nobody home: eco 16.0 °C");
}

#[test]
fn eco_with_only_a_hold_default_lets_the_units_heat() {
    let mut c = Case::new().heater(stove(), on()).heated_by(&["stove"]);
    c.home = false;
    let d = c.run();
    assert_eq!(d.source, Source::Away);
    assert_eq!(d.entities["stove"], target(None, "off", false));
    assert_eq!(d.entities["trv"], target(Some(16.0), "heat", false));
    assert_eq!(d.reason, "Nobody home: eco 16.0 °C");
}

#[test]
fn frost_turns_thermostat_heaters_on_not_hold() {
    let d = Case::new()
        .heater(rad(), off())
        .heater(stove(), off())
        .heated_by(&["stove"])
        .reading(fresh(5.0))
        .run();
    assert_eq!(d.cause, Some(SafetyCause::Frost));
    assert_eq!(d.entities["rad"], target(None, "heat", false));
    assert_eq!(d.entities["stove"], target(None, "off", false));
    assert_eq!(d.entities["trv"], target(Some(21.0), "heat", false));
    assert_eq!(d.reason, "Frost protection: 5.0 °C < 7.0 °C");
}

#[test]
fn overheat_turns_every_heater_off() {
    let d = Case::new()
        .heater(rad(), on())
        .heater(stove(), on())
        .heat_block_by(9, 18, 20.0, &["stove", "rad"])
        .reading(fresh(31.0))
        .run();
    assert_eq!(d.cause, Some(SafetyCause::Overheat));
    assert_eq!(d.entities["rad"], target(None, "off", false));
    assert_eq!(d.entities["stove"], target(None, "off", false));
    assert_eq!(d.entities["trv"], target(Some(17.0), "heat", false));
    assert_eq!(d.reason, "Overheat protection: 31.0 °C > 30.0 °C");
}

#[test]
fn stale_reading_turns_heaters_off() {
    let d = Case::new()
        .heater(rad(), on())
        .heater(stove(), on())
        .heat_block_by(9, 18, 20.0, &["stove", "rad"])
        .reading(Reading::Stale { last: Some(19.0) })
        .run();
    assert_eq!(d.cause, Some(SafetyCause::Stale));
    assert_eq!(d.entities["rad"], target(None, "off", false));
    assert_eq!(d.entities["stove"], target(None, "off", false));
    assert_eq!(d.entities["trv"], target(Some(17.0), "heat", false));
}

#[test]
fn pending_zone_keeps_heaters_off() {
    let mut c = Case::new()
        .heater(rad(), on())
        .heater(stove(), on())
        .heat_block_by(9, 18, 20.0, &["stove", "rad"])
        .reading(fresh(15.0));
    c.settings.state = ZoneState::Pending;
    let d = c.run();
    assert_eq!(
        d.entities,
        BTreeMap::from([
            ("rad".into(), target(None, "off", false)),
            ("stove".into(), target(None, "off", false)),
        ])
    );
    assert_eq!(d.reason, "Waiting for approved rules; would set 20.0 °C");
}

#[test]
fn cool_mode_leaves_heaters_out() {
    let d = Case::new()
        .heater(stove(), on())
        .heated_by(&["stove"])
        .mode(Mode::Cool)
        .run();
    assert!(!d.entities.contains_key("stove"));
    assert!(d.heaters.is_empty());
}

#[test]
fn zone_off_leaves_heaters_out() {
    let mut c = Case::new()
        .heater(stove(), on())
        .heat_block_by(9, 18, 20.0, &["stove"]);
    c.settings.state = ZoneState::Off;
    let d = c.run();
    assert!(d.entities.is_empty());
    assert!(d.heaters.is_empty());
    assert_eq!(d.reason, "Zone is off");
}

#[test]
fn boost_uses_what_the_schedule_would_choose() {
    let mut c = Case::new()
        .heater(stove(), off())
        .heat_block_by(9, 18, 20.0, &["stove"]);
    c.overrides = vec![zone_ov(22.0, NOW + 3600)];
    let d = c.run();
    assert_eq!(d.source, Source::Boost);
    assert_eq!(d.entities["stove"], target(None, "heat", false));
    assert_eq!(d.entities["trv"], target(None, "off", false));
    assert_eq!(d.reason, "Boost to 22.0 °C until 11:00");
}

#[test]
fn cooling_down_stove_waits_and_the_unit_fills_in() {
    let d = Case::new()
        .heater(stove(), off_since(NOW - 300))
        .heat_block_by(9, 18, 20.0, &["stove"])
        .at_hm(10, 0)
        .run();
    assert_eq!(d.entities["stove"], target(None, "off", false));
    assert_eq!(d.heaters["stove"], run_of(Some(NOW - 300), Some(NOW + 600)));
    assert_eq!(d.entities["trv"], target(Some(20.0), "heat", false));
    assert_eq!(
        d.reason,
        "Schedule block 1: 20.0 °C; stove cooling down until 10:10; \
             the climate units heat meanwhile"
    );
}

#[test]
fn fill_in_off_says_nothing_heats() {
    let mut c = Case::new()
        .heater(stove(), off_since(NOW - 300))
        .heat_block_by(9, 18, 20.0, &["stove"]);
    c.settings.fill_in = false;
    let d = c.run();
    assert_eq!(d.entities["stove"], target(None, "off", false));
    assert_eq!(d.entities["trv"], target(None, "off", false));
    assert_eq!(
        d.reason,
        "Schedule block 1: 20.0 °C; stove cooling down until 10:10; nothing heats the room"
    );
}

#[test]
fn unreachable_heater_fills_in() {
    let unreachable = HeaterDevice {
        available: false,
        off_since: Some(NOW - 7200),
        ..off()
    };
    let d = Case::new()
        .heater(stove(), unreachable)
        .heat_block_by(9, 18, 20.0, &["stove"])
        .run();
    assert!(!d.entities.contains_key("stove"));
    assert_eq!(d.heaters["stove"], run_of(Some(NOW - 7200), None));
    assert_eq!(d.entities["trv"], target(Some(20.0), "heat", false));
    assert_eq!(
        d.reason,
        "Schedule block 1: 20.0 °C; stove unreachable; the climate units heat meanwhile"
    );
}

#[test]
fn running_stove_never_fills_in() {
    let d = Case::new()
        .heater(stove(), on())
        .heat_block_by(9, 18, 20.0, &["stove"])
        .reading(fresh(12.0))
        .run();
    assert_eq!(d.entities["stove"], target(None, "heat", false));
    assert_eq!(d.entities["trv"], target(None, "off", false));
    assert_eq!(d.reason, "Schedule block 1: 20.0 °C");
}

#[test]
fn cool_down_blocks_a_boost_and_frost() {
    let mut c = Case::new()
        .heater(stove(), off_since(NOW - 300))
        .heat_block_by(9, 18, 20.0, &["stove"]);
    c.overrides = vec![zone_ov(22.0, NOW + 3600)];
    let d = c.run();
    assert_eq!(d.entities["stove"], target(None, "off", false));
    assert_eq!(d.heaters["stove"].wait_until, Some(NOW + 600));
    assert_eq!(d.entities["trv"], target(Some(22.0), "heat", false));
    assert_eq!(
        d.reason,
        "Boost to 22.0 °C until 11:00; stove cooling down until 10:10; \
             the climate units heat meanwhile"
    );

    let slow_rad = Heater {
        min_off_min: 15,
        ..rad()
    };
    let d = Case::new()
        .heater(slow_rad, off_since(NOW - 300))
        .reading(fresh(5.0))
        .run();
    assert_eq!(d.entities["rad"], target(None, "off", false));
    assert_eq!(d.heaters["rad"].wait_until, Some(NOW + 600));
    assert_eq!(d.entities["trv"], target(Some(21.0), "heat", false));
    assert_eq!(
        d.reason,
        "Frost protection: 5.0 °C < 7.0 °C; rad cooling down until 10:10"
    );
}

#[test]
fn hold_bridges_a_gap_shorter_than_its_cool_down() {
    let d = Case::new()
        .heater(stove(), on())
        .heat_blocks(vec![
            block_by(540, 720, 20.0, &["stove"]),
            block_by(730, 840, 20.0, &["stove"]),
        ])
        .at_hm(12, 5)
        .run();
    assert_eq!(d.entities["stove"], target(None, "heat", false));
    assert_eq!(d.entities["trv"], target(Some(17.0), "heat", false));
    assert_eq!(
        d.reason,
        "Schedule base: 17.0 °C; stove stays on until its next block"
    );
    // A gap longer than the cool-down: off.
    let d = Case::new()
        .heater(stove(), on())
        .heat_blocks(vec![
            block_by(540, 720, 20.0, &["stove"]),
            block_by(750, 840, 20.0, &["stove"]),
        ])
        .at_hm(12, 5)
        .run();
    assert_eq!(d.entities["stove"], target(None, "off", false));
    assert_eq!(d.reason, "Schedule base: 17.0 °C");
}

#[test]
fn hand_off_holds_the_heater() {
    let mut c = Case::new()
        .heater(stove(), off())
        .heat_block_by(9, 18, 20.0, &["stove"]);
    c.overrides = vec![switch_ov("stove", Some(Setpoint::Off), None)];
    let d = c.run();
    assert_eq!(d.entities["stove"], target(None, "off", true));
    assert_eq!(d.entities["trv"], target(None, "off", false));
    assert_eq!(
        d.reason,
        "Schedule block 1: 20.0 °C; stove held until 11:00"
    );
}

#[test]
fn overheat_beats_a_hand_hold_on_a_switch() {
    for ov in [
        switch_ov("stove", None, Some(Mode::Heat)),
        switch_ov("stove", Some(Setpoint::Off), None),
    ] {
        let mut c = Case::new()
            .heater(stove(), on())
            .heat_block_by(9, 18, 20.0, &["stove"])
            .reading(fresh(31.0));
        c.overrides = vec![ov];
        let d = c.run();
        assert_eq!(d.cause, Some(SafetyCause::Overheat));
        assert_eq!(d.entities["stove"], target(None, "off", false));
        assert_eq!(d.reason, "Overheat protection: 31.0 °C > 30.0 °C");
    }
}

#[test]
fn stale_reading_beats_a_hand_hold_on_a_thermostat_heater() {
    let mut c = Case::new()
        .heater(rad(), on())
        .heated_by(&["rad"])
        .reading(Reading::Stale { last: Some(15.0) });
    c.overrides = vec![switch_ov("rad", None, Some(Mode::Heat))];
    let d = c.run();
    assert_eq!(d.cause, Some(SafetyCause::Stale));
    assert_eq!(d.entities["rad"], target(None, "off", false));
    assert!(!d.reason.contains("held"), "{}", d.reason);
}

#[test]
fn frost_keeps_a_hand_hold_off_and_the_cool_down() {
    let mut c = Case::new().heater(rad(), on()).reading(fresh(5.0));
    c.overrides = vec![switch_ov("rad", Some(Setpoint::Off), None)];
    let d = c.run();
    assert_eq!(d.cause, Some(SafetyCause::Frost));
    assert_eq!(d.entities["rad"], target(None, "off", true));
    assert_eq!(d.entities["trv"], target(Some(21.0), "heat", false));

    let slow_rad = Heater {
        min_off_min: 15,
        ..rad()
    };
    let mut c = Case::new()
        .heater(slow_rad, off_since(NOW - 300))
        .reading(fresh(5.0));
    c.overrides = vec![switch_ov("rad", None, Some(Mode::Heat))];
    let d = c.run();
    assert_eq!(d.entities["rad"], target(None, "off", true));
    assert_eq!(d.heaters["rad"].wait_until, Some(NOW + 600));
}

#[test]
fn frost_out_of_season_leaves_hold_heaters_out() {
    for mode in [Some(Mode::Cool), None] {
        let mut c = Case::new()
            .heater(rad(), off())
            .heater(stove(), on())
            .reading(fresh(5.0));
        c.mode = mode;
        let d = c.run();
        assert_eq!(d.cause, Some(SafetyCause::Frost));
        assert_eq!(d.entities["rad"], target(None, "heat", false));
        assert!(!d.entities.contains_key("stove"));
        assert!(!d.heaters.contains_key("stove"));
        assert_eq!(d.heaters["rad"], HeaterRun::default());
    }
    // In season the stove is told off.
    let d = Case::new().heater(stove(), on()).reading(fresh(5.0)).run();
    assert_eq!(d.entities["stove"], target(None, "off", false));
}

#[test]
fn hand_on_holds_but_respects_cool_down() {
    // Base time: the stove is not chosen, the hand turned it on.
    let mut c = Case::new()
        .heater(stove(), off_since(NOW - 300))
        .heat_block_by(9, 18, 20.0, &["stove"])
        .at_hm(20, 0);
    c.overrides = vec![switch_ov("stove", None, Some(Mode::Heat))];
    let d = c.run();
    assert_eq!(d.entities["stove"], target(None, "off", true));
    assert_eq!(d.heaters["stove"], run_of(Some(NOW - 300), Some(NOW + 600)));
    assert_eq!(
        d.reason,
        "Schedule base: 17.0 °C; stove cooling down until 20:10; stove held until 21:00"
    );
    let mut c = Case::new()
        .heater(stove(), off())
        .heat_block_by(9, 18, 20.0, &["stove"])
        .at_hm(20, 0);
    c.overrides = vec![switch_ov("stove", None, Some(Mode::Heat))];
    let d = c.run();
    assert_eq!(d.entities["stove"], target(None, "heat", true));
    assert_eq!(d.entities["trv"], target(Some(17.0), "heat", false));
}

#[test]
fn unknown_heated_by_id_falls_back() {
    let d = Case::new()
        .heater(stove(), on())
        .heat_block_by(9, 18, 20.0, &["gone"])
        .run();
    assert_eq!(d.entities["trv"], target(Some(20.0), "heat", false));
    assert_eq!(d.entities["stove"], target(None, "off", false));
    assert_eq!(
        d.reason,
        "Schedule block 1: 20.0 °C; ignoring unknown heater gone"
    );
    // The room default is filtered the same way, then the climate units heat.
    let d = Case::new()
        .heater(stove(), on())
        .heated_by(&["gone", "stove"])
        .heat_block_by(9, 18, 20.0, &["gone"])
        .run();
    assert_eq!(d.entities["stove"], target(None, "heat", false));
    assert_eq!(d.entities["trv"], target(None, "off", false));
    assert_eq!(
        d.reason,
        "Schedule block 1: 20.0 °C; ignoring unknown heater gone"
    );
}

#[test]
fn a_heat_boost_out_of_season_leaves_heaters_out() {
    for mode in [Some(Mode::Cool), None] {
        let mut c = Case::new()
            .heater(stove(), off())
            .heater(rad(), off())
            .heat_block_by(9, 18, 20.0, &["stove", "rad"]);
        c.mode = mode;
        c.overrides = vec![Override {
            mode: Some(Mode::Heat),
            ..zone_ov(22.0, NOW + 3600)
        }];
        let d = c.run();
        assert_eq!(d.source, Source::Boost, "{mode:?}");
        assert_eq!(d.mode, Some(Mode::Heat));
        assert!(!d.entities.contains_key("stove"), "{mode:?}: {d:?}");
        assert!(!d.entities.contains_key("rad"), "{mode:?}: {d:?}");
        assert!(d.heaters.is_empty(), "{mode:?}: {d:?}");
        // The climate units run the boost as they always did.
        assert_eq!(d.entities["trv"], target(Some(22.0), "heat", false));
        assert_eq!(d.reason, "Boost to 22.0 °C until 11:00");
    }
}

#[test]
fn an_unreachable_unit_named_in_heated_by_is_not_replaced() {
    let mut c = Case::new()
        .heater(stove(), off())
        .heated_by(&["stove"])
        .heat_block_by(9, 18, 20.0, &["trv"]);
    // The zone lists the unit; it is offline, so the hub hands no device for it.
    c.units = Some(vec!["trv".into()]);
    c.devices = vec![];
    let d = c.run();
    assert_eq!(d.entities["stove"], target(None, "off", false));
    assert!(!d.entities.contains_key("trv"));
    assert!(d.reason.contains("trv unreachable"), "{}", d.reason);
    assert_eq!(
        d.reason,
        "Schedule block 1: 20.0 °C; trv unreachable; nothing heats the room"
    );
}

#[test]
fn an_enabled_heater_the_hub_does_not_see_is_unreachable() {
    // Listed and enabled, but no switch for it (its entity is gone).
    let mut c = Case::new().heat_block_by(9, 18, 20.0, &["stove"]);
    c.settings.heaters.push(stove());
    let d = c.run();
    assert!(!d.entities.contains_key("stove"));
    assert_eq!(d.heaters["stove"], HeaterRun::default());
    assert_eq!(d.entities["trv"], target(Some(20.0), "heat", false));
    assert_eq!(
        d.reason,
        "Schedule block 1: 20.0 °C; stove unreachable; the climate units heat meanwhile"
    );
}

#[test]
fn disabled_heater_is_not_chosen() {
    let disabled = Heater {
        enabled: false,
        ..stove()
    };
    let d = Case::new()
        .heater(disabled, on())
        .heat_block_by(9, 18, 20.0, &["stove"])
        .run();
    assert_eq!(d.entities["trv"], target(Some(20.0), "heat", false));
    assert!(!d.entities.contains_key("stove"));
    assert!(d.heaters.is_empty());
    assert_eq!(d.reason, "Schedule block 1: 20.0 °C");
}

#[test]
fn no_correction_while_only_heaters_heat() {
    // A previous offset of -1.0; the thermometer is over the band, but the stove drives
    // the room and the unit is off: no step, no offset, no note.
    let mut c = corrected(19.6, 30)
        .heater(stove(), on())
        .heat_block_by(9, 18, 19.0, &["stove"]);
    let previous = FeedbackState {
        target: 19.0,
        offset: -1.0,
        since: NOW - 7200,
        last_step_at: Some(NOW - 7200),
        reached: true,
    };
    c.feedback = Some(previous);
    let d = c.run();
    assert_eq!(d.adjust, None);
    assert_eq!(d.entities["trv"], target(None, "off", false));
    assert_eq!(d.entities["stove"], target(None, "heat", false));
    assert_eq!(d.reason, "Schedule block 1: 19.0 °C");
    // The hub's state goes back to 0 instead of stepping on.
    let kept = FeedbackState::after(
        c.feedback.as_ref(),
        &d,
        &c.settings.feedback,
        c.thermometer,
        NOW,
    )
    .unwrap();
    assert_eq!(kept.offset, 0.0);

    // The stove cooling down: the unit fills in, corrected as it would be without heaters.
    let d = corrected(19.6, 30)
        .heater(stove(), off_since(NOW - 300))
        .heat_block_by(9, 18, 19.0, &["stove"])
        .run();
    assert_eq!(d.adjust, Some(-0.5));
    assert_eq!(d.entities["trv"], target(Some(18.5), "heat", false));
    assert!(
        d.reason.contains("above 19.5 °C: AC 18.5 °C")
            && d.reason.ends_with("; the climate units heat meanwhile"),
        "{}",
        d.reason
    );
}

#[test]
fn correction_never_touches_a_heater() {
    // The room thermometer reads 19.6 (the correction lowers the unit), the decision's
    // reading is 18.0 (the radiator runs).
    let d = corrected(19.6, 30)
        .heater(rad(), off())
        .heated_by(&["rad", "trv"])
        .reading(fresh(18.0))
        .run();
    assert_eq!(d.adjust, Some(-0.5));
    assert_eq!(d.entities["trv"].setpoint, Some(18.5));
    assert_eq!(d.entities["rad"], target(None, "heat", false));
}
