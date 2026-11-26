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
    let labels: Vec<String> = tick_labels(Lang::En, HistoryRange::Day, 1_792_497_600 + 3 * 3_600)
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
