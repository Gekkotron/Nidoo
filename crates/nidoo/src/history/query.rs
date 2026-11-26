//! The reader side of history: `series`, `daily`, `today` and `last_change`, plus the
//! shared `hours` helper that stitches stored hourly rows together with a live rollup for
//! the window past `done_until`.

use chrono::{DateTime, NaiveDate, TimeZone, Timelike, Utc};
use chrono_tz::Tz;

use nidoo_api::{
    DailyPer, GridDay, GridSlot, GridTotals, HistoryDaily, HistoryExtent, HistoryFields,
    HistoryRange, HistorySeries, HistoryToday, LastChange, LastChangeItem,
};
use nidoo_core::energy::{
    GridCounters, OffPeakDevice, OffPeakEntry, device_off_peak, is_off_peak, windows_for,
};
use nidoo_core::flow::fields as entity_fields;
use nidoo_core::history::buckets::{
    Bucket, EnergyKind, Sample, fill, kwh, merge, merge_from, rollup, round3,
};
use nidoo_core::history::grid_day::{HALF_HOUR, fill_slots, increases, slot_starts};
use nidoo_core::history::{HOUR, Micros};
use nidoo_core::model::{Availability, Entity, StateValue};

use super::fields::{TrackInput, build_tracks};
use super::window::{DateWindow, WindowUse, boundaries, check_limits, days, resolve, step_secs};
use super::{History, HistoryError};

fn hour_floor(t: Micros) -> Micros {
    t.div_euclid(HOUR) * HOUR
}

fn align_down(t: Micros, window: Micros) -> Micros {
    t.div_euclid(window) * window
}

fn unavailable(e: sqlx::Error) -> HistoryError {
    HistoryError::Unavailable(e.to_string())
}

/// Returns `days + 1` boundaries: local midnight of (today − days + 1), …, local midnight
/// of today, then `now` as the closing boundary. DST-safe for whole-hour offsets.
pub fn day_starts(zone: Tz, now: Micros, days: u32) -> Vec<Micros> {
    let now_utc: DateTime<Utc> =
        DateTime::<Utc>::from_timestamp_micros(now).unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
    let now_local = now_utc.with_timezone(&zone);
    let today = now_local.date_naive();

    let mut out = Vec::with_capacity(days as usize + 1);
    for i in (0..days as i64).rev() {
        out.push(local_midnight(zone, today - chrono::Duration::days(i)));
    }
    out.push(now);
    out
}

/// Local midnight of `date` in `zone`, the earliest instant when midnight is ambiguous.
pub(crate) fn local_midnight(zone: Tz, date: NaiveDate) -> Micros {
    let naive = date
        .and_hms_opt(0, 0, 0)
        .expect("00:00:00 is always a valid clock time");
    zone.from_local_datetime(&naive)
        .earliest()
        .or_else(|| zone.from_local_datetime(&naive).latest())
        .unwrap_or_else(|| Utc.from_utc_datetime(&naive).with_timezone(&zone))
        .timestamp_micros()
}

/// Local midnight of `date` and of the day after: the day's real-time span.
pub fn day_bounds(zone: Tz, date: NaiveDate) -> (Micros, Micros) {
    let next = date.succ_opt().unwrap_or(date);
    (local_midnight(zone, date), local_midnight(zone, next))
}

fn local(zone: Tz, at: Micros) -> DateTime<Tz> {
    DateTime::<Utc>::from_timestamp_micros(at)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
        .with_timezone(&zone)
}

/// The house-local date of `at`.
pub fn local_date(zone: Tz, at: Micros) -> NaiveDate {
    local(zone, at).date_naive()
}

fn local_minute(zone: Tz, at: Micros) -> u16 {
    let t = local(zone, at);
    (t.hour() * 60 + t.minute()) as u16
}

const MINUTE: Micros = 60_000_000;

