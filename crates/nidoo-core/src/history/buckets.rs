//! Carried-forward buckets and kWh maths.
//!
//! Inside one bucket the segments are `[bucket start or first known, s1)` with the carry,
//! `[s_i, s_i+1)` with `v_i`, and `[s_n, min(bucket end, end))` with `v_n`. `covered` is the
//! total length of those segments; `increase` sums `max(0, b - a)` over consecutive pairs
//! (carry → s1 and s_i → s_i+1); the carry into the next bucket is that bucket's `last`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{HOUR, Micros};

/// A single raw sample for an entity: the hub's microsecond and the entity's `num` at that
/// point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub at: Micros,
    pub num: f64,
}

/// One bucket of carried-forward statistics. `integral` is in value-hours and `mean()`
/// divides it by the covered span in hours.
#[derive(Clone, Debug, PartialEq)]
pub struct Bucket {
    pub start: Micros,
    pub covered: Micros,
    pub integral: f64,
    pub min: f64,
    pub max: f64,
    pub last: f64,
    pub increase: f64,
}

impl Bucket {
    pub fn mean(&self) -> f64 {
        self.integral / (self.covered as f64 / HOUR as f64)
    }
}

/// What kWh means for this entity: a counter (meter reading, cumulative) or a power
/// reading (W, step-integrated into Wh and divided by 1000).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum EnergyKind {
    Counter,
    Power,
}

impl EnergyKind {
    pub fn parse(s: &str) -> Option<EnergyKind> {
        match s {
            "counter" => Some(EnergyKind::Counter),
            "power" => Some(EnergyKind::Power),
            _ => None,
        }
    }
}

/// Builds the buckets of `size` from `start` (aligned) to `end` (may be mid-bucket) that
/// hold at least one sample. `carry` is the value before `start`; `samples` are sorted and
/// within `[start, end)`.
pub fn rollup(
    carry: Option<f64>,
    samples: &[Sample],
    start: Micros,
    end: Micros,
    size: Micros,
) -> Vec<Bucket> {
    let mut out = Vec::new();
    if size <= 0 || start >= end || samples.is_empty() {
        return out;
    }
    let mut current_carry = carry;
    let mut k: i64 = 0;
    let mut cursor = 0usize;
    loop {
        let b_start = start + k * size;
        if b_start >= end {
            break;
        }
        let b_end = (b_start + size).min(end);
        let mut j = cursor;
        while j < samples.len() && samples[j].at < b_end {
            j += 1;
        }
        if j > cursor {
            let bucket = build_bucket(current_carry, &samples[cursor..j], b_start, b_end);
            current_carry = Some(bucket.last);
            out.push(bucket);
            cursor = j;
        }
        k += 1;
    }
    out
}

fn build_bucket(carry: Option<f64>, samples: &[Sample], b_start: Micros, b_end: Micros) -> Bucket {
    let mut covered: Micros = 0;
    let mut integral: f64 = 0.0;
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut increase: f64 = 0.0;

    let s1_v = samples[0].num;

    if let Some(cv) = carry {
        let span = samples[0].at - b_start;
        if span > 0 {
            covered += span;
            integral += cv * (span as f64 / HOUR as f64);
        }
        if cv < min {
            min = cv;
        }
        if cv > max {
            max = cv;
        }
        increase += (s1_v - cv).max(0.0);
    }

    for i in 0..samples.len() {
        let v = samples[i].num;
        let seg_start = samples[i].at;
        let seg_end = if i + 1 < samples.len() {
            samples[i + 1].at
        } else {
            b_end
        };
        let span = seg_end - seg_start;
        if span > 0 {
            covered += span;
            integral += v * (span as f64 / HOUR as f64);
        }
        if v < min {
            min = v;
        }
        if v > max {
            max = v;
        }
        if i + 1 < samples.len() {
            increase += (samples[i + 1].num - v).max(0.0);
        }
    }

    Bucket {
        start: b_start,
        covered,
        integral,
        min,
        max,
        last: samples.last().unwrap().num,
        increase,
    }
}

