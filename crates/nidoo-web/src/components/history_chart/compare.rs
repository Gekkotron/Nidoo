//! What a date window adds to a chart: the compared period placed on the shown one's axis,
//! the bars of a window laid out by bucket, the running "now" mark, and the legend and tooltip
//! wording that names each period by its own date.

use chrono::{Datelike, NaiveDate};
use leptos::html;
use leptos::prelude::*;
use nidoo_api::{DailyPer, HistoryDaily, HistoryRange, HistorySeries};

use super::format::{tooltip_day, tooltip_time};
use super::geom::{slot_index, tooltip_flips};
use super::ticks::window_ticks;
use super::view::{pct, pointer_frac};
use super::window::{
    ChartWindow, local_today, runs_today, today_or, window_days, window_months, window_secs,
};
use crate::api::history::WindowReq;
use crate::components::history_chart::format::tick_labels;
use crate::history_compare::{Bucketing, bucket_index, nominal};
use crate::history_period::{Span, Unit};
use crate::i18n::Lang;
use crate::i18n::history::{kwh, period_counter_legend, period_name, period_total_line};

/// Width of a shown bar, as a fraction of its slot.
pub const SHOWN_W: f64 = 0.8;
/// Width of a compared bar: wider, so it shows around the shown one.
pub const COMPARED_W: f64 = 0.96;

/// A compared-window bucket moved onto the shown window's x axis.
#[derive(Clone, Debug, PartialEq)]
pub struct Shifted<T> {
    /// The bucket's index from its window start (and so from the shown one's).
    pub idx: i64,
    /// Its time on the shown axis, unix seconds.
    pub t: i64,
    /// Its own time, for its own date in the tooltip.
    pub orig: i64,
    pub v: T,
}

/// Move the buckets of a compared reply (starting at `cmp_start`) onto the axis of the shown
/// one (starting at `shown_start`), by bucket index. Buckets beyond the shown reply's length
/// (`shown_len` seconds) or before its start are cut: the compared series never runs past the
/// shown period.
pub fn place<T: Copy>(
    items: &[(i64, T)],
    cmp_start: i64,
    shown_start: i64,
    shown_len: i64,
    bucketing: Bucketing,
) -> Vec<Shifted<T>> {
    let bucketing = match bucketing {
        Bucketing::Nominal(n) => Bucketing::Nominal(n.max(1)),
        months => months,
    };
    // Seconds one bucket stands for on the shown axis, and how many buckets the shown reply has.
    let (step, buckets) = match bucketing {
        Bucketing::Nominal(n) => (n, (shown_len + n - 1).div_euclid(n)),
        Bucketing::Months if shown_len > 0 => (
            nominal(DailyPer::Month),
            Bucketing::Months.index(shown_start + shown_len - 1, shown_start) + 1,
        ),
        Bucketing::Months => (nominal(DailyPer::Month), 0),
    };
    items
        .iter()
        .filter_map(|&(orig, v)| {
            let idx = bucketing.index(orig, cmp_start);
            (idx >= 0 && idx < buckets).then_some(Shifted {
                idx,
                t: shown_start + idx * step,
                orig,
                v,
            })
        })
        .collect()
}

/// [`place`] for series points `(t, mean, min, max)`.
pub fn place_points(
    points: &[(i64, f64, f64, f64)],
    cmp_start: i64,
    shown_start: i64,
    shown_len: i64,
    bucketing: Bucketing,
) -> Vec<Shifted<(f64, f64, f64)>> {
    let items: Vec<(i64, (f64, f64, f64))> = points
        .iter()
        .map(|&(t, m, lo, hi)| (t, (m, lo, hi)))
        .collect();
    place(&items, cmp_start, shown_start, shown_len, bucketing)
}

/// The compared bucket sharing the bucket of the shown time `shown_t`.
pub fn compared_at<T>(
    cmp: &[Shifted<T>],
    shown_t: i64,
    shown_start: i64,
    bucketing: Bucketing,
) -> Option<&Shifted<T>> {
    let idx = bucketing.index(shown_t, shown_start);
    cmp.iter().find(|c| c.idx == idx)
}

/// How many bucket slots a window holds: hours by its real length (a clock-change day has 23 or
/// 25), days by its dates, months by the calendar months it touches (a window starting
/// mid-month has a short first one).
pub fn slots_for(per: DailyPer, req: &WindowReq) -> usize {
    match per {
        DailyPer::Hour => bucket_index(window_secs(req), 3_600).max(1) as usize,
        DailyPer::Day => window_days(req).max(1) as usize,
        DailyPer::Month => window_months(req),
    }
}

