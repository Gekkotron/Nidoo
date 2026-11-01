//! The solar card of the home header: today's kWh, the day curve and tomorrow, one link to the
//! solar page.

use chrono::{DateTime, NaiveDate, Utc};
use leptos::prelude::*;
use leptos_router::components::A;
use nidoo_api::{SolarPeriod, SolarView};

use super::curve::{day_curve, producing_span};
use super::measured::SolarMeasured;
use super::model::{day_bounds, forecast_so_far, local_date, periods_in, tomorrow};
use crate::components::now::{Now, unix_now};
use crate::house::House;
use crate::i18n::{self, Lang, SolarText};
use crate::icons;
use crate::screens::flow_canvas::sun::house_place;

const W: f64 = 200.0;
const H: f64 = 60.0;

/// Everything the card draws, recomputed when the forecast, the clock or the measures change.
#[derive(Clone, Debug, Default, PartialEq)]
struct CardData {
    /// Measured production and forecast total, kWh; the first is `None` while nothing is measured.
    measured: Option<f64>,
    so_far: f64,
    total: Option<f64>,
    remaining: Option<f64>,
    tomorrow: Option<(f64, f64, f64)>,
    stale_at: Option<chrono::NaiveTime>,
    forecast_area: String,
    forecast_line: String,
    measured_line: String,
    measured_area: String,
    /// The measured line and area are the forecast's own estimates, not production.
    measured_is_forecast: bool,
    now_x: Option<f64>,
    /// `y` of the highest plotted value, where the now line starts.
    now_top: f64,
}

/// The measured points of the curve, `(period end, kW)`, and whether they are the forecast's own
/// estimates (nothing is measured): the periods already over.
pub(crate) fn curve_points(
    real: Vec<(u64, f64)>,
    day: &[SolarPeriod],
    now: u64,
) -> (Vec<(u64, f64)>, bool) {
    if real.is_empty() {
        let est = day
            .iter()
            .filter(|p| p.end <= now)
            .map(|p| (p.end, p.estimate))
            .collect();
        (est, true)
    } else {
        (real, false)
    }
}

/// Whether the hub's `today` block is about the house-local `today`. Just after midnight it can
/// still describe yesterday.
pub(crate) fn today_is_current(s: &SolarView, today: NaiveDate) -> bool {
    s.today.date == today.format("%Y-%m-%d").to_string()
}

/// Today's forecast total, kWh: the hub's figure when its `today` block is current, else the
/// sum of the day's periods.
pub(crate) fn today_total(s: &SolarView, day: &[SolarPeriod], today: NaiveDate) -> Option<f64> {
    let hub = today_is_current(s, today)
        .then_some(s.today.estimate)
        .flatten();
    hub.or_else(|| {
        (!day.is_empty()).then(|| {
            day.iter()
                .map(|p| p.estimate * f64::from(p.minutes) / 60.0)
                .sum()
        })
    })
}

fn card_data(house: House, now: u64, measured: Option<SolarMeasured>) -> Option<CardData> {
    let zone = house.flows_config.with(|c| house_place(c.as_ref()).1);
    house.solar.with(|s| {
        let s = s.as_ref()?;
        let today = local_date(zone, now);
        let (from, to) = day_bounds(zone, today);
        let day = periods_in(&s.periods, from, to);
        let today_kwh = measured.and_then(|m| m.today_kwh.get());
        let real: Vec<(u64, f64)> = measured.map(|m| m.periods.get()).unwrap_or_default();
        let (points, is_forecast) = curve_points(real, &day, now);
        let so_far = forecast_so_far(&day, now);
        let total = today_total(s, &day, today);
        let mut data = CardData {
            measured: today_kwh,
            so_far,
            total,
            remaining: s.now.remaining_today,
            tomorrow: tomorrow(s, today).map(|d| (d.estimate, d.low, d.high)),
            stale_at: s
                .stale
                .then(|| {
                    DateTime::<Utc>::from_timestamp(s.fetched_at as i64, 0)
                        .map(|t| t.with_timezone(&zone).time())
                })
                .flatten(),
            measured_is_forecast: is_forecast,
            ..CardData::default()
        };
        if let Some(span) = producing_span(&day) {
            let c = day_curve(&day, &points, span, now, W, H);
            data.forecast_area = c.forecast_area;
            data.forecast_line = c.forecast_line;
            data.measured_line = c.measured_line;
            data.measured_area = c.measured_area;
            data.now_x = c.now_x;
            data.now_top = c.now_top;
        }
        Some(data)
    })
}