/// Returns every bucket from `start` to `end` of width `size`, the given ones as-is. A
/// quiet bucket gets the carried value for `min`, `max` and `last`, zero `increase`, and
/// `integral = carry × span` where `span` is cut at `end`. Buckets before the first known
/// value are omitted.
pub fn fill(
    carry: Option<f64>,
    buckets: &[Bucket],
    start: Micros,
    end: Micros,
    size: Micros,
) -> Vec<Bucket> {
    let mut out = Vec::new();
    if size <= 0 || start >= end {
        return out;
    }
    let mut current_carry = carry;
    let mut idx = 0usize;
    let mut k: i64 = 0;
    loop {
        let b_start = start + k * size;
        if b_start >= end {
            break;
        }
        let b_end = (b_start + size).min(end);
        if idx < buckets.len() && buckets[idx].start == b_start {
            let b = buckets[idx].clone();
            current_carry = Some(b.last);
            out.push(b);
            idx += 1;
        } else if let Some(cv) = current_carry {
            let span = b_end - b_start;
            out.push(Bucket {
                start: b_start,
                covered: span,
                integral: cv * (span as f64 / HOUR as f64),
                min: cv,
                max: cv,
                last: cv,
                increase: 0.0,
            });
        }
        k += 1;
    }
    out
}

/// Mean of the readings at or after `start`: `recent` holds every reading the hub received
/// (sorted), `recorded` the history rows (sorted, one per change) and fills only the part of
/// the window before the first recent reading. `None` when no reading falls in the window.
pub fn readings_mean(recorded: &[Sample], recent: &[Sample], start: Micros) -> Option<f64> {
    let first_recent = recent.first().map_or(Micros::MAX, |r| r.at);
    let readings: Vec<f64> = recorded
        .iter()
        .filter(|r| r.at < first_recent)
        .chain(recent)
        .filter(|r| r.at >= start)
        .map(|r| r.num)
        .collect();
    (!readings.is_empty()).then(|| readings.iter().sum::<f64>() / readings.len() as f64)
}

/// Merges consecutive buckets into UTC-aligned windows of `window`. Sums `covered`,
/// `integral` and `increase`, takes min of `min` and max of `max`, and keeps the last
/// bucket's `last` within each window.
pub fn merge(buckets: &[Bucket], window: Micros) -> Vec<Bucket> {
    if window <= 0 || buckets.is_empty() {
        return Vec::new();
    }
    let mut groups: BTreeMap<Micros, Vec<&Bucket>> = BTreeMap::new();
    for b in buckets {
        let ws = (b.start / window) * window;
        groups.entry(ws).or_default().push(b);
    }
    let mut out = Vec::new();
    for (ws, group) in groups {
        let mut covered: Micros = 0;
        let mut integral = 0.0;
        let mut increase = 0.0;
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        let mut last = 0.0;
        for b in &group {
            covered += b.covered;
            integral += b.integral;
            increase += b.increase;
            if b.min < min {
                min = b.min;
            }
            if b.max > max {
                max = b.max;
            }
            last = b.last;
        }
        out.push(Bucket {
            start: ws,
            covered,
            integral,
            min,
            max,
            last,
            increase,
        });
    }
    out
}

/// Σ `increase` for counters, Σ `integral` / 1000 for power. Uses buckets with `start` in
/// `[from, to)`. `None` when no bucket is in range.
pub fn kwh(buckets: &[Bucket], from: Micros, to: Micros, kind: EnergyKind) -> Option<f64> {
    let selected: Vec<&Bucket> = buckets
        .iter()
        .filter(|b| b.start >= from && b.start < to)
        .collect();
    if selected.is_empty() {
        return None;
    }
    let result = match kind {
        EnergyKind::Counter => selected.iter().map(|b| b.increase).sum::<f64>(),
        EnergyKind::Power => selected.iter().map(|b| b.integral).sum::<f64>() / 1000.0,
    };
    Some(result)
}