/// One counter's carry (last reading before the day), its readings within the day and its kWh
/// per unit.
#[derive(Clone, Debug)]
pub struct CounterDay {
    pub carry: Option<f64>,
    pub samples: Vec<Sample>,
    pub factor: f64,
}

/// The bought and sold counters' readings for the day.
#[derive(Clone, Debug, Default)]
pub struct DayReadings {
    pub bought: Vec<CounterDay>,
    pub sold: Vec<CounterDay>,
}

/// Everything [`build_grid_day`] needs, read beforehand.
pub struct DayInput<'a> {
    pub zone: Tz,
    pub date: NaiveDate,
    pub first_date: Option<NaiveDate>,
    pub history: &'a [OffPeakEntry],
    pub readings: &'a DayReadings,
    pub has_export: bool,
    pub now: Micros,
    /// What the off-peak device said through the day, when one is set; a moment no span
    /// answers for is split by the windows.
    pub device: Option<&'a [DeviceSpan]>,
}

/// `[from, to)` during which the off-peak device said `off_peak`; `None` when its reading was
/// not usable then.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceSpan {
    pub from: Micros,
    pub to: Micros,
    pub off_peak: Option<bool>,
}

/// A carried reading (the last before the day) and the readings within it, oldest first.
pub type Readings<T> = (Option<T>, Vec<(Micros, T)>);

/// The device's answer over `[day_start, until)` from its value and availability rows: one span
/// per change of either, equal neighbours merged. Empty when `until <= day_start`.
pub fn device_spans(
    device: &OffPeakDevice,
    day_start: Micros,
    until: Micros,
    values: Readings<StateValue>,
    availability: Readings<Availability>,
) -> Vec<DeviceSpan> {
    if until <= day_start {
        return Vec::new();
    }
    let (value_carry, value_rows) = values;
    let (avail_carry, avail_rows) = availability;
    let mut points: Vec<Micros> = std::iter::once(day_start)
        .chain(value_rows.iter().map(|(t, _)| *t))
        .chain(avail_rows.iter().map(|(t, _)| *t))
        .filter(|t| (day_start..until).contains(t))
        .collect();
    points.sort_unstable();
    points.dedup();
    let in_force = |at: Micros| {
        let value = value_rows
            .iter()
            .rev()
            .find(|(t, _)| *t <= at)
            .map(|(_, v)| v)
            .or(value_carry.as_ref());
        let avail = avail_rows
            .iter()
            .rev()
            .find(|(t, _)| *t <= at)
            .map(|(_, a)| *a)
            .or(avail_carry)
            .unwrap_or_default();
        value.and_then(|v| device_off_peak(device, v, avail))
    };
    let mut spans: Vec<DeviceSpan> = Vec::new();
    for (i, from) in points.iter().copied().enumerate() {
        let to = points.get(i + 1).copied().unwrap_or(until);
        let off_peak = in_force(from);
        match spans.last_mut() {
            Some(last) if last.off_peak == off_peak => last.to = to,
            _ => spans.push(DeviceSpan { from, to, off_peak }),
        }
    }
    spans
}

