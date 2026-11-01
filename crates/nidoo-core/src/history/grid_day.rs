//! A day of grid exchange in half-hour slots: counter increases attributed to the slot and the
//! tariff of the reading that shows them, so the slots add up exactly to the counter's rise.

use super::Micros;
use super::buckets::{Sample, round3};

/// Half an hour in microseconds.
pub const HALF_HOUR: Micros = 30 * 60 * 1_000_000;

/// kWh crossing the grid meter in the half hour from `start`.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotKwh {
    pub start: Micros,
    pub off_peak: f64,
    pub peak: f64,
    pub export: f64,
}

/// The slot starts `day_start + k·HALF_HOUR` below `min(day_end, until)`.
pub fn slot_starts(day_start: Micros, day_end: Micros, until: Micros) -> Vec<Micros> {
    let end = day_end.min(until);
    (0..)
        .map(|k| day_start + k * HALF_HOUR)
        .take_while(|&s| s < end)
        .collect()
}

/// One `(reading time, rise)` per reading that rose above the value before it (the carry for
/// the first); a drop (reset, replaced meter) counts nothing.
pub fn increases(carry: Option<f64>, samples: &[Sample]) -> Vec<(Micros, f64)> {
    let mut prev = carry;
    let mut out = Vec::new();
    for sample in samples {
        if let Some(p) = prev {
            let rise = sample.num - p;
            if rise > 0.0 {
                out.push((sample.at, rise));
            }
        }
        prev = Some(sample.num);
    }
    out
}

/// Adds each rise to the slot holding its reading time: bought ones to `off_peak` or `peak` by
/// `off_peak_at`, sold ones to `export`. Rises outside every slot are dropped.
pub fn fill_slots(
    slots: &[Micros],
    bought: &[(Micros, f64)],
    sold: &[(Micros, f64)],
    off_peak_at: impl Fn(Micros) -> bool,
) -> Vec<SlotKwh> {
    let mut out: Vec<SlotKwh> = slots
        .iter()
        .map(|&start| SlotKwh {
            start,
            off_peak: 0.0,
            peak: 0.0,
            export: 0.0,
        })
        .collect();
    let Some(&first) = slots.first() else {
        return out;
    };
    let len = out.len();
    let index = |at: Micros| {
        (at >= first)
            .then(|| ((at - first) / HALF_HOUR) as usize)
            .filter(|&i| i < len)
    };
    for &(at, kwh) in bought {
        if let Some(i) = index(at) {
            if off_peak_at(at) {
                out[i].off_peak += kwh;
            } else {
                out[i].peak += kwh;
            }
        }
    }
    for &(at, kwh) in sold {
        if let Some(i) = index(at) {
            out[i].export += kwh;
        }
    }
    for slot in &mut out {
        slot.off_peak = round3(slot.off_peak);
        slot.peak = round3(slot.peak);
        slot.export = round3(slot.export);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::buckets::Sample;

    fn s(at: Micros, num: f64) -> Sample {
        Sample { at, num }
    }

    fn min(m: i64) -> Micros {
        m * 60_000_000
    }

    #[test]
    fn slots_stop_at_until() {
        let s = slot_starts(0, 48 * HALF_HOUR, 3 * HALF_HOUR + 1);
        assert_eq!(s, vec![0, HALF_HOUR, 2 * HALF_HOUR, 3 * HALF_HOUR]);
        assert_eq!(slot_starts(0, 48 * HALF_HOUR, i64::MAX).len(), 48);
        assert_eq!(slot_starts(0, 50 * HALF_HOUR, i64::MAX).len(), 50);
        assert!(slot_starts(10, 48 * HALF_HOUR, 10).is_empty());
    }

    #[test]
    fn reset_never_counts_negative() {
        let inc = increases(Some(100.0), &[s(1, 100.5), s(2, 3.0), s(3, 3.2)]);
        assert_eq!(inc.len(), 2);
        assert!((inc[0].1 - 0.5).abs() < 1e-9 && (inc[1].1 - 0.2).abs() < 1e-9);
        assert_eq!((inc[0].0, inc[1].0), (1, 3));
    }

    #[test]
    fn first_reading_without_carry_counts_nothing() {
        let inc = increases(None, &[s(1, 50.0), s(2, 50.25)]);
        assert_eq!(inc, vec![(2, 0.25)]);
    }

    #[test]
    fn quiet_meter_catch_up_lands_on_next_reading() {
        // Last reading before the day at 10.0, silent until 05:40 at 12.0.
        let inc = increases(Some(10.0), &[s(min(340), 12.0)]);
        let slots = slot_starts(0, 48 * HALF_HOUR, i64::MAX);
        let out = fill_slots(&slots, &inc, &[], |_| false);
        assert_eq!(out[11].peak, 2.0);
        assert!(out[..11].iter().all(|x| x.peak == 0.0));
        assert_eq!(out[11].start, 11 * HALF_HOUR);
    }

    #[test]
    fn split_by_tariff_at_reading_time() {
        let slots = slot_starts(0, 48 * HALF_HOUR, i64::MAX);
        let bought = vec![(min(92), 0.1), (min(95), 0.2)];
        let out = fill_slots(&slots, &bought, &[(min(93), 0.05)], |at| at >= min(94));
        assert_eq!(
            (out[3].peak, out[3].off_peak, out[3].export),
            (0.1, 0.2, 0.05)
        );
    }

    #[test]
    fn sums_several_counters() {
        let slots = slot_starts(0, 48 * HALF_HOUR, i64::MAX);
        let out = fill_slots(&slots, &[(min(2), 0.3), (min(1), 0.1)], &[], |_| true);
        assert_eq!(out[0].off_peak, 0.4);
    }

    #[test]
    fn increases_outside_the_slots_are_dropped() {
        let slots = slot_starts(0, 48 * HALF_HOUR, 2 * HALF_HOUR);
        let out = fill_slots(&slots, &[(-1, 1.0), (min(61), 1.0)], &[], |_| false);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|x| x.peak == 0.0));
    }

    #[test]
    fn values_are_rounded() {
        let slots = slot_starts(0, HALF_HOUR, i64::MAX);
        let out = fill_slots(&slots, &[(1, 0.1), (2, 0.2)], &[], |_| false);
        assert_eq!(out[0].peak, 0.3);
    }
}