/// The solar card: a link to the solar page.
#[component]
pub fn SolarCard() -> impl IntoView {
    let house: House = expect_context();
    let lang = use_context::<RwSignal<Lang>>().unwrap_or_else(|| RwSignal::new(Lang::En));
    let now = use_context::<Now>().map_or_else(|| RwSignal::new(unix_now()), |n| n.0);
    let measured = use_context::<SolarMeasured>();
    let data = Memo::new(move |_| card_data(house, now.get(), measured));

    let title = move || {
        let d = data.get()?;
        let l = lang.get();
        Some(i18n::solar_title(l, d.measured, d.so_far, d.total))
    };
    let bottom = move || {
        let d = data.get()?;
        let l = lang.get();
        let line = match d.stale_at {
            Some(at) => i18n::data_from(l, at),
            None => i18n::solar_bottom(l, d.remaining, d.measured, d.tomorrow),
        };
        (!line.is_empty()).then_some(line)
    };
    let field = move |pick: fn(&CardData) -> String| {
        move || data.get().map(|d| pick(&d)).unwrap_or_default()
    };
    // Production is a line over the forecast; while nothing is measured, the hours already past
    // are a faint fill under the forecast instead.
    let measured_area = move || {
        data.get()
            .filter(|d| d.measured_is_forecast)
            .map(|d| d.measured_area)
            .unwrap_or_default()
    };
    let measured_line = move || {
        data.get()
            .filter(|d| !d.measured_is_forecast)
            .map(|d| d.measured_line)
            .unwrap_or_default()
    };
    let now_line = move || {
        data.get().and_then(|d| {
            d.now_x.map(|x| {
                view! { <line class="solar-curve__now" x1=x x2=x y1=d.now_top y2=H /> }
            })
        })
    };

    view! {
        <A href="/solar" attr:class="solar-card" attr:aria-label=move || i18n::solar_text(lang.get(), SolarText::CardLabel)>
            <span class="solar-card__head">
                <span class="solar-card__icon" inner_html=icons::SUN></span>
                <span class="solar-card__title">{title}</span>
            </span>
            <svg class="solar-curve" viewBox="0 0 200 60" preserveAspectRatio="none" aria-hidden="true">
                <path class="solar-curve__forecast" d=field(|d| d.forecast_area.clone()) />
                <path class="solar-curve__measured" d=measured_area />
                <path class="solar-curve__line" d=field(|d| d.forecast_line.clone()) />
                <path class="solar-curve__measured-line" d=measured_line />
                {now_line}
            </svg>
            {move || bottom().map(|b| view! { <span class="solar-card__line">{b}</span> })}
        </A>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(today_date: &str) -> SolarView {
        serde_json::from_value(serde_json::json!({
            "fetched_at": 0, "stale": false, "next_refresh_at": null,
            "calls_today": 1, "daily_limit": 10,
            "now": {"power": null, "remaining_today": null},
            "today": {"date": today_date, "estimate": 12.4, "low": 8.0, "high": 15.0,
                      "peak": null, "best_hours": null},
            "days": [], "sites": [], "periods": [],
            "history": {"forecasts": []}
        }))
        .expect("fixture")
    }

    fn period(end: u64, estimate: f64) -> SolarPeriod {
        serde_json::from_value(serde_json::json!({
            "end": end, "minutes": 30, "estimate": estimate, "low": estimate, "high": estimate
        }))
        .expect("period")
    }

    #[test]
    fn stale_today_block_falls_back_to_the_period_sum() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        let day = [period(1800, 2.0), period(3600, 4.0)];
        let current = view("2026-10-25");
        assert!(today_is_current(&current, today));
        assert_eq!(today_total(&current, &day, today), Some(12.4));
        let yesterday = view("2026-10-24");
        assert!(!today_is_current(&yesterday, today));
        assert_eq!(today_total(&yesterday, &day, today), Some(3.0));
        assert_eq!(today_total(&yesterday, &[], today), None);
    }
}