/// The grid day from its readings: half-hour slots to now, split by the windows in force on
/// that day, with the off-peak bands and totals.
pub fn build_grid_day(input: &DayInput) -> GridDay {
    let zone = input.zone;
    let (day_start, day_end) = day_bounds(zone, input.date);
    let windows = windows_for(input.history, input.date);
    let spans = input.device.unwrap_or_default();
    let off_peak_at = |at: Micros| {
        spans
            .iter()
            .find(|s| (s.from..s.to).contains(&at))
            .and_then(|s| s.off_peak)
            .unwrap_or_else(|| is_off_peak(windows, local_minute(zone, at)))
    };
    let tariffs = !windows.is_empty() || input.device.is_some();

    let rises = |counters: &[CounterDay]| -> Vec<(Micros, f64)> {
        counters
            .iter()
            .flat_map(|c| {
                increases(c.carry, &c.samples)
                    .into_iter()
                    .map(move |(at, rise)| (at, rise * c.factor))
            })
            .collect()
    };
    let slots = slot_starts(day_start, day_end, input.now);
    let filled = fill_slots(
        &slots,
        &rises(&input.readings.bought),
        &rises(&input.readings.sold),
        off_peak_at,
    );

    let mut bands: Vec<(i64, i64)> = Vec::new();
    if tariffs {
        let mut open: Option<Micros> = None;
        let mut t = day_start;
        while t < day_end {
            match (off_peak_at(t), open) {
                (true, None) => open = Some(t),
                (false, Some(from)) => {
                    bands.push((from / 1_000_000, t / 1_000_000));
                    open = None;
                }
                _ => {}
            }
            t += MINUTE;
        }
        if let Some(from) = open {
            bands.push((from / 1_000_000, day_end / 1_000_000));
        }
    }

    let sum = |f: fn(&GridSlot) -> f64, slots: &[GridSlot]| round3(slots.iter().map(f).sum());
    let slots: Vec<GridSlot> = filled
        .into_iter()
        .map(|s| GridSlot {
            start: s.start / 1_000_000,
            local: local(zone, s.start).format("%H:%M").to_string(),
            off_peak: s.off_peak,
            peak: s.peak,
            export: s.export,
        })
        .collect();
    let totals = GridTotals {
        off_peak: sum(|s| s.off_peak, &slots),
        peak: sum(|s| s.peak, &slots),
        export: sum(|s| s.export, &slots),
    };
    let ymd = |d: NaiveDate| d.format("%Y-%m-%d").to_string();
    GridDay {
        date: ymd(input.date),
        today: ymd(local_date(zone, input.now)),
        first_date: input.first_date.map(ymd),
        day_slots: ((day_end - day_start + HALF_HOUR - 1) / HALF_HOUR) as usize,
        slots,
        bands,
        totals,
        has_export: input.has_export,
        tariffs,
    }
}

/// `GET /history/grid_day`: what crossed the grid meter on `date` (today when `None`).
pub async fn grid_day(
    h: &History,
    counters: &GridCounters,
    history: &[OffPeakEntry],
    device: Option<&OffPeakDevice>,
    date: Option<NaiveDate>,
    now: Micros,
) -> Result<GridDay, HistoryError> {
    let db = h.db()?;
    if counters.bought.is_empty() {
        return Err(HistoryError::NoGridMeter);
    }
    let zone = h.zone();
    let today = local_date(zone, now);
    let date = date.unwrap_or(today);
    if date > today {
        return Err(HistoryError::BadRequest("date is in the future".into()));
    }
    let (day_start, day_end) = day_bounds(zone, date);
    let until = day_end.min(now);
    let read = |ids: &[String]| {
        let ids = ids.to_vec();
        async move {
            let mut out = Vec::with_capacity(ids.len());
            for id in &ids {
                let carry = db.last_before(id, day_start).await?;
                let samples = if day_start < until {
                    db.samples(day_start, until, Some(id))
                        .await?
                        .into_iter()
                        .map(|(_, s)| s)
                        .collect()
                } else {
                    Vec::new()
                };
                out.push(CounterDay {
                    carry,
                    samples,
                    factor: counters.factor(id),
                });
            }
            Ok::<_, sqlx::Error>(out)
        }
    };
    let readings = DayReadings {
        bought: read(&counters.bought).await.map_err(unavailable)?,
        sold: read(&counters.sold).await.map_err(unavailable)?,
    };
    let spans = match device {
        Some(device) => {
            let values = db
                .states_between(&device.entity, day_start, until)
                .await
                .map_err(unavailable)?;
            let availability = db
                .availability_between(&device.entity, day_start, until)
                .await
                .map_err(unavailable)?;
            Some(device_spans(device, day_start, until, values, availability))
        }
        None => None,
    };
    let first_date = db
        .first_time_of(&counters.bought)
        .await
        .map_err(unavailable)?
        .map(|t| local_date(zone, t));
    Ok(build_grid_day(&DayInput {
        zone,
        date,
        first_date,
        history,
        readings: &readings,
        has_export: !counters.sold.is_empty(),
        now,
        device: spans.as_deref(),
    }))
}

