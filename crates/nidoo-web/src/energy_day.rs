//! Pure presentation of the Energy tab's grid day chart: day labels and navigation, the bar
//! scale, hour ticks, tooltip and totals wording.

use chrono::{Datelike, Duration, NaiveDate, NaiveTime};
pub use nidoo_api::{GridDay, GridSlot, GridTotals};

use crate::i18n::Lang;
use crate::i18n::energy::{self as words, EnergyText, lower_first, part_names};
use crate::i18n::format::{date_short, decimal, weekday_short, with_unit};

fn date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// "Today", "Yesterday", else "Thu Oct 22" ("jeu. 22 oct."); an unparsable date is shown as is.
pub fn day_label(lang: Lang, day: &str, today: &str) -> String {
    match (date(day), date(today)) {
        (Some(d), Some(t)) if d == t => words::energy_text(lang, EnergyText::Today).into(),
        (Some(d), Some(t)) if d.succ_opt() == Some(t) => {
            words::energy_text(lang, EnergyText::Yesterday).into()
        }
        (Some(d), _) => format!(
            "{} {}",
            weekday_short(lang, d.weekday()),
            date_short(lang, d)
        ),
        _ => day.to_string(),
    }
}

/// The day `by` days from `day`, or `None` past `today` or before `first`.
pub fn shift_day(day: &str, by: i64, first: Option<&str>, today: &str) -> Option<String> {
    let next = date(day)? + Duration::days(by);
    if next > date(today)? || first.and_then(date).is_some_and(|f| next < f) {
        return None;
    }
    Some(next.format("%Y-%m-%d").to_string())
}

/// Whether `reply` answers the request for `requested` (`None` is the hub's today), so a late
/// reply for another day is dropped.
pub fn accepts(requested: Option<&str>, reply: &GridDay) -> bool {
    match requested {
        Some(day) => reply.date == day,
        None => reply.date == reply.today,
    }
}

/// The day the card shows: the one asked for, else the hub's today once known.
pub fn shown_day(requested: Option<&str>, today: Option<&str>) -> Option<String> {
    requested.or(today).map(str::to_string)
}

/// The kWh at the top (bought) and bottom (sold) of the chart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale {
    pub up: f64,
    pub down: f64,
}

fn half_up(x: f64) -> f64 {
    ((x / 0.5).ceil() * 0.5).max(0.5)
}

/// The day's largest bought and sold slot, each rounded up to the next 0.5 kWh (0.5 at least);
/// a day that sold nothing gets no room below zero.
pub fn scale(day: &GridDay) -> Scale {
    let max = |f: fn(&GridSlot) -> f64| day.slots.iter().map(f).fold(0.0, f64::max);
    let sold = max(|s| s.export);
    Scale {
        up: half_up(max(|s| s.off_peak + s.peak)),
        down: if sold > 0.0 { half_up(sold) } else { 0.0 },
    }
}

/// Slot indexes of the hour ticks: every 3 h, or every 6 h on a narrow screen.
pub fn tick_hours(slots: usize, narrow: bool) -> Vec<usize> {
    let step = if narrow { 12 } else { 6 };
    (0..slots).step_by(step).collect()
}

/// "14:30" → "14:30–15:00".
pub fn slot_range(local: &str) -> String {
    match NaiveTime::parse_from_str(local, "%H:%M") {
        Ok(t) => format!("{local}–{}", (t + Duration::minutes(30)).format("%H:%M")),
        Err(_) => local.to_string(),
    }
}

/// The bar's details: its half hour and every non-zero part.
pub fn tooltip(lang: Lang, slot: &GridSlot, tariffs: bool) -> String {
    let (off_peak, peak, bought, sold) = part_names(lang);
    let mut parts = vec![slot_range(&slot.local)];
    let mut push = |name: &str, v: f64| {
        if v > 0.0 {
            parts.push(format!("{name} {}", decimal(lang, v, 2)));
        }
    };
    if tariffs {
        push(off_peak, slot.off_peak);
        push(peak, slot.peak);
    } else {
        push(bought, slot.off_peak + slot.peak);
    }
    push(sold, slot.export);
    if parts.len() == 1 {
        parts.push("0".into());
    }
    with_unit(parts.join(" · "), Some("kWh"))
}

/// The bought line, its off-peak / peak split with tariffs, and the sold line with export.
pub fn totals_text(lang: Lang, day: &GridDay) -> (String, Option<String>, Option<String>) {
    let t = &day.totals;
    let (off_peak, peak, _, _) = part_names(lang);
    let kwh = |v: f64| with_unit(decimal(lang, v, 1), Some("kWh"));
    let bought = format!(
        "{} {}",
        words::energy_text(lang, EnergyText::Bought),
        kwh(t.off_peak + t.peak)
    );
    let split = day.tariffs.then(|| {
        format!(
            "{off_peak} {} · {peak} {}",
            decimal(lang, t.off_peak, 1),
            decimal(lang, t.peak, 1)
        )
    });
    let sold = day.has_export.then(|| {
        format!(
            "{} {}",
            words::energy_text(lang, EnergyText::Sold),
            kwh(t.export)
        )
    });
    (bought, split, sold)
}

