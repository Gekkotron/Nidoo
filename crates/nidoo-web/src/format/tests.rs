use super::{
    date_only, display_value, entity_title, entity_value, list_label, shown_device_name,
    shown_name, wrap_id,
};
use crate::i18n::Lang;
use nidoo_api::EntityView;
use nidoo_core::model::{
    Capabilities, ClimateState, EntityCategory, EntityState, Kind, LightState, StateValue,
    UiOverride,
};

fn entity(name: &str, device: &str, kind: Kind, class: Option<&str>) -> EntityView {
    EntityView {
        id: "x".into(),
        device_id: None,
        name: name.into(),
        device_name: Some(device.into()),
        label: None,
        kind,
        component: kind.component().to_string(),
        protocol_kind: kind,
        device_class: class.map(Into::into),
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "salon".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState::default(),
    }
}

#[test]
fn weather_entities_read_in_the_language_on_screen() {
    let mut cloud = entity("Cloud cover", "Weather", Kind::Sensor, None);
    cloud.id = "weather_cloud_cover".into();
    assert_eq!(shown_name(Lang::Fr, &cloud), "Couverture nuageuse");
    assert_eq!(shown_name(Lang::En, &cloud), "Cloud cover");
    assert_eq!(shown_device_name(Lang::Fr, &cloud), Some("Météo"));
    assert_eq!(entity_title(Lang::Fr, &cloud), "Couverture nuageuse");
    // A name the user gave stays, in both languages.
    cloud.name = "Nuages".into();
    assert_eq!(shown_name(Lang::En, &cloud), "Nuages");
    // Only the hub's weather entities are translated.
    let mut other = entity("Cloud cover", "Weather", Kind::Sensor, None);
    other.id = "station_cloud_cover".into();
    assert_eq!(shown_name(Lang::Fr, &other), "Cloud cover");
    assert_eq!(shown_device_name(Lang::Fr, &other), Some("Weather"));
}

#[test]
fn a_switch_shown_as_a_light_reads_allume() {
    let mut lamp = entity("Lampe", "Lampe", Kind::Switch, None);
    lamp.ui.show_as = Some("light".into());
    lamp.state.value = StateValue::OnOff(true);
    assert_eq!(entity_value(Lang::Fr, &lamp), "Allumé");
    lamp.state.value = StateValue::OnOff(false);
    assert_eq!(entity_value(Lang::Fr, &lamp), "Éteint");
    assert_eq!(entity_value(Lang::En, &lamp), "Off");
    lamp.ui.show_as = None;
    lamp.state.value = StateValue::OnOff(true);
    assert_eq!(entity_value(Lang::Fr, &lamp), "Activé");
}

#[test]
fn unnamed_relay_titles_follow_the_type() {
    let mut gate = entity("gate", "gate", Kind::Switch, Some("outlet"));
    gate.ui.show_as = Some("door".into());
    assert_eq!(entity_title(Lang::En, &gate), "gate · Door opener");
    assert_eq!(list_label(Lang::En, &gate), "gate · Door opener");
    let mut named = entity("Gate relay", "gate", Kind::Switch, None);
    named.ui.show_as = Some("siren".into());
    assert_eq!(list_label(Lang::En, &named), "Relay · Siren");
    let plain = entity("gate", "gate", Kind::Switch, Some("outlet"));
    assert_eq!(entity_title(Lang::En, &plain), "gate · Outlet");
}

#[test]
fn type_word_names_the_switch_type() {
    let mut door = entity("gate", "gate", Kind::Switch, None);
    door.ui.show_as = Some("door".into());
    assert_eq!(super::type_word(Lang::En, &door), "Door opener");
    assert_eq!(
        super::type_word(Lang::En, &entity("s", "s", Kind::Switch, None)),
        "Switch"
    );
    assert_eq!(
        super::type_word(Lang::En, &entity("l", "l", Kind::Light, None)),
        "Light"
    );
}

#[test]
fn ids_break_after_underscores_and_slashes() {
    assert_eq!(wrap_id("door_siren/x"), "door_\u{200B}siren/\u{200B}x");
    assert_eq!(wrap_id("plain"), "plain");
}

#[test]
fn same_named_entity_gets_its_class_or_kind() {
    let siren = entity(
        "door_siren",
        "door_siren",
        Kind::BinarySensor,
        Some("tamper"),
    );
    assert_eq!(entity_title(Lang::En, &siren), "door_siren · Tamper");
    assert_eq!(list_label(Lang::En, &siren), "door_siren · Tamper");
    let plain = entity("door_siren", "Door_Siren", Kind::Switch, None);
    assert_eq!(entity_title(Lang::En, &plain), "door_siren · Switch");
}

#[test]
fn suffix_that_repeats_the_name_is_dropped() {
    let heater = entity("Water heater", "Water heater", Kind::WaterHeater, None);
    assert_eq!(entity_title(Lang::En, &heater), "Water heater");
    assert_eq!(list_label(Lang::En, &heater), "Water heater");
}

#[test]
fn distinct_names_are_untouched() {
    let e = entity(
        "powers/ac energy",
        "powers/ac",
        Kind::Sensor,
        Some("energy"),
    );
    assert_eq!(entity_title(Lang::En, &e), "Energy");
    assert_eq!(list_label(Lang::En, &e), "Energy · Sensor");
}

