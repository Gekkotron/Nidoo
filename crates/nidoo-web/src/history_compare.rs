//! Pure maths for comparing a history window with an earlier one.
//!
//! Buckets of the two replies are paired by their rounded offset from each
//! reply's own window start, so a DST day (23 h or 25 h) or a month of
//! another length still lands on the matching index.

use std::collections::BTreeMap;

use nidoo_api::DailyPer;

/// Nominal bucket length in seconds for a daily-counter grouping.
pub fn nominal(per: DailyPer) -> i64 {
    match per {
        DailyPer::Hour => 3_600,
        DailyPer::Day => 86_400,
        DailyPer::Month => 2_629_746,
    }
}

/// Index of the bucket at `offset_s` from its window start: the offset in
/// nominal buckets, rounded half away from zero. A non-positive nominal gives 0.
pub fn bucket_index(offset_s: i64, nominal_s: i64) -> i64 {
    if nominal_s <= 0 {
        return 0;
    }
    (offset_s as f64 / nominal_s as f64).round() as i64
}

/// Totals of a counter reply against another one, over shared buckets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CounterSummary {
    pub shown: f64,
    pub compared: f64,
    pub diff: f64,
    pub pct: Option<f64>,
    pub left_out: usize,
}

/// Aggregates of a sensor reply against another one, over shared buckets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SensorSummary {
    pub mean: f64,
    pub min: f64,
    pub max: f64,
    pub cmp_mean: f64,
    pub cmp_min: f64,
    pub cmp_max: f64,
    pub diff: f64,
    pub left_out: usize,
}

/// Index each bucket by its offset; on a collision the first one wins.
fn by_index<T: Copy>(items: &[(i64, T)], start: i64, nominal_s: i64) -> BTreeMap<i64, T> {
    let mut map = BTreeMap::new();
    for &(t, v) in items {
        map.entry(bucket_index(t - start, nominal_s)).or_insert(v);
    }
    map
}

/// Pair two series by index: matched values (shown, compared) in index order,
/// and the number of indexes present on one side only.
fn pair<T: Copy>(
    shown: &[(i64, T)],
    shown_start: i64,
    cmp: &[(i64, T)],
    cmp_start: i64,
    nominal_s: i64,
) -> (Vec<(T, T)>, usize) {
    let a = by_index(shown, shown_start, nominal_s);
    let b = by_index(cmp, cmp_start, nominal_s);
    let matched: Vec<(T, T)> = a
        .iter()
        .filter_map(|(i, x)| b.get(i).map(|y| (*x, *y)))
        .collect();
    let left_out = a.len() + b.len() - 2 * matched.len();
    (matched, left_out)
}

pub fn counter_summary(
    shown: &[(i64, f64)],
    shown_start: i64,
    cmp: &[(i64, f64)],
    cmp_start: i64,
    per: DailyPer,
) -> Option<CounterSummary> {
    if cmp.is_empty() {
        return None;
    }
    let (matched, left_out) = pair(shown, shown_start, cmp, cmp_start, nominal(per));
    if matched.is_empty() {
        return None;
    }
    let shown_total: f64 = matched.iter().map(|(a, _)| a).sum();
    let compared: f64 = matched.iter().map(|(_, b)| b).sum();
    let diff = shown_total - compared;
    Some(CounterSummary {
        shown: shown_total,
        compared,
        diff,
        pct: (compared > 0.0).then(|| diff / compared * 100.0),
        left_out,
    })
}

type Point = (f64, f64, f64);

fn stats(points: impl Iterator<Item = Point>) -> Option<Point> {
    let (mut sum, mut n, mut min, mut max) = (0.0, 0usize, f64::INFINITY, f64::NEG_INFINITY);
    for (mean, lo, hi) in points {
        sum += mean;
        n += 1;
        min = min.min(lo);
        max = max.max(hi);
    }
    (n > 0).then(|| (sum / n as f64, min, max))
}

pub fn sensor_summary(
    shown: &[(i64, f64, f64, f64)],
    shown_start: i64,
    cmp: &[(i64, f64, f64, f64)],
    cmp_start: i64,
    window: i64,
) -> Option<SensorSummary> {
    if cmp.is_empty() {
        return None;
    }
    let tuple = |p: &[(i64, f64, f64, f64)]| -> Vec<(i64, Point)> {
        p.iter().map(|&(t, m, lo, hi)| (t, (m, lo, hi))).collect()
    };
    let (matched, left_out) = pair(&tuple(shown), shown_start, &tuple(cmp), cmp_start, window);
    let (mean, min, max) = stats(matched.iter().map(|(a, _)| *a))?;
    let (cmp_mean, cmp_min, cmp_max) = stats(matched.iter().map(|(_, b)| *b))?;
    Some(SensorSummary {
        mean,
        min,
        max,
        cmp_mean,
        cmp_min,
        cmp_max,
        diff: mean - cmp_mean,
        left_out,
    })
}

/// Sum of a counter reply when nothing is compared.
pub fn counter_total(days: &[(i64, f64)]) -> f64 {
    days.iter().map(|(_, k)| k).sum()
}

/// (mean of bucket means, min of mins, max of maxes) of a sensor reply.
pub fn sensor_stats(points: &[(i64, f64, f64, f64)]) -> Option<(f64, f64, f64)> {
    stats(points.iter().map(|&(_, m, lo, hi)| (m, lo, hi)))
}

#[cfg(test)]
mod tests;
