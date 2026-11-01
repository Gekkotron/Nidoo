//! The sensor card: the big value and its unit.

use leptos::prelude::*;
use leptos_router::components::A;
use nidoo_api::EntityView;
use nidoo_core::model::{EntityState, Kind, StateValue};

use super::Sender;
use super::text::{decimal, freshness};
use crate::components::card_frame::{CardFrame, open_entity};
use crate::components::now::{Now, unix_now};
use crate::components::sparkline::Sparkline;
use crate::format::{display_value, short_name, shown_device_name, shown_name, value_class};
use crate::house::House;
use crate::i18n::use_lang;
use crate::icons::DROP;
use crate::rooms::shows_on_home;
use crate::screens::history_detail::day_curve_of;

/// The text a sensor shows: the value with its unit, "—" while unknown or unavailable.
pub fn sensor_text(lang: crate::i18n::Lang, state: &EntityState, unit: Option<&str>) -> String {
    display_value(lang, state, unit)
}

/// The number a sensor reports, whether sent as a number or as numeric text.
fn numeric(state: &EntityState) -> Option<f64> {
    match &state.value {
        StateValue::Number(n) if n.is_finite() => Some(*n),
        StateValue::Text(t) => t.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
        _ => None,
    }
}

/// The comfort tint of a temperature: below 18 cold, 18 to 24 comfortable, above 24 warm.
fn comfort_class(celsius: f64) -> &'static str {
    if celsius < 18.0 {
        "readout__temp--cold"
    } else if celsius <= 24.0 {
        "readout__temp--ok"
    } else {
        "readout__temp--warm"
    }
}

/// The width of the humidity bar fill, clamped to 0..=100.
fn bar_percent(humidity: Option<f64>) -> u32 {
    humidity.map_or(0, |h| h.round().clamp(0.0, 100.0) as u32)
}

/// The humidity as "66 %", "—" when unknown.
fn humidity_text(humidity: Option<f64>) -> String {
    humidity.map_or_else(|| "—".into(), |h| format!("{} %", h.round()))
}

/// The percentage classes a temperature card shows under its reading, preferred first: the air
/// humidity of a thermometer, then the moisture of a soil sensor.
const COMPANION_CLASSES: [&str; 2] = ["humidity", "moisture"];

/// The sensor among `candidates` that the temperature card of `device_id` shows under its
/// reading: the device's sensor of the first class in [`COMPANION_CLASSES`] it has.
pub(crate) fn companion<'a>(
    candidates: impl IntoIterator<Item = &'a EntityView>,
    device_id: &str,
) -> Option<&'a EntityView> {
    candidates
        .into_iter()
        .filter(|e| e.kind == Kind::Sensor && e.device_id.as_deref() == Some(device_id))
        .filter_map(|e| {
            let class = e.device_class.as_deref()?;
            Some((COMPANION_CLASSES.iter().position(|c| *c == class)?, e))
        })
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, e)| e)
}

/// The companion sensor on `device_id` that Home shows, if any.
fn partner<'a>(
    entities: impl Iterator<Item = &'a EntityView>,
    device_id: Option<&str>,
) -> Option<&'a EntityView> {
    companion(entities.filter(|e| shows_on_home(e)), device_id?)
}

/// The humidity bar under a reading.
fn humidity_view(
    humidity: impl Fn() -> Option<f64> + Send + Sync + Copy + 'static,
) -> impl IntoView {
    view! {
        <div class="readout__humidity">
            <div class="readout__humidity-row">
                <span class="readout__drop" inner_html=DROP></span>
                <span>{move || humidity_text(humidity())}</span>
            </div>
            <div class="readout__bar">
                <div
                    class="readout__fill"
                    style:width=move || format!("{}%", bar_percent(humidity()))
                ></div>
            </div>
        </div>
    }
}

/// The href of the history chart for `id`.
fn history_href(id: &str) -> String {
    format!("/history/{}", crate::api::encode_id(id))
}

