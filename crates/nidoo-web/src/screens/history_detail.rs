//! The full-screen `/history/:id?range=…` page: chrome row, range chips, and a `HistoryChart`.

use std::collections::BTreeMap;

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::{use_navigate, use_params_map, use_query_map};
use nidoo_api::{EntityView, HistoryRange};
use nidoo_core::energy::{EnergyMeters, EnergyUnits};

use crate::components::history_chart::{Flavor, HistoryChart};
use crate::energy::{energy_entities, energy_entity};
use crate::format::{area_name, shown_name, title_suffix, type_word, wrap_id};
use crate::house::House;
use crate::i18n::devices::availability_text;
use crate::i18n::history::{HistoryText, history_text, range_chip};
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

/// Parse the `range` query parameter (`1h`, `6h`, `24h`, `7d`, `30d`, `1y`); default `24h`.
pub fn parse_range(raw: Option<String>) -> HistoryRange {
    raw.as_deref()
        .and_then(HistoryRange::parse)
        .unwrap_or(HistoryRange::Day)
}

fn range_slug(r: HistoryRange) -> &'static str {
    match r {
        HistoryRange::Hour => "1h",
        HistoryRange::SixHours => "6h",
        HistoryRange::Day => "24h",
        HistoryRange::Week => "7d",
        HistoryRange::Month => "30d",
        HistoryRange::Year => "1y",
    }
}

const RANGES: [HistoryRange; 6] = [
    HistoryRange::Hour,
    HistoryRange::SixHours,
    HistoryRange::Day,
    HistoryRange::Week,
    HistoryRange::Month,
    HistoryRange::Year,
];

#[component]
pub fn HistoryDetailScreen() -> impl IntoView {
    let house = expect_context::<House>();
    let params = use_params_map();
    let query = use_query_map();
    let navigate = use_navigate();
    let lang = use_lang();

    let id = Memo::new(move |_| params.read().get("id").unwrap_or_default());
    let range = Memo::new(move |_| parse_range(query.read().get("range")));

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

    // Range chips update the `range` query param via replace-navigation so Back works.
    let id_for_nav = id;
    let navigate = navigate.clone();
    let on_range = {
        let navigate = navigate.clone();
        move |r: HistoryRange| {
            let path = format!(
                "/history/{}?range={}",
                crate::api::encode_id(&id_for_nav.get()),
                range_slug(r)
            );
            navigate(
                &path,
                NavigateOptions {
                    replace: true,
                    ..Default::default()
                },
            );
        }
    };

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
                    let on_range = on_range.clone();
                    view! {
                        <section class="sheet__panel">
                            <div class="chips history-detail__chips">
                                {RANGES.iter().map(|r| {
                                    let r = *r;
                                    let on_range = on_range.clone();
                                    view! {
                                        <button
                                            type="button"
                                            class="chip"
                                            class:chip--selected=move || range.get() == r
                                            aria-pressed=move || (range.get() == r).to_string()
                                            on:click=move |_| on_range(r)
                                        >
                                            {move || range_chip(lang(), r)}
                                        </button>
                                    }
                                }).collect_view()}
                            </div>
                            <HistoryChart
                                entity_id=Signal::derive(move || id.get())
                                range=Signal::derive(move || range.get())
                                flavor=Signal::derive(move || flavor_sig.get())
                                kind=Signal::derive(move || kind_str.get())
                                unit=Signal::derive(move || unit_opt.get())
                                power_pair=Signal::derive(move || power_pair.get())
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

    #[test]
    fn range_parses_wire_names_and_defaults_to_day() {
        assert_eq!(parse_range(Some("1h".into())), HistoryRange::Hour);
        assert_eq!(parse_range(Some("6h".into())), HistoryRange::SixHours);
        assert_eq!(parse_range(Some("24h".into())), HistoryRange::Day);
        assert_eq!(parse_range(Some("7d".into())), HistoryRange::Week);
        assert_eq!(parse_range(Some("30d".into())), HistoryRange::Month);
        assert_eq!(parse_range(Some("1y".into())), HistoryRange::Year);
        assert_eq!(parse_range(Some("junk".into())), HistoryRange::Day);
        assert_eq!(parse_range(None), HistoryRange::Day);
    }

    #[test]
    fn range_slugs_match_history_range_wire_names() {
        assert_eq!(range_slug(HistoryRange::Hour), "1h");
        assert_eq!(range_slug(HistoryRange::SixHours), "6h");
        assert_eq!(range_slug(HistoryRange::Day), "24h");
        assert_eq!(range_slug(HistoryRange::Week), "7d");
        assert_eq!(range_slug(HistoryRange::Month), "30d");
        assert_eq!(range_slug(HistoryRange::Year), "1y");
    }
}