/// Rounds to 3 decimals, half away from zero.
pub fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: Micros = 60_000_000;

    fn s(at: Micros, num: f64) -> Sample {
        Sample { at, num }
    }

    #[test]
    fn hour_with_carry_and_two_samples() {
        let b = rollup(
            Some(10.0),
            &[s(15 * M, 20.0), s(45 * M, 0.0)],
            0,
            HOUR,
            HOUR,
        );
        assert_eq!(b.len(), 1);
        assert_eq!((b[0].integral, b[0].covered), (12.5, HOUR));
        assert_eq!(
            (b[0].min, b[0].max, b[0].last, b[0].increase),
            (0.0, 20.0, 0.0, 10.0)
        );
        assert_eq!(b[0].mean(), 12.5);
    }

    #[test]
    fn first_sample_without_carry_covers_from_the_sample() {
        let b = rollup(None, &[s(30 * M, 4.0)], 0, HOUR, HOUR);
        assert_eq!(
            (b[0].covered, b[0].integral, b[0].mean()),
            (30 * M, 2.0, 4.0)
        );
    }

    #[test]
    fn counter_reset_adds_nothing() {
        let b = rollup(
            Some(100.0),
            &[s(10 * M, 105.0), s(20 * M, 0.0), s(30 * M, 3.0)],
            0,
            HOUR,
            HOUR,
        );
        assert_eq!(b[0].increase, 8.0);
    }

    #[test]
    fn partial_last_bucket_stops_at_end() {
        let b = rollup(Some(2.0), &[s(10 * M, 2.0)], 0, 30 * M, HOUR);
        assert_eq!((b[0].covered, b[0].integral), (30 * M, 1.0));
    }

    #[test]
    fn fill_quiet_buckets_with_carry_and_cut_at_end() {
        let f = fill(Some(5.0), &[], 0, HOUR + 30 * M, HOUR);
        assert_eq!(f.len(), 2);
        assert_eq!((f[0].integral, f[0].increase, f[0].last), (5.0, 0.0, 5.0));
        assert_eq!((f[1].covered, f[1].integral), (30 * M, 2.5));
    }

    #[test]
    fn fill_omits_buckets_before_first_value() {
        let b = rollup(None, &[s(HOUR + 10 * M, 1.0)], 0, 2 * HOUR, HOUR);
        assert_eq!(fill(None, &b, 0, 2 * HOUR, HOUR).len(), 1);
    }

    #[test]
    fn day_mean_is_time_weighted() {
        let raw = rollup(Some(1.0), &[s(23 * HOUR, 0.0)], 0, 24 * HOUR, HOUR);
        let day = merge(&fill(Some(1.0), &raw, 0, 24 * HOUR, HOUR), 24 * HOUR);
        assert_eq!(round3(day[0].mean()), 0.958);
        assert_eq!((day[0].min, day[0].max), (0.0, 1.0));
    }

    #[test]
    fn kwh_counter_and_power() {
        let hours = fill(Some(2000.0), &[], 0, 2 * HOUR, HOUR);
        assert_eq!(kwh(&hours, 0, 2 * HOUR, EnergyKind::Power), Some(4.0));
        assert_eq!(kwh(&hours, 0, 2 * HOUR, EnergyKind::Counter), Some(0.0));
        assert_eq!(kwh(&[], 0, HOUR, EnergyKind::Power), None);
    }

    #[test]
    fn round3_rounds_half_away() {
        assert_eq!(round3(1.23456), 1.235);
    }

    #[test]
    fn readings_mean_counts_every_recent_reading_including_the_newest() {
        let recent = [s(M, 80.0), s(5 * M, 70.0), s(9 * M, 70.0), s(13 * M, 60.0)];
        assert_eq!(readings_mean(&[], &recent, 0), Some(70.0));
        // Readings before the window are left out.
        assert_eq!(readings_mean(&[], &recent, 2 * M), Some(200.0 / 3.0));
    }

    #[test]
    fn readings_mean_fills_the_start_of_the_window_from_history() {
        let recorded = [s(M, 90.0), s(5 * M, 70.0), s(9 * M, 60.0)];
        let recent = [s(9 * M, 60.0), s(13 * M, 50.0)];
        assert_eq!(readings_mean(&recorded, &recent, 0), Some(67.5));
        assert_eq!(readings_mean(&recorded, &[], 0), Some(220.0 / 3.0));
    }

    #[test]
    fn readings_mean_without_a_reading_in_the_window_is_none() {
        assert_eq!(readings_mean(&[s(M, 90.0)], &[], 2 * M), None);
        assert_eq!(readings_mean(&[], &[], 0), None);
    }
}
