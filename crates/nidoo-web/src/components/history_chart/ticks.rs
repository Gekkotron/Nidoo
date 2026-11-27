//! x tick labels of a date window, taken from the window's own dates: hours for a day,
//! weekdays for a week, day numbers for a month, month names for a year. Positions are
//! fractions of the whole window; a label names a whole slot, so it sits at the slot's middle.

use chrono::{Datelike, Duration, NaiveDate};

use super::window::{present_range, window_days};
use crate::api::history::WindowReq;
use crate::history_period::Unit;
use crate::i18n::Lang;
use crate::i18n::format as fmt;
use crate::i18n::history::{HistoryText, history_text};
use nidoo_api::HistoryRange;

/// The smallest gap, as a fraction of the axis, between a tick and the "now" label.
const NOW_GAP: f64 = 0.12;

/// The most month labels a long window carries.
const MAX_MONTH_LABELS: usize = 13;

/// Labels of `shown` as `(position in 0..1, text)`. `now_pos` is where the running period has
/// got to, as a fraction of the window: it adds a "now" label there, and a window that is not
/// running (`None`) has none.
pub fn window_ticks(
    lang: Lang,
    unit: Unit,
    shown: &WindowReq,
    now_pos: Option<f64>,
    by_slot: bool,
) -> Vec<(f64, String)> {
    let n = window_days(shown).max(1);
    let at = |i: i64| (i as f64 + 0.5) / n as f64;
    let mut out: Vec<(f64, String)> = match present_range(unit, shown) {
        HistoryRange::Hour | HistoryRange::SixHours | HistoryRange::Day => [0u32, 6, 12, 18]
            .into_iter()
            .map(|h| (f64::from(h) / 24.0, format!("{h:02}:00")))
            .collect(),
        HistoryRange::Week => (0..n)
            .map(|i| {
                let d = shown.from + Duration::days(i);
                (at(i), fmt::weekday_short(lang, d.weekday()).to_owned())
            })
            .collect(),
        HistoryRange::Month => {
            let step = if unit == Unit::Month { 7 } else { (n + 4) / 5 };
            (0..n)
                .step_by(step as usize)
                .map(|i| {
                    let d = shown.from + Duration::days(i);
                    (at(i), format!("{:02}", d.day()))
                })
                .collect()
        }
        HistoryRange::Year => month_ticks(lang, shown, n, by_slot),
    };
    if let Some(p) = now_pos {
        out.retain(|(pos, _)| (pos - p).abs() >= NOW_GAP);
        out.push((p, history_text(lang, HistoryText::Now).into()));
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    out
}

/// One label per month the window touches (every few months for a long window), at the middle
/// of the days of that month inside the window, or of the month's slot when `by_slot`.
fn month_ticks(lang: Lang, shown: &WindowReq, n: i64, by_slot: bool) -> Vec<(f64, String)> {
    let mut months: Vec<(f64, NaiveDate)> = Vec::new();
    let mut first = shown.from.with_day(1).unwrap_or(shown.from);
    while first <= shown.to {
        let next = first
            .checked_add_months(chrono::Months::new(1))
            .unwrap_or(NaiveDate::MAX);
        let start = first.max(shown.from);
        let end = (next - Duration::days(1)).min(shown.to);
        let a = (start - shown.from).num_days();
        let b = (end - shown.from).num_days() + 1;
        months.push(((a + b) as f64 / 2.0 / n as f64, first));
        if next == NaiveDate::MAX {
            break;
        }
        first = next;
    }
    if by_slot {
        // Month bars are equal slots, one per month the window touches.
        let slots = months.len() as f64;
        for (k, m) in months.iter_mut().enumerate() {
            m.0 = (k as f64 + 0.5) / slots;
        }
    }
    let step = months.len().div_ceil(MAX_MONTH_LABELS).max(1);
    let years = months.len() > MAX_MONTH_LABELS;
    months
        .into_iter()
        .step_by(step)
        .map(|(pos, d)| {
            let name = fmt::month_short(lang, d.month());
            let label = if years {
                format!("{name} {}", d.year())
            } else {
                name.to_owned()
            };
            (pos, label)
        })
        .collect()
}

#[cfg(test)]
mod tests;
