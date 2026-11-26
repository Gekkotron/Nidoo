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
mod tests;