/// Carry at `start` for the 24h series: the last raw sample in `[hour_floor(start), start)`
/// for `entity`, falling back to `hourly_carry(hour_floor(start))`.
async fn series_carry_at(
    db: &super::db::Db,
    entity: &str,
    start: Micros,
) -> Result<Option<f64>, sqlx::Error> {
    let hour_start = hour_floor(start);
    if hour_start < start {
        let pre = db.samples(hour_start, start, Some(entity)).await?;
        if let Some((_, last)) = pre.last() {
            return Ok(Some(last.num));
        }
    }
    let carries = db.hourly_carry(hour_start, &[entity.to_string()]).await?;
    Ok(carries.get(entity).copied())
}

/// Hourly buckets from `from` to `now`: stored rows for `[from, done_until)`, a live
/// rollup from raw samples for `[done_until, now)`, then quiet-filled over the whole span
/// with the carry at `from`.
pub(crate) async fn hours(
    h: &History,
    entity: &str,
    from: Micros,
    now: Micros,
) -> Result<Vec<Bucket>, HistoryError> {
    let db = h.db()?;
    let done_until = db.done_until().await.map_err(unavailable)?;
    let mut live_start = done_until.unwrap_or(from).max(from);
    let mut stored_end = live_start.min(now);
    if stored_end < live_start && stored_end % HOUR != 0 {
        // The span ends mid-hour inside the stored part: that hour's stored row covers all
        // of it, so roll it from raw samples cut at `now` instead.
        stored_end = hour_floor(stored_end).max(from);
        live_start = stored_end;
    }

    let stored = if from < stored_end {
        db.hourly(entity, from, stored_end)
            .await
            .map_err(unavailable)?
    } else {
        Vec::new()
    };

    let live = if live_start < now {
        let samples: Vec<Sample> = db
            .samples(live_start, now, Some(entity))
            .await
            .map_err(unavailable)?
            .into_iter()
            .map(|(_, s)| s)
            .collect();
        let carry = match stored.last() {
            Some(last) => Some(last.last),
            None => db
                .hourly_carry(live_start, &[entity.to_string()])
                .await
                .map_err(unavailable)?
                .get(entity)
                .copied(),
        };
        rollup(carry, &samples, live_start, now, HOUR)
    } else {
        Vec::new()
    };

    let mut all = stored;
    all.extend(live);

    let carry_at_from = db
        .hourly_carry(from, &[entity.to_string()])
        .await
        .map_err(unavailable)?
        .get(entity)
        .copied();
    Ok(fill(carry_at_from, &all, from, now, HOUR))
}

/// Buckets of `window` from raw samples over `[start, end)`, quiet ones filled with the carry
/// at `start`.
async fn raw_series(
    db: &super::db::Db,
    entity: &str,
    start: Micros,
    end: Micros,
    window: Micros,
) -> Result<Vec<Bucket>, HistoryError> {
    let carry = series_carry_at(db, entity, start)
        .await
        .map_err(unavailable)?;
    let samples: Vec<Sample> = db
        .samples(start, end, Some(entity))
        .await
        .map_err(unavailable)?
        .into_iter()
        .map(|(_, s)| s)
        .collect();
    let raw = rollup(carry, &samples, start, end, window);
    Ok(fill(carry, &raw, start, end, window))
}

/// `(t, mean, min, max)` per bucket, `t` in unix seconds.
fn points_of(buckets: &[Bucket]) -> Vec<(i64, f64, f64, f64)> {
    buckets
        .iter()
        .map(|b| {
            (
                b.start / 1_000_000,
                round3(b.mean()),
                round3(b.min),
                round3(b.max),
            )
        })
        .collect()
}

