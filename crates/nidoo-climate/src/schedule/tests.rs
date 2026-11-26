use super::*;
use crate::doc::{Block, HhMm};

fn dt(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
}

fn block(days: &[Day], from: u16, to: u16, t: f64) -> Block {
    Block {
        days: days.to_vec(),
        from: HhMm(from),
        to: HhMm(to),
        setpoint: Setpoint::Temp(t),
        evidence: None,
        heated_by: None,
    }
}

fn rules(blocks: Vec<Block>) -> ModeRules {
    ModeRules {
        base: Setpoint::Temp(17.0),
        blocks,
    }
}

fn weekday_rules() -> ModeRules {
    let days = [Day::Mon, Day::Tue, Day::Wed, Day::Thu, Day::Fri];
    rules(vec![block(&days, 420, 1350, 20.5)])
}

#[test]
fn at_inside_and_outside_block() {
    assert_eq!(
        weekday_rules().at(dt("2026-10-21 07:00")),
        (Setpoint::Temp(20.5), Some(0))
    ); // Wed
    assert_eq!(
        weekday_rules().at(dt("2026-10-21 06:59")),
        (Setpoint::Temp(17.0), None)
    );
    assert_eq!(
        weekday_rules().at(dt("2026-10-21 22:30")),
        (Setpoint::Temp(17.0), None)
    );
    assert_eq!(
        weekday_rules().at(dt("2026-10-24 10:00")),
        (Setpoint::Temp(17.0), None)
    ); // Sat
}

#[test]
fn at_handles_24_00_end() {
    let r = rules(vec![block(&[Day::Fri], 1320, 1440, 19.0)]);
    assert_eq!(
        r.at(dt("2026-10-23 23:59")),
        (Setpoint::Temp(19.0), Some(0))
    );
    assert_eq!(r.at(dt("2026-10-24 00:00")), (Setpoint::Temp(17.0), None));
}

#[test]
fn next_change_same_day_and_wraps_week() {
    assert_eq!(
        weekday_rules().next_change(dt("2026-10-21 08:00")),
        Some(dt("2026-10-21 22:30"))
    );
    assert_eq!(
        weekday_rules().next_change(dt("2026-10-23 23:00")),
        Some(dt("2026-10-26 07:00"))
    ); // Fri -> Mon
}

#[test]
fn next_change_wraps_week() {
    let r = rules(vec![block(&[Day::Sun], 600, 660, 21.0)]);
    assert_eq!(
        r.next_change(dt("2026-10-25 12:00")),
        Some(dt("2026-11-01 10:00"))
    );
}

#[test]
fn next_change_skips_boundaries_without_a_change() {
    let r = rules(vec![
        block(&[Day::Mon], 360, 720, 20.0),
        block(&[Day::Mon], 720, 1080, 20.0),
    ]);
    assert_eq!(
        r.next_change(dt("2026-10-26 07:00")),
        Some(dt("2026-10-26 18:00"))
    );
}

#[test]
fn next_change_constant_is_none() {
    assert_eq!(rules(vec![]).next_change(dt("2026-10-21 08:00")), None);
}

#[test]
fn season_auto_hysteresis() {
    let h = HouseSettings::default();
    assert_eq!(season_mode(&h, Some(10.0), None), Some(Mode::Heat));
    assert_eq!(
        season_mode(&h, Some(26.0), Some(Mode::Heat)),
        Some(Mode::Cool)
    );
    assert_eq!(
        season_mode(&h, Some(20.0), Some(Mode::Cool)),
        Some(Mode::Cool)
    );
    assert_eq!(season_mode(&h, Some(20.0), None), Some(Mode::Heat));
    assert_eq!(season_mode(&h, None, None), Some(Mode::Heat));
}

#[test]
fn season_thresholds_themselves_hold_the_previous_mode() {
    let h = HouseSettings::default();
    assert_eq!(
        season_mode(&h, Some(h.heat_below), Some(Mode::Cool)),
        Some(Mode::Cool)
    );
    assert_eq!(
        season_mode(&h, Some(h.cool_above), Some(Mode::Heat)),
        Some(Mode::Heat)
    );
}

#[test]
fn season_fixed_and_off() {
    let with = |season| HouseSettings {
        season,
        ..Default::default()
    };
    assert_eq!(season_mode(&with(Season::Off), Some(10.0), None), None);
    assert_eq!(
        season_mode(&with(Season::Cool), Some(0.0), None),
        Some(Mode::Cool)
    );
    assert_eq!(
        season_mode(&with(Season::Heat), Some(40.0), None),
        Some(Mode::Heat)
    );
}

fn src(value: f64, age_s: u64) -> ReadingSource {
    ReadingSource { value, age_s }
}

#[test]
fn reading_prefers_fresh_sensor() {
    assert_eq!(
        room_reading(Some(src(20.0, 60)), &[src(15.0, 10)], 3600, 0),
        Reading::Fresh {
            value: 20.0,
            age_s: 60,
            from_sensor: true,
            sensor_stale: false
        }
    );
}

#[test]
fn a_quiet_sensor_still_reads_the_room_within_its_limit() {
    // A thermometer that only reports changes, silent for 4 h in a steady room.
    assert_eq!(
        room_reading(Some(src(17.4, 4 * 3600)), &[src(19.0, 30)], 3600, 6 * 3600),
        Reading::Fresh {
            value: 17.4,
            age_s: 4 * 3600,
            from_sensor: true,
            sensor_stale: false
        }
    );
    assert_eq!(
        room_reading(Some(src(17.4, 4 * 3600)), &[], 3600, 6 * 3600),
        Reading::Fresh {
            value: 17.4,
            age_s: 4 * 3600,
            from_sensor: true,
            sensor_stale: false
        }
    );
}

