use super::*;
use crate::override_editor::dirty_patch;
use nidoo_api::EntityView;
use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, SwitchType, UiOverride};

fn entity(kind: Kind, class: Option<&str>, unit: Option<&str>) -> EntityView {
    EntityView {
        id: "e".into(),
        device_id: None,
        name: "e".into(),
        device_name: None,
        label: None,
        kind,
        component: kind.component().to_string(),
        protocol_kind: kind,
        device_class: class.map(Into::into),
        unit: unit.map(Into::into),
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "living".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState::default(),
    }
}

fn colour_light() -> EntityView {
    let mut e = entity(Kind::Light, None, None);
    e.capabilities.color_modes.insert("xy".into());
    e.capabilities.brightness = true;
    e
}

fn with_role(role: &str) -> UiOverride {
    UiOverride {
        energy_role: Some(role.into()),
        ..UiOverride::default()
    }
}

#[test]
fn main_control_is_shown_for_generic_with_controls() {
    let mut lock = entity(Kind::Generic, Some("lock"), None);
    lock.capabilities.generic = Some(nidoo_core::types::GenericCaps {
        controls: vec!["locked".into(), "open".into()],
        ..Default::default()
    });
    assert!(shown(&lock, &UiOverride::default()).main_control);
    let bare = entity(Kind::Generic, None, None);
    assert!(!shown(&bare, &UiOverride::default()).main_control);
}

#[test]
fn main_control_is_hidden_for_lights() {
    assert!(!shown(&colour_light(), &UiOverride::default()).main_control);
}

#[test]
fn light_has_only_light_settings() {
    let s = shown(&colour_light(), &UiOverride::default());
    assert!(s.light_on && !s.group_with_room);
    assert!(!s.show_as);
    assert!(!s.security_shown && !s.security_action && !s.security_ask);
    assert!(!s.card_style && !s.energy_role && !s.energy_meter && !s.climate_presets);
    assert!(!s.light_on_brightness, "no preset chosen yet");
    let preset = UiOverride {
        light_on_preset: Some("warm".into()),
        ..UiOverride::default()
    };
    assert!(shown(&colour_light(), &preset).light_on_brightness);
}

#[test]
fn plain_light_has_no_light_on_chips() {
    assert!(!shown(&entity(Kind::Light, None, None), &UiOverride::default()).light_on);
}

#[test]
fn shutter_mark_is_for_unclassed_covers_only() {
    assert!(shown(&entity(Kind::Cover, None, None), &UiOverride::default()).shutter_mark);
    assert!(
        !shown(
            &entity(Kind::Cover, Some("garage"), None),
            &UiOverride::default()
        )
        .shutter_mark
    );
    assert!(
        !shown(
            &entity(Kind::Cover, Some("shutter"), None),
            &UiOverride::default()
        )
        .shutter_mark
    );
    assert!(!shown(&entity(Kind::Switch, None, None), &UiOverride::default()).shutter_mark);
    assert!(!shown(&entity(Kind::Light, None, None), &UiOverride::default()).shutter_mark);
}

#[test]
fn shutter_mark_on_an_unclassed_cover_is_not_an_orphan_value() {
    let marked = UiOverride {
        show_as: Some("shutter".into()),
        ..UiOverride::default()
    };
    let cover = entity(Kind::Cover, None, None);
    assert!(other_saved(&marked, &shown(&cover, &marked)).is_empty());
    let classed = entity(Kind::Cover, Some("garage"), None);
    assert_eq!(
        other_saved(&marked, &shown(&classed, &marked)),
        vec![("showAs", "shutter".to_string())]
    );
}

#[test]
fn shutter_mark_does_not_change_the_switch_type() {
    let marked = UiOverride {
        show_as: Some("shutter".into()),
        ..UiOverride::default()
    };
    let switch = entity(Kind::Switch, None, None);
    assert_eq!(
        SwitchType::of(Kind::Switch, &marked),
        Some(SwitchType::Switch)
    );
    assert!(shown(&switch, &marked).show_as);
}

#[test]
fn switch_gets_type_and_security() {
    let switch = entity(Kind::Switch, None, None);
    let s = shown(&switch, &UiOverride::default());
    assert!(s.show_as && s.security_action);
    assert!(!s.security_ask);
    let action = UiOverride {
        security_action: true,
        ..UiOverride::default()
    };
    assert!(shown(&switch, &action).security_ask);
}

#[test]
fn door_and_siren_always_offer_ask() {
    let switch = entity(Kind::Switch, None, None);
    for (t, ask) in [
        (SwitchType::Door, true),
        (SwitchType::Siren, true),
        (SwitchType::Chime, false),
    ] {
        let mut d = UiOverride::default();
        set_type(&mut d, t);
        assert_eq!(shown(&switch, &d).security_ask, ask, "{t:?}");
    }
}