pub async fn series(
    h: &History,
    entity: &str,
    range: HistoryRange,
    now: Micros,
) -> Result<HistorySeries, HistoryError> {
    let db = h.db()?;
    if !db.is_recorded(entity).await.map_err(unavailable)? {
        return Err(HistoryError::NotRecorded);
    }

    let window_secs = range.window_secs();
    let window = window_secs * 1_000_000;
    let span = range.span_secs() * 1_000_000;
    let start = align_down(now - span, window);

    let buckets = match range {
        HistoryRange::Hour | HistoryRange::SixHours | HistoryRange::Day => {
            raw_series(db, entity, start, now, window).await?
        }
        _ => {
            let bs = hours(h, entity, start, now).await?;
            merge(&bs, window)
        }
    };

    let points = points_of(&buckets);
    Ok(HistorySeries {
        window: window_secs,
        points,
        span: None,
    })
}

/// Every value of `entity` over `range` as one track each: the headline when the main chart
/// does not draw it, then its fields. `404 not_recorded` when it has no rows.
pub async fn fields(
    h: &History,
    entity: &Entity,
    range: HistoryRange,
    now: Micros,
) -> Result<HistoryFields, HistoryError> {
    let window = range.window_secs() * 1_000_000;
    let from = align_down(now - range.span_secs() * 1_000_000, window);
    fields_between(h, entity, from, now, window).await
}

/// [`fields`] over a house-local date window. Rejects windows beyond the fields limit.
pub async fn fields_window(
    h: &History,
    entity: &Entity,
    w: &DateWindow,
    now: Micros,
) -> Result<HistoryFields, HistoryError> {
    check_limits(w, WindowUse::Fields)?;
    let r = resolve(h.zone(), w, now)?;
    let window = step_secs(days(w)) * 1_000_000;
    fields_between(h, entity, r.start, r.end, window).await
}

async fn fields_between(
    h: &History,
    entity: &Entity,
    from: Micros,
    to: Micros,
    window: Micros,
) -> Result<HistoryFields, HistoryError> {
    let db = h.db()?;
    let id = entity.unique_id.as_str();
    if !db.is_recorded(id).await.map_err(unavailable)? {
        return Err(HistoryError::NotRecorded);
    }
    let (carry, rows) = db.states_between(id, from, to).await.map_err(unavailable)?;
    let list = entity_fields(
        entity.kind,
        &entity.component,
        &entity.capabilities,
        entity.unit.as_deref(),
    );
    let tracks = build_tracks(&TrackInput {
        component: &entity.component,
        fields: &list,
        headline_num: &|v| crate::hub::history_num_of(entity, v),
        carry: carry.as_ref(),
        rows: &rows,
        from,
        to,
        window,
    });
    Ok(HistoryFields {
        from: from / 1_000_000,
        to: to / 1_000_000,
        tracks,
    })
}

pub async fn daily(
    h: &History,
    entity: &str,
    days: u32,
    kind: EnergyKind,
    now: Micros,
) -> Result<HistoryDaily, HistoryError> {
    if !matches!(days, 7 | 30 | 365) {
        return Err(HistoryError::BadRequest("days must be 7, 30 or 365".into()));
    }
    let db = h.db()?;
    if !db.is_recorded(entity).await.map_err(unavailable)? {
        return Err(HistoryError::NotRecorded);
    }
    let boundaries = day_starts(h.zone(), now, days);
    let from = boundaries[0];
    let hourly = hours(h, entity, from, now).await?;

    let mut days_vec = Vec::new();
    for i in 0..days as usize {
        let day_from = boundaries[i];
        let day_to = boundaries[i + 1];
        if let Some(k) = kwh(&hourly, day_from, day_to, kind) {
            days_vec.push((day_from / 1_000_000, round3(k)));
        }
    }
    Ok(HistoryDaily {
        days: days_vec,
        span: None,
    })
}

