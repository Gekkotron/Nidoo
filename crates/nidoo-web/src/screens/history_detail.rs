//! The full-screen `/history/:id?period=…` page: chrome row, the period bar, and a `HistoryChart`.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::NavigateOptions;
use leptos_router::hooks::{use_navigate, use_params_map, use_query_map};
use nidoo_api::{EntityView, HistoryExtent, HistoryRange};
use nidoo_core::energy::{EnergyMeters, EnergyUnits};
use nidoo_core::model::Kind;

use crate::api::history::{WindowReq, fetch_extent};
use crate::components::history_chart::format::state_axis_labels;
use crate::components::history_chart::window::local_today;
use crate::components::history_chart::{ChartWindow, Flavor, HistoryChart};
use crate::components::period_bar::PeriodBar;
use crate::energy::{energy_entities, energy_entity};
use crate::format::{area_name, shown_name, title_suffix, type_word, wrap_id};
use crate::history_period::{
    PeriodState, Span, Unit, compare_span, parse_date, parse_state, per_for, shown_span, span_of,
    state_path, switch_unit, zoom,
};
use crate::house::House;
use crate::i18n::devices::availability_text;
use crate::i18n::history::{HistoryText, history_text};
use crate::i18n::use_lang;
use crate::nav::BackLink;
use crate::screens::entity_sheet::availability_class;

/// Classify a unit as the counter flavour (energy totals: Wh/kWh/MWh) or the series flavour.
pub fn flavor_for_unit(unit: Option<&str>) -> Flavor {
    if EnergyUnits::factor(unit).is_some() {
        Flavor::Counter
    } else {
        Flavor::Series
    }
}

/// The power sensor paired with the energy counter `id`, from the live entity map.
pub fn power_pair_of(entities: &BTreeMap<String, EntityView>, id: &str) -> Option<String> {
    let all = energy_entities(entities);
    let me = entities.get(id)?;
    let ee = energy_entity(me);
    EnergyMeters::counter_power(&ee, &all).map(|p| p.id.clone())
}

/// The entity whose last 24 hours stand for `id`, as its history chart draws them: an energy
/// counter's paired power sensor, else `id` itself.
pub fn day_curve_of(entities: &BTreeMap<String, EntityView>, id: &str) -> String {
    entities
        .get(id)
        .filter(|e| flavor_for_unit(e.unit.as_deref()) == Flavor::Counter)
        .and_then(|_| power_pair_of(entities, id))
        .unwrap_or_else(|| id.to_owned())
}

/// What the chart draws for a period: the hour views are the last 1 or 6 hours; the rest is a
/// date window, set against the same period of an earlier year when `vs` says so.
pub fn chart_window(s: &PeriodState, today: NaiveDate) -> ChartWindow {
    match s.unit {
        Unit::Hour1 => ChartWindow::Preset(HistoryRange::Hour),
        Unit::Hour6 => ChartWindow::Preset(HistoryRange::SixHours),
        unit => {
            let span = shown_span(s, today).unwrap_or(Span {
                from: today,
                to: today,
            });
            let req = |sp: Span| WindowReq {
                from: sp.from,
                to: sp.to,
                upto: None,
            };
            ChartWindow::Dates {
                shown: req(span),
                unit,
                compare: s.vs.map(|v| req(compare_span(unit, span, v))),
            }
        }
    }
}

/// Whether to offer a comparison for a plot of known comparability (unknown: offer it).
pub fn compare_fits(known: Option<bool>) -> bool {
    known.unwrap_or(true)
}

/// The window without its comparison.
pub fn without_compare(w: ChartWindow) -> ChartWindow {
    match w {
        ChartWindow::Dates { shown, unit, .. } => ChartWindow::Dates {
            shown,
            unit,
            compare: None,
        },
        preset => preset,
    }
}

/// The period a click on a bucket dated `date` zooms into: a year opens its month, a week, month
/// or custom span opens the day. The comparison and the chosen values stay.
pub fn zoomed(s: &PeriodState, date: NaiveDate, today: NaiveDate) -> PeriodState {
    let shown = shown_span(s, today).unwrap_or(Span {
        from: today,
        to: today,
    });
    let unit = zoom(s.unit, per_for(s.unit, shown)).unwrap_or(Unit::Day);
    let span = span_of(unit, date);
    PeriodState {
        at: (!(span.from <= today && today <= span.to)).then_some(span.from),
        ..switch_unit(s, unit, today)
    }
}

