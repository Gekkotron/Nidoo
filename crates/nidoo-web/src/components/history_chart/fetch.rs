//! What a `HistoryChart` fetches and how a reply becomes chart state: the routing decision
//! (`plan_fetch`), the calls, and the rules for the shown and the compared replies.

use nidoo_api::{DailyPer, EnergyKind, HistoryDaily, HistoryRange, HistorySeries};

use super::view::Flavor;
use super::window::{ChartWindow, window_per};
use crate::api::history::{
    HistoryApiError, WindowReq, fetch_daily, fetch_daily_window, fetch_series, fetch_series_window,
};
use leptos::prelude::*;

/// What the chart is currently drawing.
#[derive(Clone, Debug)]
pub(super) enum State {
    Loading,
    Series(HistorySeries),
    Bars(HistoryDaily),
    NotRecorded,
    Unavailable,
    Error(String),
}

/// A late response's `request_id` must match the latest issued one, or the response is dropped.
pub fn is_stale(my_req: u32, latest: u32) -> bool {
    my_req != latest
}

/// Days to ask `/daily` for, given a non-24h range.
pub(super) fn daily_days_for(range: HistoryRange) -> u16 {
    match range {
        HistoryRange::Week => 7,
        HistoryRange::Month => 30,
        HistoryRange::Year => 365,
        HistoryRange::Hour | HistoryRange::SixHours | HistoryRange::Day => 1,
    }
}

pub(super) enum FetchResult {
    Series(Result<HistorySeries, HistoryApiError>),
    Daily(Result<HistoryDaily, HistoryApiError>),
}

impl FetchResult {
    /// The unix `[start, end]` of a successful date-window reply.
    pub(super) fn span(&self) -> Option<(i64, i64)> {
        match self {
            FetchResult::Series(Ok(s)) => s.span,
            FetchResult::Daily(Ok(d)) => d.span,
            _ => None,
        }
    }
}

/// Which endpoint to call, with its arguments; pure routing decision for the current chart
/// state. Extracted from `fetch_plan` so it is natively testable — the Review Focus items
/// (text-entity 404, counter without power pair on 24h) are branches here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchPlan {
    /// Call `/history/series?entity={0}&range={range}`.
    Series(String),
    /// Call `/history/daily?entity={0}&days={1}&kind=counter`.
    Daily(String, u16),
    /// Call `/history/series` over a date window.
    SeriesWindow(String, WindowReq),
    /// Call `/history/daily` over a date window, bucketed per hour, day or month.
    DailyWindow(String, WindowReq, DailyPer),
}

/// Decide which endpoint to fetch for the current `(flavor, window, power_pair)` triple.
pub fn plan_fetch(
    entity: &str,
    window: &ChartWindow,
    flavor: Flavor,
    power_pair: Option<&str>,
) -> FetchPlan {
    match (window, flavor) {
        (ChartWindow::Dates { shown, .. }, Flavor::Series) => {
            FetchPlan::SeriesWindow(entity.to_owned(), *shown)
        }
        (ChartWindow::Dates { shown, .. }, Flavor::Counter) => FetchPlan::DailyWindow(
            entity.to_owned(),
            *shown,
            window_per(window, flavor).unwrap_or_default(),
        ),
        (ChartWindow::Preset(_), Flavor::Series) => FetchPlan::Series(entity.to_owned()),
        (
            ChartWindow::Preset(HistoryRange::Hour | HistoryRange::SixHours | HistoryRange::Day),
            Flavor::Counter,
        ) => match power_pair {
            Some(pid) => FetchPlan::Series(pid.to_owned()),
            None => FetchPlan::Daily(entity.to_owned(), 1),
        },
        (ChartWindow::Preset(r), Flavor::Counter) => {
            FetchPlan::Daily(entity.to_owned(), daily_days_for(*r))
        }
    }
}

/// The compared window's plan: the shown plan over `req`, bucketed the same way.
pub(super) fn compared_plan(shown: &FetchPlan, req: WindowReq) -> FetchPlan {
    match shown {
        FetchPlan::SeriesWindow(id, _) => FetchPlan::SeriesWindow(id.clone(), req),
        FetchPlan::DailyWindow(id, _, per) => FetchPlan::DailyWindow(id.clone(), req, *per),
        other => other.clone(),
    }
}

/// The unit of the line `plan` draws for `entity`: `unit` for the entity's own series, else the
/// drawn entity's (`unit_of`).
pub fn drawn_unit(
    plan: &FetchPlan,
    entity: &str,
    unit: Option<String>,
    unit_of: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    match plan {
        FetchPlan::Series(drawn) if drawn != entity => unit_of(drawn),
        _ => unit,
    }
}

/// Run `plan`; `range` is the preset a plain `Series` plan is for.
pub(super) async fn fetch_plan(plan: FetchPlan, range: HistoryRange) -> FetchResult {
    match plan {
        FetchPlan::Series(id) => FetchResult::Series(fetch_series(&id, range).await),
        FetchPlan::Daily(id, days) => {
            FetchResult::Daily(fetch_daily(&id, days, EnergyKind::Counter).await)
        }
        FetchPlan::SeriesWindow(id, w) => FetchResult::Series(fetch_series_window(&id, &w).await),
        FetchPlan::DailyWindow(id, w, per) => {
            FetchResult::Daily(fetch_daily_window(&id, &w, per, EnergyKind::Counter).await)
        }
    }
}

/// Put the shown reply on screen. A failed refresh keeps what is drawn; one that superseded the
/// first fetch still has to end the shimmer.
pub(super) fn apply_shown(state: RwSignal<State>, outcome: FetchResult, refresh: bool) {
    match outcome {
        FetchResult::Series(Ok(s)) => state.set(State::Series(s)),
        FetchResult::Daily(Ok(d)) => state.set(State::Bars(d)),
        FetchResult::Series(Err(e)) | FetchResult::Daily(Err(e)) => {
            if !refresh || state.with_untracked(|s| matches!(s, State::Loading)) {
                state.set(map_error(e));
            }
        }
    }
}

/// The compared reply as chart state; a failed or empty one is no comparison at all.
pub(super) fn compared_state(outcome: FetchResult) -> Option<State> {
    match outcome {
        FetchResult::Series(Ok(s)) if !s.points.is_empty() => Some(State::Series(s)),
        FetchResult::Daily(Ok(d)) if !d.days.is_empty() => Some(State::Bars(d)),
        _ => None,
    }
}

/// What to write to the compared state after a reply: `None` leaves it as it is (a failed
/// reply during a refresh keeps the comparison drawn, like the shown series); `Some(x)` sets it.
pub(super) fn compared_next(outcome: FetchResult, refresh: bool) -> Option<Option<State>> {
    let failed = matches!(
        outcome,
        FetchResult::Series(Err(_)) | FetchResult::Daily(Err(_))
    );
    if refresh && failed {
        None
    } else {
        Some(compared_state(outcome))
    }
}

pub(super) fn map_error(err: HistoryApiError) -> State {
    match err {
        HistoryApiError::NotRecorded | HistoryApiError::NoGridMeter => State::NotRecorded,
        HistoryApiError::Unavailable => State::Unavailable,
        HistoryApiError::BadRequest(m) | HistoryApiError::Other(m) => State::Error(m),
    }
}