#[test]
fn unknown_text_renders_as_dash() {
    let s = EntityState {
        value: StateValue::Text("unknown".into()),
        ..Default::default()
    };
    assert_eq!(display_value(Lang::En, &s, None), "—");
}
#[test]
fn unavailable_text_renders_as_dash() {
    let s = EntityState {
        value: StateValue::Text("UnAvAiLaBlE".into()),
        ..Default::default()
    };
    assert_eq!(display_value(Lang::En, &s, None), "—");
}
#[test]
fn integer_number_omits_decimal() {
    let s = EntityState {
        value: StateValue::Number(21.0),
        ..Default::default()
    };
    assert_eq!(display_value(Lang::En, &s, Some("°C")), "21\u{a0}°C");
}
#[test]
fn fractional_number_keeps_one_decimal() {
    let s = EntityState {
        value: StateValue::Number(21.47),
        ..Default::default()
    };
    assert_eq!(display_value(Lang::En, &s, Some("°C")), "21.5\u{a0}°C");
}
#[test]
fn on_off_renders_on_and_off() {
    let s_on = EntityState {
        value: StateValue::OnOff(true),
        ..Default::default()
    };
    let s_off = EntityState {
        value: StateValue::OnOff(false),
        ..Default::default()
    };
    assert_eq!(display_value(Lang::En, &s_on, None), "On");
    assert_eq!(display_value(Lang::En, &s_off, None), "Off");
}
#[test]
fn unknown_variant_renders_as_dash() {
    let s = EntityState {
        value: StateValue::Unknown,
        ..Default::default()
    };
    assert_eq!(display_value(Lang::En, &s, None), "—");
}
#[test]
fn is_on_covers_switches_and_lights() {
    let on = EntityState {
        value: StateValue::OnOff(true),
        ..Default::default()
    };
    let l = LightState {
        on: false,
        ..Default::default()
    };
    let off = EntityState {
        value: StateValue::Light(l),
        ..Default::default()
    };
    assert!(super::is_on(&on));
    assert!(!super::is_on(&off));
}
#[test]
fn percent_unit_is_spaced() {
    let s = EntityState {
        value: StateValue::Number(80.0),
        ..Default::default()
    };
    assert_eq!(display_value(Lang::En, &s, Some("%")), "80\u{a0}%");
}
#[test]
fn light_shows_brightness_when_on() {
    let l = LightState {
        on: true,
        brightness: Some(128),
        ..Default::default()
    };
    let s = EntityState {
        value: StateValue::Light(l),
        ..Default::default()
    };
    assert_eq!(display_value(Lang::En, &s, None), "On · 50\u{a0}%");
}
#[test]
fn alarm_state_is_capitalised_without_underscores() {
    let s = EntityState {
        value: StateValue::Alarm("armed_away".into()),
        ..Default::default()
    };
    assert_eq!(display_value(Lang::En, &s, None), "Armed away");
}
#[test]
fn climate_defaults_to_celsius_when_unit_unknown() {
    let c = ClimateState {
        current: Some(22.0),
        target: Some(15.0),
        ..Default::default()
    };
    let s = EntityState {
        value: StateValue::Climate(c),
        ..Default::default()
    };
    assert_eq!(display_value(Lang::En, &s, None), "22 → 15\u{a0}°C");
}
#[test]
fn climate_shows_current_and_target() {
    let c = ClimateState {
        current: Some(20.5),
        target: Some(21.0),
        ..Default::default()
    };
    let s = EntityState {
        value: StateValue::Climate(c),
        ..Default::default()
    };
    assert_eq!(display_value(Lang::En, &s, Some("°C")), "20.5 → 21\u{a0}°C");
}

#[test]
fn thousands_have_no_decimals_and_a_thin_space() {
    let s = EntityState {
        value: StateValue::Number(2199.34),
        ..Default::default()
    };
    assert_eq!(
        display_value(Lang::En, &s, Some("kWh")),
        "2\u{2009}199\u{a0}kWh"
    );
    let big = EntityState {
        value: StateValue::Number(-1234567.0),
        ..Default::default()
    };
    assert_eq!(
        display_value(Lang::En, &big, None),
        "-1\u{2009}234\u{2009}567"
    );
    let under = EntityState {
        value: StateValue::Number(999.5),
        ..Default::default()
    };
    assert_eq!(display_value(Lang::En, &under, None), "999.5");
}

#[test]
fn date_only_keeps_the_date_as_sent() {
    assert_eq!(
        date_only("2026-01-15T22:30:00+02:00").as_deref(),
        Some("2026-01-15")
    );
    assert_eq!(date_only("last week"), None);
}

#[test]
fn rfc3339_text_renders_as_date_and_time() {
    let s = EntityState {
        value: StateValue::Text("2026-10-19T09:41:12+02:00".into()),
        ..Default::default()
    };
    assert_eq!(display_value(Lang::En, &s, None), "2026-10-19 09:41");
    let plain = EntityState {
        value: StateValue::Text("2026-10-19 is a date".into()),
        ..Default::default()
    };
    assert_eq!(
        display_value(Lang::En, &plain, None),
        "2026-10-19 is a date"
    );
}

#[test]
fn degrees_defaults_to_celsius() {
    assert_eq!(super::degrees(Lang::En, Some(21.0), None), "21\u{a0}°C");
    assert_eq!(super::degrees(Lang::En, None, None), "—");
}

#[test]
fn long_values_get_the_long_class() {
    assert_eq!(super::value_class("42 W"), "card__value");
    assert_eq!(
        super::value_class("2026-10-19 09:41"),
        "card__value card__value--long"
    );
}

#[test]
fn short_name_drops_the_device_prefix() {
    use super::short_name;
    assert_eq!(short_name("powers/ac energy", Some("powers/ac")), "Energy");
    assert_eq!(short_name("Lumière table", Some("lumière")), "Table");
    assert_eq!(short_name("Salonette", Some("Salon")), "Salonette");
    assert_eq!(short_name("Salon", Some("Salon")), "Salon");
    assert_eq!(short_name("Lamp", None), "Lamp");
    assert_eq!(short_name("Lamp", Some("Other device")), "Lamp");
}
