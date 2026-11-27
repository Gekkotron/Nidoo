//! Calendar periods for the history page: Day, Week, Month, Year or a custom
//! span, stepping between them and comparing with the same period of an
//! earlier year. Pure date logic; the screen only renders it.

use chrono::{Datelike, Months, NaiveDate, Weekday};
use nidoo_api::DailyPer;

#[cfg(test)]
mod tests;

/// What the picker's unit chips choose between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Hour1,
    Hour6,
    Day,
    Week,
    Month,
    Year,
    Custom,
}

impl Unit {
    pub fn slug(self) -> &'static str {
        match self {
            Unit::Hour1 => "1h",
            Unit::Hour6 => "6h",
            Unit::Day => "day",
            Unit::Week => "week",
            Unit::Month => "month",
            Unit::Year => "year",
            Unit::Custom => "custom",
        }
    }

    pub fn from_slug(s: &str) -> Option<Unit> {
        Some(match s {
            "1h" => Unit::Hour1,
            "6h" => Unit::Hour6,
            "day" => Unit::Day,
            "week" => Unit::Week,
            "month" => Unit::Month,
            "year" => Unit::Year,
            "custom" => Unit::Custom,
            _ => return None,
        })
    }

    fn is_hours(self) -> bool {
        matches!(self, Unit::Hour1 | Unit::Hour6)
    }
}

/// An inclusive range of days.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

impl Span {
    pub fn days(&self) -> i64 {
        (self.to - self.from).num_days() + 1
    }

    fn contains(&self, d: NaiveDate) -> bool {
        self.from <= d && d <= self.to
    }
}

/// The page's period, as the URL carries it. `at == None` means the current
/// period (the one containing today).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeriodState {
    pub unit: Unit,
    pub at: Option<NaiveDate>,
    pub custom: Option<Span>,
    pub vs: Option<u8>,
    pub show: Option<String>,
}

