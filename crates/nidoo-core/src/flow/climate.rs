//! Pure clock helpers for the climate steps.

use chrono::{DateTime, Days, NaiveDateTime, TimeDelta, TimeZone};

use super::clock::parse_clock;

/// The next local `HH:MM` strictly after `now`: today when it is still ahead, else tomorrow.
/// A time that falls in a DST gap resolves to the first valid instant after the gap; an
/// ambiguous one (clocks going back) to the earlier instant. `None` for a malformed `hhmm`.
pub fn until_after<Tz: TimeZone>(now: &DateTime<Tz>, hhmm: &str) -> Option<DateTime<Tz>> {
    let minute = parse_clock(hhmm).ok()?;
    let tz = now.timezone();
    let today = now.date_naive();
    let at = |day: chrono::NaiveDate| -> Option<DateTime<Tz>> {
        let mut naive: NaiveDateTime =
            day.and_hms_opt(u32::from(minute / 60), u32::from(minute % 60), 0)?;
        // A gap is at most a few hours; step a minute at a time until the clock exists.
        for _ in 0..=24 * 60 {
            if let Some(t) = tz.from_local_datetime(&naive).earliest() {
                return Some(t);
            }
            naive += TimeDelta::minutes(1);
        }
        None
    };
    let first = at(today)?;
    if first > *now {
        return Some(first);
    }
    at(today.checked_add_days(Days::new(1))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::Europe::Paris;

    #[test]
    fn until_past_rolls_to_tomorrow() {
        let now = Paris.with_ymd_and_hms(2026, 11, 25, 6, 30, 0).unwrap();
        let at = |d, h, m| Paris.with_ymd_and_hms(2026, 11, d, h, m, 0).unwrap();
        assert_eq!(until_after(&now, "06:00"), Some(at(26, 6, 0)));
        assert_eq!(until_after(&now, "07:00"), Some(at(25, 7, 0)));
        assert_eq!(until_after(&now, "06:30"), Some(at(26, 6, 30)));
        assert_eq!(until_after(&now, "6:30"), None);
    }

    #[test]
    fn until_in_a_dst_gap_lands_after_it() {
        // 02:30 does not exist on 2026-03-29 in Paris; the clock jumps to 03:00.
        let now = Paris.with_ymd_and_hms(2026, 3, 29, 0, 10, 0).unwrap();
        let got = until_after(&now, "02:30").unwrap();
        assert_eq!(got.format("%H:%M").to_string(), "03:00");
        assert_eq!(got.date_naive(), now.date_naive());
    }
}
