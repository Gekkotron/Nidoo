//! Schedule lookup, season mode and room temperature reading. Pure functions over the documents.

use chrono::{Datelike, Duration, NaiveDateTime, Timelike};

use crate::doc::{Day, HouseSettings, Mode, ModeRules, Season, Setpoint};

/// How far ahead `ModeRules::next_change` looks.
const SEARCH_DAYS: i64 = 8;

impl ModeRules {
    /// The setpoint in force at `local`, and the index of the block that sets it (`None` = base).
    /// A block covers `[from, to)` on each of its days.
    pub fn at(&self, local: NaiveDateTime) -> (Setpoint, Option<usize>) {
        let day = Day::from_chrono(local.weekday());
        let minute = (local.hour() * 60 + local.minute()) as u16;
        self.blocks
            .iter()
            .enumerate()
            .find(|(_, b)| b.days.contains(&day) && b.from.0 <= minute && minute < b.to.0)
            .map_or((self.base, None), |(i, b)| (b.setpoint, Some(i)))
    }

    /// The next instant strictly after `local` (within 8 days) where the setpoint changes, or
    /// `None` if it stays constant.
    pub fn next_change(&self, local: NaiveDateTime) -> Option<NaiveDateTime> {
        let current = self.at(local).0;
        let midnight = local.date().and_hms_opt(0, 0, 0)?;
        let limit = local + Duration::days(SEARCH_DAYS);
        let mut candidates: Vec<NaiveDateTime> = (0..=SEARCH_DAYS)
            .flat_map(|d| {
                let day_start = midnight + Duration::days(d);
                self.blocks.iter().flat_map(move |b| {
                    [b.from.0, b.to.0].map(|m| day_start + Duration::minutes(i64::from(m)))
                })
            })
            .filter(|c| *c > local && *c <= limit)
            .collect();
        candidates.sort();
        candidates.dedup();
        candidates.into_iter().find(|c| self.at(*c).0 != current)
    }
}

/// The mode the house runs in, or `None` for off. In `Auto`, the outdoor mean picks heat or cool
/// and the previous mode holds inside the hysteresis band.
pub fn season_mode(
    house: &HouseSettings,
    outdoor_mean: Option<f64>,
    previous: Option<Mode>,
) -> Option<Mode> {
    match house.season {
        Season::Heat => Some(Mode::Heat),
        Season::Cool => Some(Mode::Cool),
        Season::Off => None,
        Season::Auto => match outdoor_mean {
            None => Some(Mode::Heat),
            Some(t) if t < house.heat_below => Some(Mode::Heat),
            Some(t) if t > house.cool_above => Some(Mode::Cool),
            Some(_) => previous.or(Some(Mode::Heat)),
        },
    }
}

/// A temperature reading and how old it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReadingSource {
    pub value: f64,
    pub age_s: u64,
}

/// The temperature of a room.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reading {
    /// A usable value. `from_sensor` is false when it is the mean of the devices' own readings;
    /// `sensor_stale` is true when a dedicated sensor exists but was too old to use.
    Fresh {
        value: f64,
        age_s: u64,
        from_sensor: bool,
        sensor_stale: bool,
    },
    /// Readings exist but are all too old; `last` is the newest of them (the sensor unless a
    /// device reported after it, else the devices' mean).
    Stale { last: Option<f64> },
    /// No reading at all.
    Missing,
}

fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, n) = values.fold((0.0, 0u32), |(s, n), v| (s + v, n + 1));
    (n > 0).then(|| sum / f64::from(n))
}

/// Pick the room temperature: a fresh dedicated sensor wins, else the mean of the fresh device
/// readings, else a stale value if anything was reported. The sensor stays fresh up to `quiet_s`
/// when that is longer than `stale_after_s`: a thermometer that only reports changes is silent
/// for hours in a steady room.
pub fn room_reading(
    sensor: Option<ReadingSource>,
    devices: &[ReadingSource],
    stale_after_s: u64,
    quiet_s: u64,
) -> Reading {
    let fresh = |r: &&ReadingSource| r.age_s <= stale_after_s;
    let sensor_limit_s = stale_after_s.max(quiet_s);
    if let Some(s) = sensor.filter(|s| s.age_s <= sensor_limit_s) {
        return Reading::Fresh {
            value: s.value,
            age_s: s.age_s,
            from_sensor: true,
            sensor_stale: false,
        };
    }
    if let Some(value) = mean(devices.iter().filter(fresh).map(|d| d.value)) {
        let age_s = devices
            .iter()
            .filter(fresh)
            .map(|d| d.age_s)
            .min()
            .unwrap_or(0);
        return Reading::Fresh {
            value,
            age_s,
            from_sensor: false,
            sensor_stale: sensor.is_some(),
        };
    }
    let newest_device = devices.iter().map(|d| d.age_s).min();
    let last = sensor
        .filter(|s| newest_device.is_none_or(|age| s.age_s <= age))
        .map(|s| s.value)
        .or_else(|| mean(devices.iter().map(|d| d.value)));
    if last.is_some() {
        Reading::Stale { last }
    } else {
        Reading::Missing
    }
}

#[cfg(test)]
mod tests {
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
}