/// `series` over a house-local date window: a one-day window rolls raw samples as the `Day`
/// preset does; longer ones merge the hourly rows into steps aligned at the window's start.
pub async fn series_window(
    h: &History,
    entity: &str,
    w: &DateWindow,
    now: Micros,
) -> Result<HistorySeries, HistoryError> {
    let r = resolve(h.zone(), w, now)?;
    let db = h.db()?;
    if !db.is_recorded(entity).await.map_err(unavailable)? {
        return Err(HistoryError::NotRecorded);
    }
    let window_secs = step_secs(days(w));
    let window = window_secs * 1_000_000;
    let buckets = if days(w) <= 1 {
        raw_series(db, entity, r.start, r.end, window).await?
    } else {
        let bs = hours(h, entity, r.start, r.end).await?;
        merge_from(&bs, r.start, window)
    };
    Ok(HistorySeries {
        window: window_secs,
        points: points_of(&buckets),
        span: Some((r.start / 1_000_000, r.end / 1_000_000)),
    })
}

/// kWh per hour, day or month over a house-local date window. Buckets with no data are left
/// out.
pub async fn daily_window(
    h: &History,
    entity: &str,
    w: &DateWindow,
    per: DailyPer,
    kind: EnergyKind,
    now: Micros,
) -> Result<HistoryDaily, HistoryError> {
    check_limits(w, WindowUse::Daily(per))?;
    let db = h.db()?;
    let r = resolve(h.zone(), w, now)?;
    if !db.is_recorded(entity).await.map_err(unavailable)? {
        return Err(HistoryError::NotRecorded);
    }
    let hourly = hours(h, entity, r.start, r.end).await?;
    let bounds = boundaries(h.zone(), w, &r, per);
    let days_vec = bounds
        .windows(2)
        .filter_map(|p| kwh(&hourly, p[0], p[1], kind).map(|k| (p[0] / 1_000_000, round3(k))))
        .collect();
    Ok(HistoryDaily {
        days: days_vec,
        span: Some((r.start / 1_000_000, r.end / 1_000_000)),
    })
}

/// The dates history covers for `entity`: its first recorded day and the house's today.
pub async fn extent(h: &History, entity: &str, now: Micros) -> Result<HistoryExtent, HistoryError> {
    let db = h.db()?;
    if !db.is_recorded(entity).await.map_err(unavailable)? {
        return Err(HistoryError::NotRecorded);
    }
    let zone = h.zone();
    let first = db
        .first_time_of(&[entity.to_string()])
        .await
        .map_err(unavailable)?;
    let ymd = |d: NaiveDate| d.format("%Y-%m-%d").to_string();
    Ok(HistoryExtent {
        first_date: first.map(|t| ymd(local_date(zone, t))),
        today: ymd(local_date(zone, now)),
    })
}

pub async fn today(
    h: &History,
    entity: &str,
    kind: EnergyKind,
    now: Micros,
) -> Result<HistoryToday, HistoryError> {
    let db = h.db()?;
    if !db.is_recorded(entity).await.map_err(unavailable)? {
        return Err(HistoryError::NotRecorded);
    }
    let boundaries = day_starts(h.zone(), now, 1);
    let midnight = boundaries[0];
    let hourly = hours(h, entity, midnight, now).await?;
    let kwh_val = kwh(&hourly, midnight, now, kind).map(round3);
    Ok(HistoryToday {
        kwh: kwh_val,
        since: midnight / 1_000_000,
    })
}

pub async fn last_change(h: &History, ids: &[String]) -> Result<LastChange, HistoryError> {
    if ids.is_empty() || ids.len() > 50 {
        return Err(HistoryError::BadRequest(
            "entity count must be 1..=50".into(),
        ));
    }
    let db = h.db()?;
    let changes = db.last_changes(ids).await.map_err(unavailable)?;
    let items = ids
        .iter()
        .map(|id| {
            let changed_at = changes.get(id).map(|m| m / 1_000_000);
            LastChangeItem {
                changed_at,
                recorded: changed_at.is_some(),
            }
        })
        .collect();
    Ok(LastChange { items })
}

#[cfg(test)]
mod tests;
