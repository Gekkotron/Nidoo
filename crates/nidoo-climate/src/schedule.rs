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

    /// The block's `heated_by` at `local`, `None` for the base or a block that names nobody.
    pub fn heated_by_at(&self, local: NaiveDateTime) -> Option<&[String]> {
        let (_, index) = self.at(local);
        self.blocks.get(index?)?.heated_by.as_deref()
    }

    /// The next instant strictly after `local` (within 8 days) where the setpoint or the
    /// `heated_by` changes, or `None` if both stay constant.
    pub fn next_change(&self, local: NaiveDateTime) -> Option<NaiveDateTime> {
        let key = |t| (self.at(t).0, self.heated_by_at(t));
        let current = key(local);
        self.edges(local, local + Duration::days(SEARCH_DAYS), SEARCH_DAYS)?
            .into_iter()
            .find(|c| key(*c) != current)
    }

    /// The sorted, distinct block edges in `(local, limit]`, taken from `local`'s day and the
    /// `days` days after it.
    fn edges(
        &self,
        local: NaiveDateTime,
        limit: NaiveDateTime,
        days: i64,
    ) -> Option<Vec<NaiveDateTime>> {
        let midnight = local.date().and_hms_opt(0, 0, 0)?;
        let mut edges: Vec<NaiveDateTime> = (0..=days)
            .flat_map(|d| {
                let day_start = midnight + Duration::days(d);
                self.blocks.iter().flat_map(move |b| {
                    [b.from.0, b.to.0].map(|m| day_start + Duration::minutes(i64::from(m)))
                })
            })
            .filter(|c| *c > local && *c <= limit)
            .collect();
        edges.sort();
        edges.dedup();
        Some(edges)
    }

    /// Whether `id` is named at `local`: the block in force lists it, or `default_names` is set
    /// and the instant is base time or in a block whose `heated_by` is `None`.
    fn names(&self, id: &str, default_names: bool, local: NaiveDateTime) -> bool {
        match self.heated_by_at(local) {
            Some(ids) => ids.iter().any(|i| i == id),
            None => default_names,
        }
    }

    /// The next instant strictly after `local`, within `within_min` minutes, where a block
    /// naming `id` starts: `id` is named there and was not named one minute earlier.
    /// `default_names` says whether base time and unnamed blocks count as naming `id`.
    pub fn next_start_of(
        &self,
        id: &str,
        default_names: bool,
        local: NaiveDateTime,
        within_min: u32,
    ) -> Option<NaiveDateTime> {
        let limit = local + Duration::minutes(i64::from(within_min));
        let days = i64::from(within_min) / (24 * 60) + 1;
        self.edges(local, limit, days)?.into_iter().find(|c| {
            self.names(id, default_names, *c)
                && !self.names(id, default_names, *c - Duration::minutes(1))
        })
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
mod tests;