#[component]
pub fn HistoryDetailScreen() -> impl IntoView {
    let house = expect_context::<House>();
    let params = use_params_map();
    let query = use_query_map();
    let navigate = use_navigate();
    let lang = use_lang();

    let id = Memo::new(move |_| params.read().get("id").unwrap_or_default());
    let state = Memo::new(move |_| {
        let q = query.read();
        parse_state(|k| q.get(k))
    });
    let show = Memo::new(move |_| state.get().show);

    // The hub's today and first day of data, asked once per entity. Until the reply lands the
    // browser's date stands in for today, and the arrows and the comparison stay off.
    let extent = RwSignal::new(None::<HistoryExtent>);
    Effect::new(move |_| {
        let asked = id.get();
        extent.set(None);
        spawn_local(async move {
            if let Ok(e) = fetch_extent(&asked).await
                && id.try_get_untracked().is_some_and(|cur| cur == asked)
            {
                extent.try_set(Some(e));
            }
        });
    });
    let hub_today =
        Memo::new(move |_| extent.with(|e| e.as_ref().and_then(|e| parse_date(&e.today))));
    let first = Memo::new(move |_| {
        extent.with(|e| {
            e.as_ref()
                .and_then(|e| e.first_date.as_deref())
                .and_then(parse_date)
        })
    });
    let today = Memo::new(move |_| {
        hub_today
            .get()
            .unwrap_or_else(|| local_today((crate::energy::now_ms() / 1000) as i64))
    });
    // Whether the plot drawn can be compared: `None` until it is known (and again for each new
    // entity). A plot of values or words draws no comparison, so none is asked for.
    let plot_comparable = RwSignal::new(None::<bool>);
    Effect::new(move |_| {
        id.track();
        plot_comparable.set(None);
    });
    let window = Memo::new(move |_| {
        let w = chart_window(&state.get(), today.get());
        if compare_fits(plot_comparable.get()) {
            w
        } else {
            without_compare(w)
        }
    });

    let entity: Memo<Option<EntityView>> =
        Memo::new(move |_| house.entities.with(|m| m.get(&id.get()).cloned()));

    let kind_str: Memo<String> = Memo::new(move |_| {
        entity
            .get()
            .map(|e| format!("{:?}", e.kind))
            .unwrap_or_default()
    });
    let unit_opt: Memo<Option<String>> = Memo::new(move |_| entity.get().and_then(|e| e.unit));
    let flavor_sig: Memo<Flavor> = Memo::new(move |_| flavor_for_unit(unit_opt.get().as_deref()));

    // For a counter, resolve its paired power sensor from the live entity map.
    let power_pair: Memo<Option<String>> = Memo::new(move |_| {
        if flavor_sig.get() != Flavor::Counter {
            return None;
        }
        let id = id.get();
        house.entities.with(|m| power_pair_of(m, &id))
    });

    let header = move || {
        entity.get().map(|e| {
            let area = house.areas.with(|areas| {
                areas
                    .iter()
                    .find(|a| a.id == e.area)
                    .map_or_else(|| e.area.clone(), |a| area_name(lang(), &a.id, &a.name))
            });
            let availability = e.state.availability;
            let title = match title_suffix(lang(), &e) {
                Some(suffix) => format!("{} · {suffix}", shown_name(lang(), &e)),
                None => shown_name(lang(), &e).to_owned(),
            };
            view! {
                <div class="sheet__titles">
                    <h1 class="sheet__title">{wrap_id(&title)}</h1>
                    <p class="sheet__subtitle">
                        {area} " · " {type_word(lang(), &e)} " · "
                        <span class=availability_class(availability)></span>
                        " "
                        {availability_text(lang(), availability)}
                    </p>
                </div>
            }
        })
    };

    // Every change of period or chips replaces the history entry, so Back leaves the page.
    let replace = {
        let navigate = navigate.clone();
        move |s: &PeriodState| {
            navigate(
                &state_path(&id.get_untracked(), s),
                NavigateOptions {
                    replace: true,
                    ..Default::default()
                },
            );
        }
    };
    let on_period = {
        let replace = replace.clone();
        Callback::new(move |s: PeriodState| replace(&s))
    };
    let on_show = {
        let replace = replace.clone();
        Callback::new(move |param: String| {
            let mut s = state.get_untracked();
            s.show = Some(param).filter(|p| !p.is_empty());
            replace(&s);
        })
    };
    let on_zoom = Callback::new(move |date: NaiveDate| {
        replace(&zoomed(&state.get_untracked(), date, today.get_untracked()));
    });
    let device_class: Memo<Option<String>> =
        Memo::new(move |_| entity.get().and_then(|e| e.device_class));
    let kind_sig: Memo<Kind> = Memo::new(move |_| entity.get().map_or(Kind::Generic, |e| e.kind));

    // A discrete entity (light, switch, cover) is drawn in words: nothing to set against.
    let compare_allowed = Signal::derive(move || {
        state_axis_labels(lang(), None, &kind_str.get()).is_none()
            && compare_fits(plot_comparable.get())
    });

    let known = Memo::new(move |_| house.entities.with(|m| m.contains_key(&id.get())));
    let loaded = Memo::new(move |_| {
        house.entities.with(|m| !m.is_empty())
            || matches!(house.connection.get(), nidoo_api::ConnectionStatus::Online)
    });

    view! {
        <div class="sheet history-detail">
            <header class="sheet__header">
                <BackLink />
                {header}
            </header>
            {move || {
                if known.get() {
                    view! {
                        <section class="sheet__panel">
                            <PeriodBar
                                state=state
                                today=today
                                first=first
                                on_change=on_period
                                compare_allowed=compare_allowed
                                ready=Signal::derive(move || hub_today.get().is_some())
                            />
                            <HistoryChart
                                entity_id=Signal::derive(move || id.get())
                                window=window
                                flavor=Signal::derive(move || flavor_sig.get())
                                kind=Signal::derive(move || kind_str.get())
                                unit=Signal::derive(move || unit_opt.get())
                                power_pair=Signal::derive(move || power_pair.get())
                                show=Signal::derive(move || show.get())
                                on_show=on_show
                                on_zoom=on_zoom
                                today=Signal::derive(move || hub_today.get())
                                device_class=Signal::derive(move || device_class.get())
                                field_kind=Signal::derive(move || kind_sig.get())
                                comparable=plot_comparable
                            />
                        </section>
                    }.into_any()
                } else if loaded.get() {
                    view! {
                        <section class="sheet__panel history-detail__unknown">
                            <p>{move || history_text(lang(), HistoryText::UnknownEntity)}</p>
                            <p class="sheet__subtitle">{move || wrap_id(&id.get())}</p>
                        </section>
                    }.into_any()
                } else {
                    view! { <div class="card card--skeleton" aria-busy="true"></div> }.into_any()
                }
            }}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plot_that_draws_no_comparison_asks_for_none() {
        use crate::components::history_chart::plot::{Plot, comparable};
        assert_eq!(comparable(&Plot::Loading), None);
        assert_eq!(comparable(&Plot::Bars), Some(true));
        assert_eq!(comparable(&Plot::Headline { overlay: None }), Some(true));
        assert_eq!(
            comparable(&Plot::Field {
                track: 0,
                overlay: None
            }),
            Some(false)
        );
        assert_eq!(comparable(&Plot::Words { track: 0 }), Some(false));
        assert!(compare_fits(None) && compare_fits(Some(true)) && !compare_fits(Some(false)));
        let today = NaiveDate::from_ymd_opt(2026, 11, 26).unwrap();
        let s = PeriodState {
            unit: Unit::Month,
            at: None,
            custom: None,
            vs: Some(1),
            show: None,
        };
        let w = chart_window(&s, today);
        assert!(matches!(
            &w,
            ChartWindow::Dates {
                compare: Some(_),
                ..
            }
        ));
        assert!(matches!(
            without_compare(w),
            ChartWindow::Dates { compare: None, .. }
        ));
    }

    #[test]
    fn flavor_for_entity_is_counter_for_energy_units() {
        assert_eq!(flavor_for_unit(Some("kWh")), Flavor::Counter);
        assert_eq!(flavor_for_unit(Some("Wh")), Flavor::Counter);
        assert_eq!(flavor_for_unit(Some("W")), Flavor::Series);
        assert_eq!(flavor_for_unit(Some("°C")), Flavor::Series);
        assert_eq!(flavor_for_unit(None), Flavor::Series);
    }

    fn sensor(id: &str, label: &str, class: &str, unit: &str) -> EntityView {
        use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
        EntityView {
            id: id.into(),
            device_id: Some("inverter".into()),
            name: label.into(),
            device_name: Some("Inverter".into()),
            label: Some(label.into()),
            kind: Kind::Sensor,
            component: Kind::Sensor.component().to_string(),
            protocol_kind: Kind::Sensor,
            device_class: Some(class.into()),
            unit: Some(unit.into()),
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: "garage".into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride::default(),
            state: EntityState::default(),
        }
    }

    #[test]
    fn day_curve_of_a_counter_is_its_paired_power_sensor() {
        let entities: BTreeMap<String, EntityView> = [
            sensor("inverter_energy_1", "Energy panel 1", "energy", "Wh"),
            sensor("inverter_power_1", "Power panel 1", "power", "W"),
            sensor("inverter_temp", "Temperature", "temperature", "°C"),
        ]
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect();
        assert_eq!(
            day_curve_of(&entities, "inverter_energy_1"),
            "inverter_power_1"
        );
        assert_eq!(
            day_curve_of(&entities, "inverter_power_1"),
            "inverter_power_1"
        );
        assert_eq!(day_curve_of(&entities, "inverter_temp"), "inverter_temp");
        assert_eq!(day_curve_of(&entities, "gone"), "gone");

        // A counter without a power sensor keeps its own curve.
        let mut alone = entities;
        alone.remove("inverter_power_1");
        assert_eq!(
            day_curve_of(&alone, "inverter_energy_1"),
            "inverter_energy_1"
        );
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn state(unit: Unit, at: Option<NaiveDate>, vs: Option<u8>) -> PeriodState {
        PeriodState {
            unit,
            at,
            custom: None,
            vs,
            show: Some("battery".into()),
        }
    }

    #[test]
    fn the_hour_views_are_presets() {
        let today = d(2026, 11, 26);
        assert_eq!(
            chart_window(&state(Unit::Hour1, None, Some(1)), today),
            ChartWindow::Preset(HistoryRange::Hour)
        );
        assert_eq!(
            chart_window(&state(Unit::Hour6, None, None), today),
            ChartWindow::Preset(HistoryRange::SixHours)
        );
    }

    #[test]
    fn a_month_is_a_date_window_with_its_comparison() {
        let today = d(2026, 11, 26);
        let req = |from, to| WindowReq {
            from,
            to,
            upto: None,
        };
        assert_eq!(
            chart_window(&state(Unit::Month, None, Some(1)), today),
            ChartWindow::Dates {
                shown: req(d(2026, 11, 1), d(2026, 11, 30)),
                unit: Unit::Month,
                compare: Some(req(d(2025, 11, 1), d(2025, 11, 30))),
            }
        );
        assert_eq!(
            chart_window(&state(Unit::Week, Some(d(2026, 10, 5)), None), today),
            ChartWindow::Dates {
                shown: req(d(2026, 10, 5), d(2026, 10, 11)),
                unit: Unit::Week,
                compare: None,
            }
        );
    }

    #[test]
    fn a_click_on_a_year_opens_the_month_and_on_a_month_the_day() {
        let today = d(2026, 11, 26);
        let year = state(Unit::Year, None, Some(2));
        let z = zoomed(&year, d(2026, 3, 1), today);
        assert_eq!(
            (z.unit, z.at, z.vs),
            (Unit::Month, Some(d(2026, 3, 1)), Some(2))
        );
        assert_eq!(z.show.as_deref(), Some("battery"));
        let z = zoomed(&year, d(2026, 11, 1), today);
        assert_eq!(
            (z.unit, z.at),
            (Unit::Month, None),
            "the running month is current"
        );
        let month = state(Unit::Month, None, None);
        let z = zoomed(&month, d(2026, 11, 12), today);
        assert_eq!((z.unit, z.at), (Unit::Day, Some(d(2026, 11, 12))));
        let z = zoomed(&month, today, today);
        assert_eq!((z.unit, z.at), (Unit::Day, None));
    }
}
