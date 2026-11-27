//! The cards under a date window's chart: the shown period, the compared one and the
//! difference between them. What they say is decided by the pure [`summary`]; the component
//! only words it.

use leptos::prelude::*;
use nidoo_api::DailyPer;

use super::fetch::{Compared, State};
use super::format::state_axis_labels;
use super::window::{ChartWindow, runs_today, today_or};
use crate::history_compare::{counter_summary, counter_total, sensor_stats, sensor_summary};
use crate::history_period::{Span, per_for};
use crate::i18n::Lang;
use crate::i18n::format as fmt;
use crate::i18n::history::{
    HistoryText, history_text, kwh, left_out, min_max, no_data_for, period_name,
};
use chrono::NaiveDate;

/// What a card shows of one period.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Figures {
    /// A counter's energy over the period.
    Total(f64),
    /// A sensor's mean of means, lowest low and highest high.
    Stats { mean: f64, min: f64, max: f64 },
}

/// The shown period against the compared one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Diff {
    Total { diff: f64, pct: Option<f64> },
    Mean(f64),
}

/// The cards' content.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub shown: Figures,
    /// `None`: no comparison was asked for. `Some(None)`: asked, but it has no data to set
    /// against the shown period.
    pub compared: Option<Option<Figures>>,
    pub diff: Option<Diff>,
    /// Buckets left out of the comparison because only one period had them, counted in
    /// `left_per` units.
    pub left_out: usize,
    pub left_per: DailyPer,
}

/// A sensor's left-out buckets of `step` seconds, as a count of the nearest unit the text has
/// (hours below half a day, days below two weeks, else months), rounded up.
pub fn sensor_left_out(n: usize, step: i64) -> (usize, DailyPer) {
    let (unit, per) = match step {
        ..43_200 => (3_600, DailyPer::Hour),
        43_200..1_296_000 => (86_400, DailyPer::Day),
        _ => (2_629_746, DailyPer::Month),
    };
    (
        (n as i64 * step.max(1) + unit - 1).div_euclid(unit) as usize,
        per,
    )
}

/// Decide the cards from the replies. `asked` is whether the window has a comparison, `per` how
/// a counter is bucketed. With a comparison both periods are summed over the buckets they
/// share, so the difference compares like with like.
pub fn summary(
    shown: &State,
    compared: Option<&State>,
    asked: bool,
    per: DailyPer,
) -> Option<Summary> {
    match shown {
        State::Bars(d) if !d.days.is_empty() => {
            let cmp = match compared {
                Some(State::Bars(c)) => Some(c),
                _ => None,
            };
            let pair = match (cmp, d.span) {
                (Some(c), Some(s)) => c
                    .span
                    .and_then(|cs| counter_summary(&d.days, s.0, &c.days, cs.0, per)),
                _ => None,
            };
            Some(match pair {
                Some(c) => Summary {
                    shown: Figures::Total(c.shown),
                    compared: Some(Some(Figures::Total(c.compared))),
                    diff: Some(Diff::Total {
                        diff: c.diff,
                        pct: c.pct,
                    }),
                    left_out: c.left_out,
                    left_per: per,
                },
                None => Summary {
                    shown: Figures::Total(counter_total(&d.days)),
                    compared: asked.then_some(None),
                    diff: None,
                    left_out: 0,
                    left_per: per,
                },
            })
        }
        State::Series(s) => {
            let (mean, min, max) = sensor_stats(&s.points)?;
            let cmp = match compared {
                Some(State::Series(c)) => Some(c),
                _ => None,
            };
            let pair = match (cmp, s.span) {
                (Some(c), Some(sp)) => c.span.and_then(|cs| {
                    sensor_summary(&s.points, sp.0, &c.points, cs.0, s.window.max(1))
                }),
                _ => None,
            };
            Some(match pair {
                Some(c) => {
                    let (left_out, left_per) = sensor_left_out(c.left_out, s.window);
                    Summary {
                        left_out,
                        left_per,
                        shown: Figures::Stats {
                            mean: c.mean,
                            min: c.min,
                            max: c.max,
                        },
                        compared: Some(Some(Figures::Stats {
                            mean: c.cmp_mean,
                            min: c.cmp_min,
                            max: c.cmp_max,
                        })),
                        diff: Some(Diff::Mean(c.diff)),
                    }
                }
                None => Summary {
                    shown: Figures::Stats { mean, min, max },
                    compared: asked.then_some(None),
                    diff: None,
                    left_out: 0,
                    left_per: per,
                },
            })
        }
        _ => None,
    }
}

/// `v` with its sign: "+1.5", "−1.5" (a real minus), and a plain "0.0" when it rounds to zero.
pub fn signed(lang: Lang, v: f64, decimals: usize) -> String {
    let k = 10f64.powi(decimals as i32);
    let text = fmt::decimal_grouped(lang, v.abs(), decimals);
    match (v * k).round() {
        r if r > 0.0 => format!("+{text}"),
        r if r < 0.0 => format!("\u{2212}{text}"),
        _ => text,
    }
}