/// The body of a temperature card: the temperature large, then the humidity (or soil moisture) of
/// the same device when Home shows one, and the curve of the last 24 hours. The value and the
/// curve open the temperature's history, the humidity its own.
fn temperature_body(entity: &EntityView, house: House) -> impl IntoView + use<> {
    let lang = use_lang();
    let value = numeric(&entity.state);
    let class = match value {
        Some(v) => format!("temp__value {}", comfort_class(v)),
        None => "temp__value".into(),
    };
    let unit = entity
        .unit
        .clone()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| "°C".into());
    let number = move || value.map_or_else(|| "—".to_owned(), |v| decimal(lang(), v));
    let temp_href = history_href(&entity.id);
    let device_id = entity.device_id.clone();
    let device_id_val = device_id.clone();
    let humidity_id: Memo<Option<String>> = Memo::new(move |_| {
        house
            .entities
            .with(|m| partner(m.values(), device_id.as_deref()).map(|e| e.id.clone()))
    });
    let humidity_value: Memo<Option<f64>> = Memo::new(move |_| {
        house
            .entities
            .with(|m| partner(m.values(), device_id_val.as_deref()).and_then(|e| numeric(&e.state)))
    });
    let id = entity.id.clone();
    view! {
        <div class="temp">
            <A href=temp_href.clone() attr:class=class>
                {number}
                {value.map(|_| view! { " " <small class="temp__unit">{unit}</small> })}
            </A>
            <div class="temp__foot">
                {move || humidity_id.get().map(|id| {
                    let href = history_href(&id);
                    view! {
                        <A href=href attr:class="temp__humidity">
                            <span class="readout__drop" inner_html=DROP></span>
                            {move || humidity_text(humidity_value.get())}
                        </A>
                    }
                })}
                <A href=temp_href attr:class="temp__spark">
                    <Sparkline entity_id=id />
                </A>
            </div>
        </div>
    }
}

/// The body of a lone humidity card. The readout itself links to the history chart; the curve of
/// the last 24 hours sits under the bar.
fn humidity_body(entity: &EntityView) -> impl IntoView + use<> {
    let value = numeric(&entity.state);
    let href = history_href(&entity.id);
    let id = entity.id.clone();
    view! {
        <div class="readout">
            <A href=href attr:class="readout__link">
                <span class="readout__temp">{humidity_text(value)}</span>
                {humidity_view(move || value)}
                <Sparkline entity_id=id />
            </A>
        </div>
    }
}

/// The curve of a numeric sensor's last 24 hours, opening its history chart: an energy counter
/// draws its paired power sensor, like the chart. Nothing for a sensor that reports text: it has
/// no curve to draw.
fn value_spark(entity: &EntityView) -> Option<impl IntoView + use<>> {
    numeric(&entity.state)?;
    let href = history_href(&entity.id);
    let house = expect_context::<House>();
    let id = entity.id.clone();
    let curve = Memo::new(move |_| house.entities.with(|m| day_curve_of(m, &id)));
    Some(view! {
        <A href=href attr:class="value__spark">
            {move || view! { <Sparkline entity_id=curve.get() /> }}
        </A>
    })
}