/// What one bar slot holds: the shown bucket and the compared one (its own time, value).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarSlot {
    pub shown: Option<(i64, f64)>,
    pub cmp: Option<(i64, f64)>,
}

impl BarSlot {
    pub fn has_data(&self) -> bool {
        self.shown.is_some() || self.cmp.is_some()
    }
}

/// All `n` slots of a window, each with the bars that fall in it: a running period leaves the
/// slots to come empty.
pub fn bar_slots(
    shown: &[(i64, f64)],
    cmp: &[Shifted<f64>],
    shown_start: i64,
    bucketing: Bucketing,
    n: usize,
) -> Vec<BarSlot> {
    let mut slots = vec![
        BarSlot {
            shown: None,
            cmp: None
        };
        n
    ];
    for &(t, v) in shown {
        let idx = bucketing.index(t, shown_start);
        if let Some(slot) = usize::try_from(idx).ok().and_then(|i| slots.get_mut(i))
            && slot.shown.is_none()
        {
            slot.shown = Some((t, v));
        }
    }
    for c in cmp {
        if let Some(slot) = usize::try_from(c.idx).ok().and_then(|i| slots.get_mut(i))
            && slot.cmp.is_none()
        {
            slot.cmp = Some((c.orig, c.v));
        }
    }
    slots
}

/// The tallest bar, so shown and compared share one scale.
pub fn slots_max(slots: &[BarSlot]) -> f64 {
    slots
        .iter()
        .flat_map(|s| [s.shown, s.cmp])
        .flatten()
        .map(|(_, v)| v)
        .fold(0.0, f64::max)
}

/// Left edge and width of slot `i` of `n`, as fractions of the plot, for a bar `frac` of a slot.
pub fn bar_box(i: usize, n: usize, frac: f64) -> (f64, f64) {
    (i as f64 / n as f64, frac / n as f64)
}

/// "Thu 26 Nov 2026" in the house zone: a date that tells its year, for a tooltip that sets two
/// years side by side.
pub fn day_when(lang: Lang, t: i64) -> String {
    format!("{} {}", tooltip_day(lang, t), local_today(t).year())
}

/// A series point's time with its date: "Thu 26 Nov 2026 14:00", or "Nov 2026" per month.
pub fn series_when(lang: Lang, range: HistoryRange, t: i64) -> String {
    match range {
        HistoryRange::Year => tooltip_time(lang, HistoryRange::Year, t, t),
        _ => format!(
            "{} {}",
            day_when(lang, t),
            tooltip_time(lang, HistoryRange::Day, t, t)
        ),
    }
}

/// A bar's date: its hour, day or month, with the year.
pub fn bar_when(lang: Lang, per: DailyPer, t: i64) -> String {
    match per {
        DailyPer::Hour => format!(
            "{} {}",
            day_when(lang, t),
            tooltip_time(lang, HistoryRange::Day, t, t)
        ),
        DailyPer::Day => day_when(lang, t),
        DailyPer::Month => tooltip_time(lang, HistoryRange::Year, t, t),
    }
}

/// A date window as the chart draws it: the periods' names and whether the shown one is running.
#[derive(Clone, Debug)]
pub struct DatesCtx {
    pub req: WindowReq,
    pub unit: Unit,
    pub running: bool,
    pub name: String,
    pub cmp_name: Option<String>,
}

fn span_of(w: &WindowReq) -> Span {
    Span {
        from: w.from,
        to: w.to,
    }
}

/// The drawing context of `window` at unix second `now`; `None` for a preset.
pub fn dates_ctx(
    lang: Lang,
    window: &ChartWindow,
    now: i64,
    today: Option<NaiveDate>,
) -> Option<DatesCtx> {
    let ChartWindow::Dates {
        shown,
        unit,
        compare,
    } = window
    else {
        return None;
    };
    Some(DatesCtx {
        req: *shown,
        unit: *unit,
        running: runs_today(shown, today_or(today, now)),
        name: period_name(lang, *unit, span_of(shown)),
        cmp_name: compare.map(|c| period_name(lang, *unit, span_of(&c))),
    })
}