fn card(name: String, value: String, sub: Option<String>, class: &'static str) -> AnyView {
    view! {
        <div class=format!("history-summary__card {class}")>
            <span class="history-summary__name">{name}</span>
            <strong class="history-summary__value">{value}</strong>
            {sub.map(|s| view! { <span class="history-summary__sub">{s}</span> })}
        </div>
    }
    .into_any()
}

/// A period's card: the value, and under it what the value is.
fn figures_card(
    lang: Lang,
    name: String,
    f: Figures,
    unit: Option<&str>,
    class: &'static str,
) -> AnyView {
    match f {
        Figures::Total(t) => card(
            name,
            kwh(lang, t),
            Some(history_text(lang, HistoryText::Total).to_owned()),
            class,
        ),
        Figures::Stats { mean, min, max } => card(
            name,
            fmt::with_unit(fmt::decimal_grouped(lang, mean, 1), unit),
            Some(format!(
                "{} · {}",
                history_text(lang, HistoryText::Mean),
                min_max(lang, min, max, unit)
            )),
            class,
        ),
    }
}

fn diff_card(lang: Lang, d: Diff, unit: Option<&str>) -> AnyView {
    let name = history_text(lang, HistoryText::Difference).to_owned();
    match d {
        Diff::Total { diff, pct } => {
            let class = if diff > 0.0 {
                "history-summary__card--up"
            } else if diff < 0.0 {
                "history-summary__card--down"
            } else {
                ""
            };
            card(
                name,
                fmt::with_unit(signed(lang, diff, 1), Some("kWh")),
                pct.map(|p| fmt::with_unit(signed(lang, p, 0), Some("%"))),
                class,
            )
        }
        Diff::Mean(m) => card(
            name,
            fmt::with_unit(signed(lang, m, 1), unit),
            Some(history_text(lang, HistoryText::Mean).to_owned()),
            "",
        ),
    }
}

/// The shown period, the compared one and their difference, for a date window. A comparison with
/// nothing to set against says so and has no difference card.
#[component]
pub fn PeriodSummary(
    lang: Lang,
    window: ChartWindow,
    shown: State,
    compared: Compared,
    /// The entity's kind, which tells a state (no summary) from a number.
    kind: String,
    unit: Option<String>,
    /// The hub's today, when known.
    today: Option<NaiveDate>,
) -> impl IntoView {
    let ChartWindow::Dates {
        shown: req,
        unit: period_unit,
        compare,
    } = window
    else {
        return ().into_any();
    };
    if matches!(shown, State::Series(_)) && state_axis_labels(lang, None, &kind).is_some() {
        return ().into_any();
    }
    let span = |w: &crate::api::history::WindowReq| Span {
        from: w.from,
        to: w.to,
    };
    let per = per_for(period_unit, span(&req));
    let pending = compared.is_pending();
    let Some(mut s) = summary(&shown, compared.ready(), compare.is_some(), per) else {
        return ().into_any();
    };
    let running = runs_today(
        &req,
        today_or(today, (crate::energy::now_ms() / 1000) as i64),
    );
    let unit = unit.as_deref();
    let mut shown_name = period_name(lang, period_unit, span(&req));
    if running {
        shown_name = format!("{shown_name} · {}", history_text(lang, HistoryText::SoFar));
    }
    let shown_card = figures_card(lang, shown_name, s.shown, unit, "");
    if pending {
        // No difference to show until the compared period is in.
        s.diff = None;
    }
    let cmp_card = compare.map(|c| {
        let name = period_name(lang, period_unit, span(&c));
        if pending {
            return view! {
                <div class="history-summary__card history-summary__card--pending">
                    <span class="history-summary__name">{name}</span>
                    <span class="history-summary__value" aria-hidden="true"></span>
                </div>
            }
            .into_any();
        }
        match s.compared.flatten() {
            Some(f) => {
                let label = if running {
                    format!("{name} · {}", history_text(lang, HistoryText::SameDays))
                } else {
                    name
                };
                figures_card(lang, label, f, unit, "history-summary__card--compare")
            }
            None => view! {
                <div class="history-summary__card history-summary__card--empty">
                    <span class="history-summary__name">{no_data_for(lang, &name)}</span>
                </div>
            }
            .into_any(),
        }
    });
    let diff = s.diff.map(|d| diff_card(lang, d, unit));
    let note = (s.left_out > 0).then(
        || view! { <p class="history-summary__note">{left_out(lang, s.left_out, s.left_per)}</p> },
    );
    view! {
        <div class="history-summary">
            {shown_card}
            {cmp_card}
            {diff}
        </div>
        {note}
    }
    .into_any()
}

#[cfg(test)]
mod tests;