#[test]
fn light_type_follows_the_draft() {
    let switch = entity(Kind::Switch, None, None);
    let mut d = UiOverride {
        security_action: true,
        ..UiOverride::default()
    };
    set_type(&mut d, SwitchType::Light);
    assert_eq!(d.show_as.as_deref(), Some("light"));
    assert!(!d.security_action);
    let s = shown(&switch, &d);
    assert!(!s.group_with_room && s.show_as);
    assert!(!s.security_action && !s.security_ask);
}

#[test]
fn invert_is_patched_and_cleared() {
    let stored = UiOverride::default();
    let d = UiOverride {
        invert: true,
        ..UiOverride::default()
    };
    assert_eq!(
        dirty_patch(&stored, &d),
        serde_json::json!({"invert": true})
            .as_object()
            .cloned()
            .unwrap()
    );
    // Saved on an entity that is no longer a switch: listed so it can be cleared.
    let light = entity(Kind::Light, None, None);
    assert_eq!(
        other_saved(&d, &shown(&light, &d)),
        vec![("invert", "true".to_owned())]
    );
    let mut cleared = d.clone();
    clear_field(&mut cleared, "invert");
    assert!(!cleared.invert);
}

#[test]
fn type_round_trip_is_clean() {
    let stored = UiOverride::default();
    let mut d = stored.clone();
    set_type(&mut d, SwitchType::Door);
    set_type(&mut d, SwitchType::Switch);
    assert!(dirty_patch(&stored, &d).is_empty());

    let stored = UiOverride {
        security_action: true,
        ..UiOverride::default()
    };
    let mut d = stored.clone();
    set_type(&mut d, SwitchType::Door);
    let patch = dirty_patch(&stored, &d);
    assert_eq!(
        patch,
        serde_json::json!({"showAs": "door"})
            .as_object()
            .cloned()
            .unwrap()
    );
}

#[test]
fn power_sensor_gets_energy() {
    let watts = entity(Kind::Sensor, Some("power"), Some("W"));
    let s = shown(&watts, &with_role("grid"));
    assert!(s.card_style && s.energy_role && s.energy_invert && s.energy_meter);
    assert!(!s.energy_icon && !s.unit_warning);
    let kwh = entity(Kind::Sensor, Some("energy"), Some("kWh"));
    assert!(shown(&kwh, &with_role("grid")).unit_warning);
    let consumer = shown(&watts, &with_role("consumer"));
    assert!(consumer.energy_icon && !consumer.energy_invert);
    assert!(!shown(&watts, &with_role("water")).energy_meter);
    let none = shown(&watts, &UiOverride::default());
    assert!(none.energy_role && !none.energy_invert && !none.energy_meter && !none.energy_icon);
}

#[test]
fn door_sensor_gets_security_shown() {
    let s = shown(
        &entity(Kind::BinarySensor, Some("door"), None),
        &UiOverride::default(),
    );
    assert!(s.security_shown && !s.security_action);
}

#[test]
fn elsewhere_switches_hidden_for_show_on_home() {
    let watts = entity(Kind::Sensor, Some("power"), Some("W"));
    assert!(shown(&watts, &with_role("grid")).show_on_home);
    assert!(!shown(&watts, &UiOverride::default()).show_on_home);
}

#[test]
fn climate_gets_presets() {
    let s = shown(&entity(Kind::Climate, None, None), &UiOverride::default());
    assert!(s.climate_presets && !s.energy_role && !s.light_on);
}

#[test]
fn group_with_room_is_never_shown() {
    assert!(!shown(&colour_light(), &UiOverride::default()).group_with_room);
    assert!(!shown(&entity(Kind::Cover, None, None), &UiOverride::default()).group_with_room);
}

#[test]
fn other_saved_lists_hidden_values() {
    let light = colour_light();
    let mut d = UiOverride {
        energy_role: Some("grid".into()),
        security_ask: false,
        favorite: true,
        ..UiOverride::default()
    };
    let s = shown(&light, &d);
    assert_eq!(
        other_saved(&d, &s),
        vec![
            ("energyRole", "grid".to_owned()),
            ("securityAsk", "false".to_owned())
        ]
    );
    clear_field(&mut d, "energyRole");
    assert_eq!(d.energy_role, None);
    clear_field(&mut d, "securityAsk");
    assert!(d.security_ask);
    assert!(other_saved(&d, &shown(&light, &d)).is_empty());
}