/// A `YYYY-MM-DD` date, zero-padded, in years 2000 to 2100: anything else (an unpadded date, a
/// signed or far-off year from a crafted link) is rejected, as the hub does.
pub fn parse_date(s: &str) -> Option<NaiveDate> {
    if s.len() != 10 {
        return None;
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .filter(|d| (2000..=2100).contains(&d.year()))
}

fn fmt_date(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// Reads the period from query parameters (`period`, `at`, `from`, `to`, `vs`,
/// `show`), falling back to the legacy `range` parameter.
pub fn parse_state(get: impl Fn(&str) -> Option<String>) -> PeriodState {
    let mut unit = Unit::Day;
    let mut at = None;
    let mut custom = None;
    if let Some(period) = get("period") {
        if let Some(u) = Unit::from_slug(&period) {
            unit = u;
            if u == Unit::Custom {
                let from = get("from").as_deref().and_then(parse_date);
                let to = get("to").as_deref().and_then(parse_date);
                match (from, to) {
                    (Some(from), Some(to)) if from <= to => custom = Some(Span { from, to }),
                    _ => unit = Unit::Day,
                }
            } else if !u.is_hours() {
                at = get("at").as_deref().and_then(parse_date);
            }
        }
    } else if let Some(range) = get("range") {
        unit = match range.as_str() {
            "1h" => Unit::Hour1,
            "6h" => Unit::Hour6,
            "7d" => Unit::Week,
            "30d" => Unit::Month,
            "1y" => Unit::Year,
            _ => Unit::Day,
        };
    }
    let vs = get("vs")
        .and_then(|v| v.parse::<u8>().ok())
        .filter(|v| *v >= 1);
    let show = get("show").filter(|s| !s.is_empty());
    PeriodState {
        unit,
        at,
        custom,
        vs,
        show,
    }
}

/// The page's address for a period.
pub fn state_path(id: &str, s: &PeriodState) -> String {
    let mut path = format!(
        "/history/{}?period={}",
        crate::api::encode_id(id),
        s.unit.slug()
    );
    if let Some(at) = s.at {
        path.push_str(&format!("&at={}", fmt_date(at)));
    }
    if s.unit == Unit::Custom
        && let Some(c) = s.custom
    {
        path.push_str(&format!("&from={}&to={}", fmt_date(c.from), fmt_date(c.to)));
    }
    if let Some(vs) = s.vs {
        path.push_str(&format!("&vs={vs}"));
    }
    if let Some(show) = s.show.as_deref() {
        path.push_str("&show=");
        path.push_str(&crate::api::encode_id(show));
    }
    path
}

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap_or(NaiveDate::MIN)
}

/// The calendar period containing `at`. Not for Hour1/Hour6/Custom (those
/// yield the single day).
pub fn span_of(unit: Unit, at: NaiveDate) -> Span {
    match unit {
        Unit::Week => {
            let from = at.week(Weekday::Mon).first_day();
            Span {
                from,
                to: from
                    .checked_add_signed(chrono::Duration::days(6))
                    .unwrap_or(from),
            }
        }
        Unit::Month => {
            let from = ymd(at.year(), at.month(), 1);
            let to = from
                .checked_add_months(Months::new(1))
                .map_or(at, |n| n - chrono::Duration::days(1));
            Span { from, to }
        }
        Unit::Year => Span {
            from: ymd(at.year(), 1, 1),
            to: ymd(at.year(), 12, 31),
        },
        _ => Span { from: at, to: at },
    }
}

/// The days the state shows; `None` for the hour views.
pub fn shown_span(s: &PeriodState, today: NaiveDate) -> Option<Span> {
    match s.unit {
        Unit::Hour1 | Unit::Hour6 => None,
        Unit::Custom => Some(s.custom.unwrap_or(Span {
            from: today,
            to: today,
        })),
        unit => Some(span_of(unit, s.at.unwrap_or(today))),
    }
}

/// Whether the shown period contains today.
pub fn is_current(s: &PeriodState, today: NaiveDate) -> bool {
    shown_span(s, today).is_none_or(|sp| sp.contains(today))
}

/// The state's `at` for a span: `None` for the current period.
fn at_for(span: Span, today: NaiveDate) -> Option<NaiveDate> {
    (!span.contains(today)).then_some(span.from)
}

/// The span `by` periods away, or `None` for the hour views.
fn moved_span(s: &PeriodState, by: i32, today: NaiveDate) -> Option<Span> {
    let shown = shown_span(s, today)?;
    Some(match s.unit {
        Unit::Custom => {
            let shift = chrono::Duration::days(shown.days() * i64::from(by));
            Span {
                from: shown.from + shift,
                to: shown.to + shift,
            }
        }
        Unit::Day | Unit::Week => {
            let n = if s.unit == Unit::Day { 1 } else { 7 };
            span_of(
                s.unit,
                shown.from + chrono::Duration::days(n * i64::from(by)),
            )
        }
        unit => {
            let months = Months::new(by.unsigned_abs() * if unit == Unit::Year { 12 } else { 1 });
            let from = if by >= 0 {
                shown.from.checked_add_months(months)
            } else {
                shown.from.checked_sub_months(months)
            }
            .unwrap_or(shown.from);
            span_of(unit, from)
        }
    })
}

/// Moves the period by `by` units (a custom span by its own length).
pub fn step(s: &PeriodState, by: i32, today: NaiveDate) -> PeriodState {
    let Some(span) = moved_span(s, by, today) else {
        return s.clone();
    };
    let mut next = s.clone();
    if s.unit == Unit::Custom {
        next.custom = Some(span);
        next.at = None;
    } else {
        next.at = at_for(span, today);
    }
    next
}

/// Whether the arrows can move: › not past today, ‹ not before the first data.
pub fn can_step(s: &PeriodState, by: i32, today: NaiveDate, first: Option<NaiveDate>) -> bool {
    let Some(shown) = shown_span(s, today) else {
        return false;
    };
    if by > 0 {
        moved_span(s, by, today).is_some_and(|next| next.from <= today)
    } else if by < 0 {
        first.is_none_or(|f| shown.from > f)
    } else {
        true
    }
}

/// Changes the unit, keeping the shown date, `vs` and `show`.
pub fn switch_unit(s: &PeriodState, unit: Unit, today: NaiveDate) -> PeriodState {
    let mut next = PeriodState {
        unit,
        at: None,
        custom: None,
        vs: s.vs,
        show: s.show.clone(),
    };
    let shown = shown_span(s, today);
    if unit == Unit::Custom {
        next.custom = Some(shown.unwrap_or(Span {
            from: today,
            to: today,
        }));
    } else if !unit.is_hours() && !is_current(s, today) {
        let base = shown.map_or(today, |sp| sp.from);
        next.at = at_for(span_of(unit, base), today);
    }
    next
}

fn shift_years(d: NaiveDate, years: u8) -> NaiveDate {
    let y = d.year() - i32::from(years);
    d.with_year(y).unwrap_or_else(|| ymd(y, d.month(), 28))
}

/// The same period `vs` years earlier (29 Feb becomes 28 Feb; a week is the
/// same ISO week number, 53 falling back to 52).
pub fn compare_span(unit: Unit, shown: Span, vs: u8) -> Span {
    if unit == Unit::Week {
        let iso = shown.from.iso_week();
        let year = iso.year() - i32::from(vs);
        let from = NaiveDate::from_isoywd_opt(year, iso.week(), Weekday::Mon)
            .or_else(|| NaiveDate::from_isoywd_opt(year, 52, Weekday::Mon));
        if let Some(from) = from {
            return Span {
                from,
                to: from + chrono::Duration::days(6),
            };
        }
    }
    Span {
        from: shift_years(shown.from, vs),
        to: shift_years(shown.to, vs),
    }
}

/// The year offsets whose compared span still reaches the first data.
pub fn compare_years(shown: Span, first: Option<NaiveDate>) -> Vec<u8> {
    let Some(first) = first else {
        return Vec::new();
    };
    (1..=u8::MAX)
        .take_while(|n| shift_years(shown.to, *n) >= first)
        .collect()
}

/// How a daily series is bucketed for a period.
pub fn per_for(unit: Unit, shown: Span) -> DailyPer {
    match unit {
        Unit::Day => DailyPer::Hour,
        Unit::Year => DailyPer::Month,
        Unit::Custom if shown.days() > 200 => DailyPer::Month,
        _ => DailyPer::Day,
    }
}

/// The unit a click on a bar zooms into.
pub fn zoom(unit: Unit, per: DailyPer) -> Option<Unit> {
    match (unit, per) {
        (Unit::Year, _) => Some(Unit::Month),
        (Unit::Week | Unit::Month | Unit::Custom, DailyPer::Day) => Some(Unit::Day),
        _ => None,
    }
}

/// 1 Nov to 31 Mar of the season containing `today` (the coming one from
/// April to October).
pub fn this_winter(today: NaiveDate) -> Span {
    let start = if today.month() <= 3 {
        today.year() - 1
    } else {
        today.year()
    };
    Span {
        from: ymd(start, 11, 1),
        to: ymd(start + 1, 3, 31),
    }
}

pub fn last_12_months(today: NaiveDate) -> Span {
    Span {
        from: shift_years(today, 1) + chrono::Duration::days(1),
        to: today,
    }
}

pub fn all_data(first: NaiveDate, today: NaiveDate) -> Span {
    Span {
        from: first,
        to: today,
    }
}