#[component]
pub fn SensorCard(entity: EntityView) -> impl IntoView {
    let sender = Sender::from_context();
    let text = super::live_value(&entity);
    let class = move || match value_class(&text.get()) {
        "card__value" => "card__value card__value--big",
        long => long,
    };
    let pending = sender.pending(&entity.id);
    let on_click = open_entity(&entity.id);
    if entity.kind == Kind::Sensor {
        match entity.device_class.as_deref() {
            Some("temperature") => {
                let house = expect_context::<House>();
                let lang = use_lang();
                let paired = house
                    .entities
                    .with_untracked(|m| partner(m.values(), entity.device_id.as_deref()).is_some());
                let title = if paired {
                    short_name(
                        shown_name(lang(), &entity),
                        shown_device_name(lang(), &entity),
                    )
                } else {
                    String::new()
                };
                let body = temperature_body(&entity, house);
                let now = use_context::<Now>();
                let last_update = entity.state.last_update;
                let fresh = Signal::derive(move || {
                    let now = now.map_or_else(unix_now, |n| n.0.get());
                    freshness(lang(), now, last_update)
                });
                let trailing = view! {
                    {move || fresh.get().map(|t| view! { <span class="card__badge">{t}</span> })}
                }
                .into_any();
                return view! {
                    <CardFrame
                        entity=entity
                        on_click=on_click
                        pending=pending
                        title=title
                        tint="cool"
                        trailing=trailing
                    >
                        {body}
                    </CardFrame>
                }
                .into_any();
            }
            Some("humidity") => {
                let body = humidity_body(&entity);
                return view! {
                    <CardFrame entity=entity on_click=on_click pending=pending>{body}</CardFrame>
                }
                .into_any();
            }
            _ => {}
        }
    }
    let spark = value_spark(&entity);
    view! {
        <CardFrame entity=entity on_click=on_click pending=pending>
            <span class=class>{text}</span>
            {spark}
        </CardFrame>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::{
        bar_percent, comfort_class, companion, humidity_text, numeric, partner, sensor_text,
    };
    use nidoo_api::EntityView;
    use nidoo_core::model::{
        Capabilities, EntityCategory, EntityState, Kind, StateValue, UiOverride,
    };

    fn sensor(id: &str, device: Option<&str>, class: &str, value: f64) -> EntityView {
        EntityView {
            id: id.into(),
            device_id: device.map(Into::into),
            name: id.into(),
            device_name: None,
            label: None,
            kind: Kind::Sensor,
            component: "sensor".to_string(),
            protocol_kind: Kind::Sensor,
            device_class: Some(class.into()),
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: "living".into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride::default(),
            state: EntityState {
                value: StateValue::Number(value),
                ..Default::default()
            },
        }
    }

    #[test]
    fn comfort_bands() {
        assert_eq!(comfort_class(17.9), "readout__temp--cold");
        assert_eq!(comfort_class(18.0), "readout__temp--ok");
        assert_eq!(comfort_class(24.0), "readout__temp--ok");
        assert_eq!(comfort_class(24.1), "readout__temp--warm");
    }

    #[test]
    fn bar_is_clamped_and_empty_when_unknown() {
        assert_eq!(bar_percent(None), 0);
        assert_eq!(bar_percent(Some(-5.0)), 0);
        assert_eq!(bar_percent(Some(66.4)), 66);
        assert_eq!(bar_percent(Some(140.0)), 100);
    }

    #[test]
    fn humidity_text_rounds() {
        assert_eq!(humidity_text(Some(65.6)), "66 %");
        assert_eq!(humidity_text(None), "—");
    }

    #[test]
    fn numeric_reads_numbers_and_numeric_text() {
        let text = |t: &str| EntityState {
            value: StateValue::Text(t.into()),
            ..Default::default()
        };
        assert_eq!(numeric(&text("21.5")), Some(21.5));
        assert_eq!(numeric(&text("unknown")), None);
        assert_eq!(numeric(&EntityState::default()), None);
    }

    #[test]
    fn partner_matches_class_and_device() {
        let all = [
            sensor("t", Some("dev"), "temperature", 20.0),
            sensor("h_other", Some("other"), "humidity", 50.0),
            sensor("h", Some("dev"), "humidity", 60.0),
        ];
        let found = partner(all.iter(), Some("dev"));
        assert_eq!(found.map(|e| e.id.as_str()), Some("h"));
        assert!(partner(all.iter(), None).is_none());
        assert!(partner(all.iter(), Some("none")).is_none());
    }

    #[test]
    fn partner_must_be_shown_on_home() {
        let mut hidden = sensor("h", Some("dev"), "humidity", 60.0);
        hidden.ui.hidden = true;
        assert!(partner([hidden].iter(), Some("dev")).is_none());
    }

    #[test]
    fn soil_moisture_is_a_companion() {
        let all = [
            sensor("t", Some("soil"), "temperature", 14.0),
            sensor("m", Some("soil"), "moisture", 100.0),
        ];
        assert_eq!(companion(&all, "soil").map(|e| e.id.as_str()), Some("m"));
    }

    #[test]
    fn humidity_is_preferred_over_moisture() {
        let all = [
            sensor("m", Some("dev"), "moisture", 30.0),
            sensor("h", Some("dev"), "humidity", 60.0),
        ];
        assert_eq!(companion(&all, "dev").map(|e| e.id.as_str()), Some("h"));
    }

    #[test]
    fn unknown_sensor_text() {
        let s = EntityState {
            value: StateValue::Text("unknown".into()),
            ..Default::default()
        };
        assert_eq!(sensor_text(crate::i18n::Lang::En, &s, Some("W")), "—");
    }
    #[test]
    fn numeric_sensor_text() {
        let s = EntityState {
            value: StateValue::Number(42.0),
            ..Default::default()
        };
        assert_eq!(
            sensor_text(crate::i18n::Lang::En, &s, Some("W")),
            "42\u{a0}W"
        );
    }
}
