//! Tick labels, tooltip time strings, and state-axis labels for the history chart.
//!
//! Times are rendered in the house timezone (Europe/Paris). DST is computed by hand (last Sunday
//! of March at 01:00 UTC → +02:00; last Sunday of October at 01:00 UTC → +01:00), so no new
//! dependency is pulled into the wasm bundle.

use crate::i18n::format as fmt;
use crate::i18n::history::{HistoryText, history_text};
use crate::i18n::{Lang, states};
use nidoo_api::HistoryRange;
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset, Weekday};

/// Europe/Paris UTC offset in seconds at `unix` time.
pub fn paris_offset(unix: i64) -> i32 {
    let Ok(dt) = OffsetDateTime::from_unix_timestamp(unix) else {
        return 3600;
    };
    let year = dt.year();
    let dst_start = last_sunday_01_utc(year, Month::March);
    let dst_end = last_sunday_01_utc(year, Month::October);
    if (dst_start..dst_end).contains(&unix) {
        7200
    } else {
        3600
    }
}

fn last_sunday_01_utc(year: i32, month: Month) -> i64 {
    let last = month.length(year);
    let mut d = Date::from_calendar_date(year, month, last).unwrap();
    while d.weekday() != Weekday::Sunday {
        d = d.previous_day().unwrap();
    }
    PrimitiveDateTime::new(d, Time::from_hms(1, 0, 0).unwrap())
        .assume_utc()
        .unix_timestamp()
}

/// `unix` as an `OffsetDateTime` in the house zone.
fn local(unix: i64) -> OffsetDateTime {
    let off = paris_offset(unix);
    let offset = UtcOffset::from_whole_seconds(off).unwrap_or(UtcOffset::UTC);
    OffsetDateTime::from_unix_timestamp(unix)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .to_offset(offset)
}

/// X-axis tick labels for a range ending at `now_secs`: a list of `(position in 0..=1, label)`.
/// `now` is always the final label at position `1.0`. On the hour, six-hour, day and month axes,
/// whose ticks are instants, a tick closer to `now` than [`NOW_GAP`] is dropped so the two labels
/// do not run into each other; week and year labels name whole slots and are kept.
pub fn tick_labels(lang: Lang, range: HistoryRange, now_secs: i64) -> Vec<(f64, String)> {
    let mut out = match range {
        HistoryRange::Hour => clock_ticks(lang, now_secs, 3_600, 900),
        HistoryRange::SixHours => clock_ticks(lang, now_secs, 21_600, 3_600),
        HistoryRange::Day => day_ticks(lang, now_secs),
        HistoryRange::Week => return week_ticks(lang, now_secs),
        HistoryRange::Month => month_ticks(lang, now_secs),
        HistoryRange::Year => return year_ticks(lang, now_secs),
    };
    let last = out.len().saturating_sub(1);
    let mut i = 0;
    out.retain(|(pos, _)| {
        let keep = i == last || *pos <= 1.0 - NOW_GAP;
        i += 1;
        keep
    });
    out
}

/// The smallest gap, as a fraction of the axis, between a tick and the final `now` label.
const NOW_GAP: f64 = 0.12;

/// Ticks on the wall clock every `step` seconds across the last `span` seconds, then "now".
/// The house offset is a whole number of hours, so a multiple of `step` in UTC is also one on
/// the wall clock for any `step` that divides an hour.
fn clock_ticks(lang: Lang, now_secs: i64, span: i64, step: i64) -> Vec<(f64, String)> {
    let start = now_secs - span;
    let mut out = Vec::new();
    let mut t = (start + step - 1).div_euclid(step) * step;
    while t < now_secs {
        let dt = local(t);
        out.push((
            (t - start) as f64 / span as f64,
            format!("{:02}:{:02}", dt.hour(), dt.minute()),
        ));
        t += step;
    }
    out.push((1.0, history_text(lang, HistoryText::Now).into()));
    out
}