impl DatesCtx {
    /// Where the running period has got to, as a fraction of the whole window; `None` when the
    /// period is over, or reaches its end.
    pub fn now_frac(&self, span: Option<(i64, i64)>) -> Option<f64> {
        let (a, b) = span.filter(|_| self.running)?;
        let frac = (b - a) as f64 / window_secs(&self.req) as f64;
        (0.0..1.0).contains(&frac).then_some(frac)
    }
}

/// The x axis of a chart: its labels, and whether they come from a date window (which has no
/// "now" at the right edge, so its labels centre on their tick).
#[derive(Clone, Debug)]
pub struct Axis {
    pub ticks: Vec<(f64, String)>,
    pub windowed: bool,
}

/// The labels of a chart: a preset's end at `domain_end`; a window's come from its own dates,
/// with "now" only while it runs. `span` is the reply's unix `[start, end]`.
pub fn axis_for(
    lang: Lang,
    ctx: Option<&DatesCtx>,
    range: HistoryRange,
    span: Option<(i64, i64)>,
    domain_end: i64,
    by_slot: bool,
) -> Axis {
    match ctx {
        Some(c) => Axis {
            ticks: window_ticks(lang, c.unit, &c.req, c.now_frac(span), by_slot),
            windowed: true,
        },
        None => Axis {
            ticks: tick_labels(lang, range, domain_end),
            windowed: false,
        },
    }
}

/// The x labels under a plot.
pub fn axis_x(axis: Axis) -> impl IntoView {
    let windowed = axis.windowed;
    view! {
        <div class="history-chart__axis-x" class:history-chart__axis-x--window=windowed>
            {axis.ticks.into_iter().map(|(pos, l)| {
                let near = !windowed && super::view::near_end(pos);
                let end = windowed && pos > 0.9;
                view! {
                    <span class:history-chart__tick--near-end=near
                        class:history-chart__tick--end=end
                        style=format!("left: {}%", pos * 100.0)>{l}</span>
                }
            }).collect::<Vec<_>>()}
        </div>
    }
}

/// The dotted line where the running period has got to.
pub fn now_mark(frac: Option<f64>) -> Option<impl IntoView + use<>> {
    frac.map(|f| view! { <div class="history-chart__now" style=format!("left: {}", pct(f))></div> })
}

/// The legend entry of the compared period.
pub fn compare_key(name: Option<String>) -> Option<impl IntoView + use<>> {
    name.map(|n| {
        view! {
            <span class="history-chart__key-item">
                <i class="history-chart__key-compare" aria-hidden="true"></i>
                {n}
            </span>
        }
    })
}

/// What a date window adds to a drawn series: the compared period on the shown axis and where
/// the running period has got to.
#[derive(Clone, Debug)]
pub struct SeriesDates {
    pub compared: Vec<Shifted<(f64, f64, f64)>>,
    pub cmp_name: Option<String>,
    /// The shown reply's start, and its bucket length, which index both series.
    pub start: i64,
    pub bucketing: Bucketing,
    pub now_frac: Option<f64>,
    /// The shown reply's end, while the period is running.
    pub running_to: Option<i64>,
}

