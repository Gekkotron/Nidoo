//! What a `HistoryChart` is asked to draw, and the pure rules around a date window: the
//! presentation range its labels borrow, the compared window's request, the refresh rule and
//! the x domain.

use chrono::{Datelike, NaiveDate};
use nidoo_api::{DailyPer, HistoryRange};

use super::view::Flavor;
use crate::api::history::WindowReq;
use crate::history_period::{Span, Unit, per_for};

/// The stretch of time a chart draws.
#[derive(Clone, Debug, PartialEq)]
pub enum ChartWindow {
    /// The last `HistoryRange` ending now (dashboard widgets, the legacy chips).
    Preset(HistoryRange),
    /// A date window, optionally set against the same period of an earlier year.
    Dates {
        shown: WindowReq,
        unit: Unit,
        compare: Option<WindowReq>,
    },
}

/// Days a window covers, both ends included.
pub fn window_days(w: &WindowReq) -> i64 {
    (w.to - w.from).num_days() + 1
}

/// Whether `today` falls inside the window.
pub fn runs_today(w: &WindowReq, today: NaiveDate) -> bool {
    w.from <= today && today <= w.to
}

/// The preset range whose labels and bucket style a date window borrows.
pub fn present_range(unit: Unit, shown: &WindowReq) -> HistoryRange {
    match unit {
        Unit::Day | Unit::Hour1 | Unit::Hour6 => HistoryRange::Day,
        Unit::Week => HistoryRange::Week,
        Unit::Month => HistoryRange::Month,
        Unit::Year => HistoryRange::Year,
        Unit::Custom => match window_days(shown) {
            ..=1 => HistoryRange::Day,
            2..=8 => HistoryRange::Week,
            9..=32 => HistoryRange::Month,
            _ => HistoryRange::Year,
        },
    }
}

/// The compared window's request: while the shown period is still running it is cut as far into
/// its own last day as the shown one has got (`span` is the shown reply's unix `[start, end]`).
pub fn compare_request(shown_span: (i64, i64), running: bool, cmp: WindowReq) -> WindowReq {
    WindowReq {
        upto: running.then(|| shown_span.1 - shown_span.0),
        ..cmp
    }
}

/// Whether an open chart refetches on its refresh tick: presets always; a date window only
/// while it holds today (a past period never changes).
pub fn refetches(window: &ChartWindow, today: NaiveDate) -> bool {
    match window {
        ChartWindow::Preset(_) => true,
        ChartWindow::Dates { shown, .. } => runs_today(shown, today),
    }
}

/// The x domain of a date window: the whole period from the reply's start, so a running month
/// leaves the days to come empty. `span` is the reply's unix `[start, end]`.
pub fn x_domain(shown: &WindowReq, span: (i64, i64)) -> (i64, i64) {
    (span.0, span.0 + window_secs(shown))
}

/// Unix second of the house-local midnight starting `d`. The offset is read at the previous
/// noon, which is before any change of the night.
fn local_midnight(d: NaiveDate) -> i64 {
    let utc = (d.num_days_from_ce() as i64 - 719_163) * 86_400;
    utc - i64::from(super::format::paris_offset(utc - 43_200))
}

/// Real seconds a window lasts: a day with a clock change has 23 or 25 hours.
pub fn window_secs(w: &WindowReq) -> i64 {
    let after = w.to.succ_opt().unwrap_or(w.to);
    local_midnight(after) - local_midnight(w.from)
}

/// Calendar months a window touches.
pub fn window_months(w: &WindowReq) -> usize {
    let ym = |d: NaiveDate| i64::from(d.year()) * 12 + i64::from(d.month0());
    (ym(w.to) - ym(w.from) + 1).max(1) as usize
}

/// The x domain of any window at unix second `now`; a date window with no reply span yet falls
/// back to the days ending now.
pub fn domain(window: &ChartWindow, span: Option<(i64, i64)>, now: i64) -> (i64, i64) {
    match window {
        ChartWindow::Preset(r) => (now - r.span_secs(), now),
        ChartWindow::Dates { shown, .. } => match span {
            Some(span) => x_domain(shown, span),
            None => (now - window_secs(shown), now),
        },
    }
}

/// The range a window is worded and bucketed as.
pub fn window_range(window: &ChartWindow) -> HistoryRange {
    match window {
        ChartWindow::Preset(r) => *r,
        ChartWindow::Dates { shown, unit, .. } => present_range(*unit, shown),
    }
}

/// The longest date window whose value chips are fetched: a fields reply over more is too big.
const FIELDS_MAX_DAYS: i64 = 400;

/// Whether the chart fetches the value chips for this window.
pub fn has_fields(window: &ChartWindow) -> bool {
    match window {
        ChartWindow::Preset(_) => true,
        ChartWindow::Dates { shown, .. } => window_days(shown) <= FIELDS_MAX_DAYS,
    }
}

/// The compared window's request, if it can be made yet: a running period needs the shown
/// reply's span to cut it, so with no span (the shown fetch failed) there is none; a finished
/// period needs nothing.
pub fn compared_req(shown: Option<(i64, i64)>, running: bool, cmp: WindowReq) -> Option<WindowReq> {
    if running {
        shown.map(|span| compare_request(span, true, cmp))
    } else {
        Some(cmp)
    }
}

/// How a date window's counter is bucketed; `None` for presets and for series.
pub fn window_per(window: &ChartWindow, flavor: Flavor) -> Option<DailyPer> {
    match (window, flavor) {
        (ChartWindow::Dates { shown, unit, .. }, Flavor::Counter) => Some(per_for(
            *unit,
            Span {
                from: shown.from,
                to: shown.to,
            },
        )),
        _ => None,
    }
}

/// The house-local date at unix second `now`, from the browser's clock. The chart uses it only
/// until (or without) the hub's own `today`, which the history page passes from `/history/extent`.
pub fn today_or(today: Option<NaiveDate>, now: i64) -> NaiveDate {
    today.unwrap_or_else(|| local_today(now))
}

/// The house-local date at unix second `now`.
pub fn local_today(now: i64) -> NaiveDate {
    let local = now + i64::from(super::format::paris_offset(now));
    NaiveDate::from_num_days_from_ce_opt(719_163 + local.div_euclid(86_400) as i32)
        .unwrap_or(NaiveDate::MIN)
}