fn day_ticks(lang: Lang, now_secs: i64) -> Vec<(f64, String)> {
    let start = now_secs - 86_400;
    let mut out: Vec<(f64, String)> = Vec::new();
    // Each candidate wall-clock hour at 00/06/12/18 in the window. Compute the offset at each
    // candidate's own local date so DST transitions don't misplace a candidate by one hour
    // (reviewer finding #9).
    for h in [0u8, 6, 12, 18] {
        for days_back in 0..=1i64 {
            let probe = now_secs - days_back * 86_400;
            let dt = local(probe);
            let offset =
                UtcOffset::from_whole_seconds(paris_offset(probe)).unwrap_or(UtcOffset::UTC);
            let t = PrimitiveDateTime::new(dt.date(), Time::from_hms(h, 0, 0).unwrap())
                .assume_offset(offset)
                .unix_timestamp();
            if (start..now_secs).contains(&t) {
                let pos = (t - start) as f64 / 86_400.0;
                let label = format!("{h:02}:00");
                if !out.iter().any(|(_, l)| *l == label) {
                    out.push((pos, label));
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    out.push((1.0, history_text(lang, HistoryText::Now).into()));
    out
}

fn week_ticks(lang: Lang, now_secs: i64) -> Vec<(f64, String)> {
    // One label per day in the 7-day window. The last tick is "now".
    let mut out = Vec::new();
    for i in 0..7 {
        let t = now_secs - (6 - i) as i64 * 86_400;
        let pos = i as f64 / 6.0;
        let dt = local(t);
        let name = fmt::weekday_short(lang, chrono_weekday(dt.weekday()));
        if i == 6 {
            out.push((1.0, history_text(lang, HistoryText::Now).into()));
        } else {
            out.push((pos, name.to_owned()));
        }
    }
    out
}

fn month_ticks(lang: Lang, now_secs: i64) -> Vec<(f64, String)> {
    // Fixed day-of-month labels within the 30-day window.
    let start = now_secs - 30 * 86_400;
    let mut out = Vec::new();
    for day in [1u8, 8, 15, 22] {
        // Try the two most recent occurrences of this day-of-month.
        for back in 0..=1 {
            let probe = now_secs - back * 30 * 86_400 / 2;
            let dt = local(probe);
            let year = dt.year();
            let month = dt.month();
            if let Ok(d) = Date::from_calendar_date(year, month, day) {
                let t = PrimitiveDateTime::new(d, Time::MIDNIGHT)
                    .assume_offset(dt.offset())
                    .unix_timestamp();
                if (start..now_secs).contains(&t) {
                    let pos = (t - start) as f64 / (30.0 * 86_400.0);
                    out.push((pos, format!("{day:02}")));
                    break;
                }
            }
        }
    }
    out.push((1.0, history_text(lang, HistoryText::Now).into()));
    out
}

fn year_ticks(lang: Lang, now_secs: i64) -> Vec<(f64, String)> {
    // Twelve month labels ending at the month that contains `now`, walking back one month per
    // slot. Earlier code labelled every slot Jan..Dec regardless of the window, so October's
    // data was read as if it were January (reviewer finding #7).
    let end_month = local(now_secs).month() as i32;
    let mut out = Vec::new();
    for i in 0..12i32 {
        let m_num = ((end_month - 12 + i).rem_euclid(12) + 1) as u8;
        let month = Month::try_from(m_num).unwrap_or(Month::January);
        let pos = (i as f64 + 0.5) / 12.0;
        out.push((pos, fmt::month_short(lang, month as u32).to_owned()));
    }
    out.push((1.0, history_text(lang, HistoryText::Now).into()));
    out
}

/// The same weekday for `i18n::format`.
fn chrono_weekday(w: Weekday) -> chrono::Weekday {
    chrono::Weekday::try_from(w.number_days_from_monday()).unwrap_or(chrono::Weekday::Mon)
}

/// Local time for a tooltip. Relative up to 24h ("14:20"), absolute from 7d up
/// ("Tue 06 Oct 14:20" / "Mar. 06 oct. 14:20" through 30d, "Oct 2026" / "oct. 2026" for 1y). 24-hour clock in both.
pub fn tooltip_time(lang: Lang, range: HistoryRange, t_secs: i64, _now_secs: i64) -> String {
    let dt = local(t_secs);
    match range {
        HistoryRange::Hour | HistoryRange::SixHours | HistoryRange::Day => {
            format!("{:02}:{:02}", dt.hour(), dt.minute())
        }
        HistoryRange::Week | HistoryRange::Month => format!(
            "{} {:02} {} {:02}:{:02}",
            fmt::weekday_short(lang, chrono_weekday(dt.weekday())),
            dt.day(),
            fmt::month_short(lang, dt.month() as u32),
            dt.hour(),
            dt.minute(),
        ),
        HistoryRange::Year => format!(
            "{} {}",
            fmt::month_short(lang, dt.month() as u32),
            dt.year()
        ),
    }
}

/// The day a bar stands for, in the house zone: "Tue 06 Oct" / "Mar. 06 oct.".
pub fn tooltip_day(lang: Lang, t_secs: i64) -> String {
    let dt = local(t_secs);
    format!(
        "{} {:02} {}",
        fmt::weekday_short(lang, chrono_weekday(dt.weekday())),
        dt.day(),
        fmt::month_short(lang, dt.month() as u32),
    )
}

/// A discrete entity's value as the tooltip words it: "On" / "Off" for lights and switches, a
/// percentage for covers. `None` for a numeric sensor.
pub fn state_value_label(lang: Lang, kind: &str, v: f64) -> Option<String> {
    let on = v >= 0.5;
    match kind.to_ascii_lowercase().as_str() {
        "light" => Some(states::on_off(lang, true, on).to_owned()),
        "onoff" | "switch" | "binarysensor" => Some(states::on_off(lang, false, on).to_owned()),
        "cover" => Some(format!("{}%", fmt::decimal_grouped(lang, v, 0))),
        _ => None,
    }
}

/// For discrete-kind entities, the Y-axis label pair that stands in for numeric min/max. `None`
/// when the entity is a numeric sensor and should keep its numeric labels.
///
/// `kind` is the debug-rendered variant of `nidoo_core::model::Kind` (e.g. `"OnOff"`,
/// `"Light"`, `"BinarySensor"`, `"Cover"`). Case-insensitive.
pub fn state_axis_labels(lang: Lang, _unit: Option<&str>, kind: &str) -> Option<(String, String)> {
    // Lower-case keys: the kind is a Debug name, not text.
    match kind.to_ascii_lowercase().as_str() {
        // Lower case, as the axis always was: "off" / "on", "éteint" / "allumé".
        "light" => Some((
            states::on_off(lang, true, false).to_lowercase(),
            states::on_off(lang, true, true).to_lowercase(),
        )),
        "onoff" | "switch" | "binarysensor" => Some((
            states::on_off(lang, false, false).to_lowercase(),
            states::on_off(lang, false, true).to_lowercase(),
        )),
        "cover" => Some(("0".into(), "100".into())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;
    use nidoo_api::HistoryRange;

    #[test]
    fn paris_offset_is_two_hours_in_summer_one_in_winter() {
        // 2026-06-01 12:00 UTC → CEST (+02:00)
        let summer = OffsetDateTime::from_unix_timestamp(1_780_660_800).unwrap();
        assert_eq!(paris_offset(summer.unix_timestamp()), 7200);
        // 2026-12-01 12:00 UTC → CET (+01:00)
        let winter = OffsetDateTime::from_unix_timestamp(1_796_558_400).unwrap();
        assert_eq!(paris_offset(winter.unix_timestamp()), 3600);
    }

    #[test]
    fn a_tick_next_to_now_is_dropped() {
        // Every quarter hour of a day, so some "now" falls just after a 6-hourly tick.
        for step in 0..96 {
            let now = 1_792_497_600 + step * 900;
            let out = tick_labels(Lang::En, HistoryRange::Day, now);
            let (last, rest) = out.split_last().unwrap();
            assert_eq!(last.1, "now");
            assert!(rest.iter().all(|(p, _)| *p <= 1.0 - NOW_GAP), "{out:?}");
        }
    }

    #[test]
    fn tick_labels_for_24h_uses_6h_steps() {
        // 17:00 on 2026-10-20 Europe/Paris: every 6-hourly tick is clear of "now".
        let labels: Vec<String> =
            tick_labels(Lang::En, HistoryRange::Day, 1_792_497_600 + 3 * 3_600)
                .into_iter()
                .map(|(_, s)| s)
                .collect();
        assert!(labels.iter().any(|l| l == "00:00"));
        assert!(labels.iter().any(|l| l == "06:00"));
        assert!(labels.iter().any(|l| l == "12:00"));
        assert!(labels.iter().any(|l| l == "18:00"));
        assert_eq!(labels.last().unwrap(), "now");
    }

    #[test]
    fn tick_labels_for_1h_land_on_quarter_hours() {
        // 2026-10-20 14:00 UTC is 16:00 in Paris; 16:10 puts the ticks at 15:15 .. 16:00.
        let now = 1_792_504_800 + 10 * 60;
        let out = tick_labels(Lang::En, HistoryRange::Hour, now);
        let labels: Vec<&str> = out.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(labels, ["15:15", "15:30", "15:45", "16:00", "now"]);
        assert!(out.iter().all(|(p, _)| (0.0..=1.0).contains(p)), "{out:?}");
        assert_eq!(out.last().unwrap().0, 1.0);
    }

    #[test]
    fn tick_labels_for_6h_land_on_whole_hours() {
        // 16:07: the 16:00 tick is inside NOW_GAP of "now" and is dropped.
        let now = 1_792_504_800 + 7 * 60;
        let out = tick_labels(Lang::En, HistoryRange::SixHours, now);
        let labels: Vec<&str> = out.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(labels, ["11:00", "12:00", "13:00", "14:00", "15:00", "now"]);
        assert!(out.iter().all(|(p, _)| (0.0..=1.0).contains(p)), "{out:?}");
    }

    #[test]
    fn a_short_range_tick_next_to_now_is_dropped() {
        for range in [HistoryRange::Hour, HistoryRange::SixHours] {
            for step in 0..120 {
                let now = 1_792_497_600 + step * 60;
                let out = tick_labels(Lang::En, range, now);
                let (last, rest) = out.split_last().unwrap();
                assert_eq!(last.1, "now");
                assert!(rest.iter().all(|(p, _)| *p <= 1.0 - NOW_GAP), "{out:?}");
            }
        }
    }

    #[test]
    fn tick_labels_for_7d_has_7_entries_ending_with_now() {
        let out = tick_labels(Lang::En, HistoryRange::Week, 1_792_497_600);
        assert_eq!(out.len(), 7);
        assert_eq!(out.last().unwrap().1, "now");
    }

    #[test]
    fn tick_labels_for_1y_end_at_current_month_not_december() {
        // 2026-10-20 12:00 UTC → October in Europe/Paris. Reviewer finding #7.
        let out = tick_labels(Lang::En, HistoryRange::Year, 1_792_497_600);
        // Last entry is "now", penultimate is the current month.
        assert_eq!(out.last().unwrap().1, "now");
        assert_eq!(out[out.len() - 2].1, "Oct");
        // 11 months before current month = November of the prior year.
        assert_eq!(out[0].1, "Nov");
    }

    #[test]
    fn day_ticks_still_produce_four_hour_labels_on_a_dst_day() {
        // 2026-10-26 10:00 UTC — the day after the autumn DST transition (Oct 25).
        let now = 1_792_015_200 + 86_400;
        let out = tick_labels(Lang::En, HistoryRange::Day, now);
        let hour_labels: Vec<&String> = out
            .iter()
            .map(|(_, s)| s)
            .filter(|s| s.ends_with(":00") && !s.starts_with(|c: char| c.is_alphabetic()))
            .collect();
        // At most one of each of 00/06/12/18 must appear (deduped) and all four should fit a
        // 24-hour window.
        assert!(hour_labels.len() >= 3 && hour_labels.len() <= 4);
        for h in ["00:00", "06:00", "12:00", "18:00"] {
            assert_eq!(hour_labels.iter().filter(|l| **l == h).count(), 1);
        }
    }

    #[test]
    fn tooltip_time_is_relative_on_24h_and_absolute_from_7d() {
        let now = 1_792_497_600;
        let t = now - 3_600;
        let d = tooltip_time(Lang::En, HistoryRange::Day, t, now);
        assert!(d.len() == 5 && d.chars().nth(2) == Some(':'));
        for range in [HistoryRange::Hour, HistoryRange::SixHours] {
            assert_eq!(tooltip_time(Lang::En, range, t, now), "13:00");
        }
        let w = tooltip_time(Lang::En, HistoryRange::Week, t, now);
        assert!(w.len() > 10 && w.contains(':'));
        let y = tooltip_time(Lang::En, HistoryRange::Year, t, now);
        // Year tooltip looks like "Oct 2026"
        assert!(y.contains("20"));
        assert!(!y.contains(':'));
    }

    #[test]
    fn state_axis_labels_pair_each_kind() {
        assert_eq!(
            state_axis_labels(Lang::En, None, "OnOff"),
            Some(("off".into(), "on".into()))
        );
        assert_eq!(
            state_axis_labels(Lang::En, None, "Light"),
            Some(("off".into(), "on".into()))
        );
        assert_eq!(
            state_axis_labels(Lang::En, None, "BinarySensor"),
            Some(("off".into(), "on".into()))
        );
        assert_eq!(
            state_axis_labels(Lang::En, None, "Cover"),
            Some(("0".into(), "100".into()))
        );
        assert_eq!(state_axis_labels(Lang::En, Some("°C"), "Sensor"), None);
    }

    #[test]
    fn the_axis_speaks_french() {
        let now = 1_792_497_600;
        let day = tick_labels(Lang::Fr, HistoryRange::Day, now);
        assert_eq!(day.last().unwrap().1, "maintenant");
        for range in [HistoryRange::Hour, HistoryRange::SixHours] {
            let out = tick_labels(Lang::Fr, range, now);
            assert_eq!(out.last().unwrap().1, "maintenant");
        }
        let week = tick_labels(Lang::Fr, HistoryRange::Week, now);
        assert_eq!(week.last().unwrap().1, "maintenant");
        // 2026-10-20 is a Tuesday, six days before the last tick sits a Wednesday.
        assert_eq!(week[0].1, "Mer.");
        let month = tick_labels(Lang::Fr, HistoryRange::Month, now);
        assert_eq!(month.last().unwrap().1, "maintenant");
        let year = tick_labels(Lang::Fr, HistoryRange::Year, now);
        assert_eq!(year[year.len() - 2].1, "oct.");
        assert_eq!(year[0].1, "nov.");
    }

    #[test]
    fn the_tooltip_speaks_french() {
        let now = 1_792_497_600;
        let t = now - 3_600;
        assert_eq!(tooltip_time(Lang::Fr, HistoryRange::Day, t, now).len(), 5);
        assert_eq!(tooltip_time(Lang::Fr, HistoryRange::Hour, t, now), "13:00");
        assert_eq!(
            tooltip_time(Lang::Fr, HistoryRange::Week, t, now),
            "Mar. 20 oct. 13:00"
        );
        assert_eq!(
            tooltip_time(Lang::En, HistoryRange::Week, t, now),
            "Tue 20 Oct 13:00"
        );
        assert_eq!(
            tooltip_time(Lang::Fr, HistoryRange::Year, t, now),
            "oct. 2026"
        );
    }

    #[test]
    fn the_state_axis_speaks_french() {
        assert_eq!(
            state_axis_labels(Lang::Fr, None, "Light"),
            Some(("éteint".into(), "allumé".into()))
        );
        assert_eq!(
            state_axis_labels(Lang::Fr, None, "Switch"),
            Some(("désactivé".into(), "activé".into()))
        );
    }

    #[test]
    fn tooltip_day_names_the_day_without_a_time() {
        // 2026-10-06 12:00 UTC is a Tuesday.
        assert_eq!(tooltip_day(Lang::En, 1_791_288_000), "Tue 06 Oct");
        assert_eq!(tooltip_day(Lang::Fr, 1_791_288_000), "Mar. 06 oct.");
    }

    #[test]
    fn state_value_label_words_discrete_values_and_skips_numbers() {
        assert_eq!(
            state_value_label(Lang::En, "Light", 1.0).as_deref(),
            Some("On")
        );
        assert_eq!(
            state_value_label(Lang::En, "Light", 0.0).as_deref(),
            Some("Off")
        );
        assert!(state_value_label(Lang::Fr, "BinarySensor", 1.0).is_some());
        assert_eq!(
            state_value_label(Lang::En, "Cover", 40.0).as_deref(),
            Some("40%")
        );
        assert_eq!(state_value_label(Lang::En, "Sensor", 1.0), None);
    }
}