/// Where unix second `t` falls on the day axis, 0 to 1.
pub fn x_of(t: i64, day: &GridDay) -> f64 {
    match day.slots.first() {
        Some(first) if day.day_slots > 0 => {
            ((t - first.start) as f64 / (day.day_slots as f64 * 1_800.0)).clamp(0.0, 1.0)
        }
        _ => 0.0,
    }
}

/// Whether any slot moved a counter.
pub fn has_data(day: &GridDay) -> bool {
    day.slots
        .iter()
        .any(|s| s.off_peak > 0.0 || s.peak > 0.0 || s.export > 0.0)
}

/// One sentence for screen readers.
pub fn aria_summary(lang: Lang, day: &GridDay) -> String {
    let (bought, split, sold) = totals_text(lang, day);
    let mut parts = vec![lower_first(&bought)];
    parts.extend(split);
    parts.extend(sold.map(|s| lower_first(&s)));
    words::grid_summary(
        lang,
        &day_label(lang, &day.date, &day.today),
        &parts.join(", "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(local: &str, off_peak: f64, peak: f64, export: f64) -> GridSlot {
        GridSlot {
            start: 0,
            local: local.into(),
            off_peak,
            peak,
            export,
        }
    }

    fn day(slots: Vec<GridSlot>, tariffs: bool, has_export: bool) -> GridDay {
        let sum = |f: fn(&GridSlot) -> f64| slots.iter().map(f).sum();
        GridDay {
            date: "2026-10-23".into(),
            today: "2026-10-24".into(),
            first_date: Some("2026-01-15".into()),
            day_slots: 48,
            totals: GridTotals {
                off_peak: sum(|s| s.off_peak),
                peak: sum(|s| s.peak),
                export: sum(|s| s.export),
            },
            slots,
            bands: vec![],
            has_export,
            tariffs,
        }
    }

    #[test]
    fn stale_reply_is_rejected() {
        let d = day(vec![], true, false); // date 2026-10-23, today 2026-10-24
        assert!(accepts(Some("2026-10-23"), &d));
        assert!(!accepts(Some("2026-10-22"), &d));
        // Today is asked as `None`: only a reply for the hub's today answers it.
        assert!(!accepts(None, &d));
        let today = GridDay {
            date: "2026-10-24".into(),
            ..d
        };
        assert!(accepts(None, &today));
    }

    #[test]
    fn shown_day_follows_the_request() {
        assert_eq!(
            shown_day(Some("2026-10-20"), Some("2026-10-24")).as_deref(),
            Some("2026-10-20")
        );
        assert_eq!(
            shown_day(None, Some("2026-10-24")).as_deref(),
            Some("2026-10-24")
        );
        assert_eq!(shown_day(None, None), None);
        assert_eq!(
            shown_day(Some("2026-10-20"), None).as_deref(),
            Some("2026-10-20")
        );
    }

    #[test]
    fn day_labels() {
        assert_eq!(day_label(Lang::En, "2026-10-24", "2026-10-24"), "Today");
        assert_eq!(day_label(Lang::En, "2026-10-23", "2026-10-24"), "Yesterday");
        assert_eq!(
            day_label(Lang::En, "2026-10-22", "2026-10-24"),
            "Thu Oct 22"
        );
        assert_eq!(
            day_label(Lang::Fr, "2026-10-22", "2026-10-24"),
            "Jeu. 22 oct."
        );
        assert_eq!(
            day_label(Lang::Fr, "2026-10-24", "2026-10-24"),
            "Aujourd'hui"
        );
        assert_eq!(day_label(Lang::Fr, "2026-10-23", "2026-10-24"), "Hier");
        assert_eq!(day_label(Lang::En, "garbage", "2026-10-24"), "garbage");
    }

    #[test]
    fn shift_day_bounds() {
        let first = Some("2026-01-15");
        assert_eq!(shift_day("2026-10-24", 1, first, "2026-10-24"), None);
        assert_eq!(
            shift_day("2026-10-24", -1, first, "2026-10-24").as_deref(),
            Some("2026-10-23")
        );
        assert_eq!(shift_day("2026-01-15", -1, first, "2026-10-24"), None);
        assert_eq!(
            shift_day("2026-10-23", 1, None, "2026-10-24").as_deref(),
            Some("2026-10-24")
        );
    }

    #[test]
    fn scale_rounds_up() {
        let d = day(vec![slot("00:00", 1.0, 0.73, 0.0)], true, false);
        assert_eq!(scale(&d), Scale { up: 2.0, down: 0.0 });
        let d = day(vec![slot("00:00", 0.1, 0.0, 1.2)], true, true);
        assert_eq!(scale(&d), Scale { up: 0.5, down: 1.5 });
    }

    #[test]
    fn ticks_every_3h_or_6h() {
        assert_eq!(tick_hours(48, false), vec![0, 6, 12, 18, 24, 30, 36, 42]);
        assert_eq!(tick_hours(48, true), vec![0, 12, 24, 36]);
        assert_eq!(tick_hours(50, true), vec![0, 12, 24, 36, 48]);
    }

    #[test]
    fn slot_range_wraps_midnight() {
        assert_eq!(slot_range("14:30"), "14:30–15:00");
        assert_eq!(slot_range("23:30"), "23:30–00:00");
        assert_eq!(slot_range("01:00"), "01:00–01:30");
    }

    #[test]
    fn tooltip_omits_zero_parts() {
        let s = slot("14:30", 0.21, 0.1, 0.05);
        assert_eq!(
            tooltip(Lang::En, &s, true),
            "14:30–15:00 · off-peak 0.21 · peak 0.10 · sold 0.05\u{a0}kWh"
        );
        assert_eq!(
            tooltip(Lang::Fr, &s, true),
            "14:30–15:00 · heures creuses 0,21 · heures pleines 0,10 · vendu 0,05\u{a0}kWh"
        );
        let s = slot("14:30", 0.0, 0.1, 0.0);
        assert_eq!(
            tooltip(Lang::En, &s, true),
            "14:30–15:00 · peak 0.10\u{a0}kWh"
        );
        let s = slot("14:30", 0.2, 0.1, 0.0);
        assert_eq!(
            tooltip(Lang::En, &s, false),
            "14:30–15:00 · bought 0.30\u{a0}kWh"
        );
        let s = slot("14:30", 0.0, 0.0, 0.0);
        assert_eq!(tooltip(Lang::En, &s, true), "14:30–15:00 · 0\u{a0}kWh");
    }

    #[test]
    fn totals_without_tariffs_or_export() {
        let d = day(vec![slot("00:00", 7.21, 9.9, 3.12)], true, true);
        assert_eq!(
            totals_text(Lang::En, &d),
            (
                "Bought 17.1\u{a0}kWh".to_string(),
                Some("off-peak 7.2 · peak 9.9".to_string()),
                Some("Sold 3.1\u{a0}kWh".to_string())
            )
        );
        assert_eq!(
            totals_text(Lang::Fr, &d),
            (
                "Acheté 17,1\u{a0}kWh".to_string(),
                Some("heures creuses 7,2 · heures pleines 9,9".to_string()),
                Some("Vendu 3,1\u{a0}kWh".to_string())
            )
        );
        let d = day(vec![slot("00:00", 0.0, 2.0, 0.0)], false, false);
        assert_eq!(
            totals_text(Lang::En, &d),
            ("Bought 2.0\u{a0}kWh".to_string(), None, None)
        );
    }

    #[test]
    fn x_of_spans_the_full_day() {
        let mut d = day(vec![slot("00:00", 0.0, 0.0, 0.0)], true, false);
        d.slots[0].start = 1_000;
        assert_eq!(x_of(1_000, &d), 0.0);
        assert_eq!(x_of(1_000 + 24 * 1_800, &d), 0.5);
        assert_eq!(x_of(1_000 + 96 * 1_800, &d), 1.0);
        assert_eq!(x_of(0, &d), 0.0);
        assert_eq!(x_of(5, &day(vec![], true, false)), 0.0);
    }

    #[test]
    fn has_data_needs_a_nonzero_slot() {
        assert!(!has_data(&day(
            vec![slot("00:00", 0.0, 0.0, 0.0)],
            true,
            false
        )));
        assert!(has_data(&day(
            vec![slot("00:00", 0.0, 0.0, 0.1)],
            true,
            true
        )));
    }

    #[test]
    fn aria_summary_reads_the_totals() {
        let d = day(vec![slot("00:00", 7.21, 9.9, 3.12)], true, true);
        assert_eq!(
            aria_summary(Lang::En, &d),
            "Grid on Yesterday: bought 17.1\u{a0}kWh, off-peak 7.2 · peak 9.9, sold 3.1\u{a0}kWh."
        );
        assert_eq!(
            aria_summary(Lang::Fr, &d),
            "Réseau, hier\u{a0}: acheté 17,1\u{a0}kWh, heures creuses 7,2 · heures pleines 9,9, vendu 3,1\u{a0}kWh."
        );
    }
}