#[test]
fn a_quiet_limit_below_stale_after_does_not_shorten_it() {
    assert!(matches!(
        room_reading(Some(src(20.0, 3000)), &[], 3600, 600),
        Reading::Fresh {
            from_sensor: true,
            ..
        }
    ));
}

#[test]
fn reading_falls_back_to_devices_when_sensor_stale() {
    assert_eq!(
        room_reading(
            Some(src(22.0, 4000)),
            &[src(19.0, 30), src(21.0, 90), src(5.0, 9000)],
            3600,
            0
        ),
        Reading::Fresh {
            value: 20.0,
            age_s: 30,
            from_sensor: false,
            sensor_stale: true
        }
    );
    assert_eq!(
        room_reading(None, &[src(19.0, 30)], 3600, 0),
        Reading::Fresh {
            value: 19.0,
            age_s: 30,
            from_sensor: false,
            sensor_stale: false
        }
    );
}

#[test]
fn reading_at_exactly_stale_after_is_fresh() {
    assert!(matches!(
        room_reading(Some(src(20.0, 3600)), &[], 3600, 0),
        Reading::Fresh {
            from_sensor: true,
            ..
        }
    ));
    assert!(matches!(
        room_reading(None, &[src(19.0, 3600)], 3600, 0),
        Reading::Fresh { .. }
    ));
    assert_eq!(
        room_reading(None, &[src(19.0, 3601)], 3600, 0),
        Reading::Stale { last: Some(19.0) }
    );
}

#[test]
fn reading_stale_keeps_the_newest_value() {
    assert_eq!(
        room_reading(Some(src(22.0, 9000)), &[src(19.0, 5000)], 3600, 0),
        Reading::Stale { last: Some(19.0) }
    );
}

#[test]
fn reading_all_stale() {
    assert_eq!(
        room_reading(Some(src(22.0, 4000)), &[src(19.0, 5000)], 3600, 0),
        Reading::Stale { last: Some(22.0) }
    );
    assert_eq!(
        room_reading(None, &[src(19.0, 5000), src(21.0, 5000)], 3600, 0),
        Reading::Stale { last: Some(20.0) }
    );
}

#[test]
fn reading_missing() {
    assert_eq!(room_reading(None, &[], 3600, 0), Reading::Missing);
}

mod heated_by {
    use super::*;

    fn at(h: u32, m: u32) -> NaiveDateTime {
        dt("2026-11-23 00:00") + Duration::minutes(i64::from(h * 60 + m)) // Monday
    }

    fn rules(base: f64, blocks: Vec<Block>) -> ModeRules {
        ModeRules {
            base: Setpoint::Temp(base),
            blocks,
        }
    }

    fn plain(from_h: u16, to_h: u16, t: f64) -> Block {
        block(&[Day::Mon], from_h * 60, to_h * 60, t)
    }

    fn named_hm(from: (u16, u16), to: (u16, u16), t: f64, ids: &[&str]) -> Block {
        Block {
            heated_by: Some(ids.iter().map(|s| s.to_string()).collect()),
            ..block(&[Day::Mon], from.0 * 60 + from.1, to.0 * 60 + to.1, t)
        }
    }

    fn named(from_h: u16, to_h: u16, t: f64, ids: &[&str]) -> Block {
        named_hm((from_h, 0), (to_h, 0), t, ids)
    }

    #[test]
    fn next_change_counts_a_heated_by_edge() {
        let r = rules(
            15.0,
            vec![named(9, 12, 20.0, &["stove"]), plain(12, 18, 20.0)],
        );
        assert_eq!(r.next_change(at(10, 0)), Some(at(12, 0)));
    }

    #[test]
    fn heated_by_at_reads_the_block() {
        let r = rules(15.0, vec![named(9, 12, 20.0, &["stove"])]);
        assert_eq!(r.heated_by_at(at(10, 0)), Some(&["stove".to_string()][..]));
        assert_eq!(r.heated_by_at(at(13, 0)), None);
    }

    #[test]
    fn next_start_of_finds_a_block_within_the_window() {
        let r = rules(
            15.0,
            vec![
                named(9, 12, 20.0, &["stove"]),
                named_hm((12, 10), (14, 0), 20.0, &["stove"]),
            ],
        );
        assert_eq!(
            r.next_start_of("stove", false, at(12, 0), 15),
            Some(at(12, 10))
        );
        assert_eq!(r.next_start_of("stove", false, at(12, 0), 5), None);
        assert_eq!(r.next_start_of("ac", false, at(12, 0), 15), None);
        // default_names: the base after 14:00 counts for a heater in the room default
        assert_eq!(r.next_start_of("ac", true, at(13, 55), 15), Some(at(14, 0)));
    }

    #[test]
    fn next_start_of_ignores_a_touching_block_that_continues_the_same_heater() {
        let r = rules(
            15.0,
            vec![
                named(9, 12, 20.0, &["stove"]),
                named(12, 14, 20.0, &["stove"]),
            ],
        );
        assert_eq!(r.next_start_of("stove", false, at(11, 55), 15), None);
    }
}