/// The series extras of a date window: `cmp` is the compared reply, if it came back.
pub fn series_dates(
    ctx: &DatesCtx,
    shown: &HistorySeries,
    cmp: Option<&HistorySeries>,
) -> SeriesDates {
    let bucketing = Bucketing::Nominal(shown.window.max(1));
    let compared = match (cmp, shown.span) {
        (Some(c), Some(s)) => c
            .span
            .map(|cs| place_points(&c.points, cs.0, s.0, s.1 - s.0, bucketing))
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let now_frac = ctx.now_frac(shown.span);
    SeriesDates {
        cmp_name: ctx.cmp_name.clone().filter(|_| !compared.is_empty()),
        compared,
        start: shown.span.map_or(0, |s| s.0),
        bucketing,
        now_frac,
        running_to: shown.span.filter(|_| ctx.running).map(|s| s.1),
    }
}

/// A window of daily bars to draw.
pub struct BarsWindow {
    pub lang: Lang,
    pub shown: HistoryDaily,
    pub compared: Option<HistoryDaily>,
    pub per: DailyPer,
    pub ctx: DatesCtx,
    pub axis: Axis,
    /// Called with a clicked bucket's local date; `None` when the unit cannot zoom.
    pub zoom: Option<Callback<NaiveDate>>,
}

/// Bars laid out by bucket over the whole window (a running month leaves the rest of it empty),
/// with the compared period's bars behind, wider and faint.
pub fn render_bars_window(p: BarsWindow) -> impl IntoView {
    let BarsWindow {
        lang,
        shown,
        compared,
        per,
        ctx,
        axis,
        zoom,
    } = p;
    let nom = Bucketing::of(per);
    let n = slots_for(per, &ctx.req);
    let start = shown
        .span
        .map_or_else(|| shown.days.first().map_or(0, |d| d.0), |s| s.0);
    let cmp = match (&compared, shown.span) {
        (Some(c), Some(s)) => c
            .span
            .map(|cs| place(&c.days, cs.0, s.0, s.1 - s.0, nom))
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let has_cmp = !cmp.is_empty();
    let slots = bar_slots(&shown.days, &cmp, start, nom, n);
    let max = slots_max(&slots).max(f64::EPSILON);
    let total: f64 = shown.days.iter().map(|d| d.1).sum();
    let now_frac = ctx.now_frac(shown.span);
    let plot = NodeRef::<html::Div>::new();
    let hover = RwSignal::new(None::<usize>);
    let bar = |i: usize, v: f64, frac: f64| {
        let (x, w) = bar_box(i, n, frac);
        format!(
            "left: {}; width: {}; height: {}",
            pct(x),
            pct(w),
            pct(v / max)
        )
    };
    let cmp_bars = slots
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.cmp.map(|(_, v)| (i, v)))
        .map(|(i, v)| {
            view! { <div class="history-chart__bar history-chart__bar--compare" style=bar(i, v, COMPARED_W)></div> }
        })
        .collect::<Vec<_>>();
    let shown_bars = slots
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.shown.map(|(_, v)| (i, v)))
        .map(|(i, v)| {
            view! {
                <div class="history-chart__bar" style=bar(i, v, SHOWN_W)
                    class:history-chart__bar--on=move || hover.get() == Some(i)></div>
            }
        })
        .collect::<Vec<_>>();
    let slots = StoredValue::new(slots);

    let track = move |ev: web_sys::PointerEvent| {
        if let Some(f) = pointer_frac(&plot, &ev) {
            hover.set(slot_index(n, f));
        }
    };
    let tip = move || {
        hover
            .get()
            .and_then(|i| slots.with_value(|s| s.get(i).copied()).map(|s| (i, s)))
            .filter(|(_, s)| s.has_data())
    };
    let x_at = move || tip().map(|(i, _)| (i as f64 + 0.5) / n as f64);
    let click = move |_| {
        let bucket = hover
            .get_untracked()
            .and_then(|i| slots.with_value(|s| s.get(i).and_then(|s| s.shown)));
        if let (Some(zoom), Some((t, _))) = (zoom, bucket) {
            zoom.run(local_today(t));
        }
    };
    let (name, cmp_name) = (ctx.name.clone(), ctx.cmp_name.clone().filter(|_| has_cmp));

    view! {
        <div class="history-chart__plot" node_ref=plot
            class:history-chart__plot--zoom=zoom.is_some()
            on:pointermove=track on:pointerdown=track on:click=click
            on:pointerleave=move |_| hover.set(None)
            on:pointercancel=move |_| hover.set(None)>
            {cmp_bars}
            {shown_bars}
            {now_mark(now_frac)}
            <div class="history-chart__tip" hidden=move || tip().is_none()
                class:history-chart__tip--left=move || x_at().is_some_and(tooltip_flips)
                style=move || x_at().map(|x| format!("left: {}", pct(x))).unwrap_or_default()>
                {move || tip().map(|(_, s)| view! {
                    {s.shown.map(|(t, v)| view! {
                        <span class="history-chart__tip-time">{bar_when(lang, per, t)}</span>
                        <strong class="history-chart__tip-value">{kwh(lang, v)}</strong>
                    })}
                    {s.cmp.map(|(t, v)| view! {
                        <span class="history-chart__tip-time history-chart__tip-time--compare">{bar_when(lang, per, t)}</span>
                        <strong class="history-chart__tip-value history-chart__tip-value--compare">{kwh(lang, v)}</strong>
                    })}
                })}
            </div>
        </div>
        {axis_x(axis)}
        <div class="history-chart__legend">
            <span>{period_counter_legend(lang, &name, per)}</span>
            {compare_key(cmp_name)}
            <span>{period_total_line(lang, &name, total)}</span>
        </div>
    }
}

#[cfg(test)]
mod tests;
