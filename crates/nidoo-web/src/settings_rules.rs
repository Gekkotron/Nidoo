//! Which settings the device sheet shows for an entity, following its draft override: the app's
//! Customize sheet rules plus the switch type selector.

use nidoo_api::EntityView;
use nidoo_core::energy::{EnergyRole, PowerUnits};
use nidoo_core::model::{Kind, Placement, SwitchType, UiOverride};

use crate::cards::light::has_colour;

/// The controls the settings form shows. Every rule reads the draft, so picking a type or an
/// energy role reshapes the form before it is saved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shown {
    /// The "Show as" chips: switches only.
    pub show_as: bool,
    /// "Show on Home" instead of "Hide from Home": Energy or Security already shows it.
    pub show_on_home: bool,
    pub group_with_room: bool,
    pub light_on: bool,
    pub light_on_brightness: bool,
    /// "Show on Security" for kinds Security lists by default (the inverse of `securityHidden`).
    pub security_shown: bool,
    /// "Show on Security" for kinds the user opts in as actions (`securityAction`).
    pub security_action: bool,
    pub security_ask: bool,
    pub card_style: bool,
    pub energy_role: bool,
    pub energy_invert: bool,
    pub energy_icon: bool,
    pub energy_meter: bool,
    pub unit_warning: bool,
    pub climate_presets: bool,
    /// The "Main control" chips: generic entities that announce at least one control.
    pub main_control: bool,
}

pub fn shown(entity: &EntityView, draft: &UiOverride) -> Shown {
    let kind = SwitchType::effective_kind(entity.protocol_kind, draft);
    let class = entity.device_class.as_deref();
    let switch_type = SwitchType::of(entity.protocol_kind, draft);
    let security_action = Placement::action_kind(kind);
    let light_on = kind == Kind::Light && has_colour(&entity.capabilities.color_modes);
    let energy_role = kind == Kind::Sensor;
    let role = draft
        .energy_role
        .as_deref()
        .and_then(EnergyRole::parse)
        .filter(|_| energy_role);
    let energy_meter = role.is_some_and(EnergyRole::power);
    Shown {
        show_as: switch_type.is_some(),
        show_on_home: Placement::elsewhere(kind, class, draft),
        group_with_room: false,
        light_on,
        light_on_brightness: light_on
            && draft.light_on_preset.is_some()
            && entity.capabilities.brightness,
        security_shown: Placement::security_kind(kind, class),
        security_action,
        security_ask: security_action
            && (draft.security_action
                || matches!(switch_type, Some(SwitchType::Door | SwitchType::Siren))),
        card_style: kind == Kind::Sensor,
        energy_role,
        energy_invert: role.is_some_and(EnergyRole::signed),
        energy_icon: role == Some(EnergyRole::Consumer),
        energy_meter,
        unit_warning: energy_meter && PowerUnits::factor(entity.unit.as_deref()).is_none(),
        climate_presets: kind == Kind::Climate,
        main_control: entity.kind == Kind::Generic
            && entity
                .capabilities
                .generic
                .as_ref()
                .is_some_and(|g| !g.controls.is_empty()),
    }
}

/// Picks a type in the draft. A light cannot be a Security action, so Light also drops it.
pub fn set_type(draft: &mut UiOverride, t: SwitchType) {
    draft.show_as = t.key().map(str::to_owned);
    if t == SwitchType::Light {
        draft.security_action = false;
    }
}

/// Values the draft holds for controls the form does not show (written by MCP or before a type
/// change), in [`UiOverride::FIELDS`] order, as (camelCase key, value as text).
pub fn other_saved(draft: &UiOverride, shown: &Shown) -> Vec<(&'static str, String)> {
    let default = UiOverride::default();
    let text = |o: &Option<String>| o.clone().unwrap_or_default();
    let candidates: [(&'static str, bool, bool, String); 15] = [
        (
            "cardStyle",
            shown.card_style,
            draft.card_style != default.card_style,
            text(&draft.card_style),
        ),
        (
            "energyRole",
            shown.energy_role,
            draft.energy_role != default.energy_role,
            text(&draft.energy_role),
        ),
        (
            "energyInvert",
            shown.energy_invert,
            draft.energy_invert != default.energy_invert,
            draft.energy_invert.to_string(),
        ),
        (
            "securityHidden",
            shown.security_shown,
            draft.security_hidden != default.security_hidden,
            draft.security_hidden.to_string(),
        ),
        (
            "energyIcon",
            shown.energy_icon,
            draft.energy_icon != default.energy_icon,
            text(&draft.energy_icon),
        ),
        (
            "energyMeter",
            shown.energy_meter,
            draft.energy_meter != default.energy_meter,
            text(&draft.energy_meter),
        ),
        (
            "securityAction",
            shown.security_action,
            draft.security_action != default.security_action,
            draft.security_action.to_string(),
        ),
        (
            "securityAsk",
            shown.security_ask,
            draft.security_ask != default.security_ask,
            draft.security_ask.to_string(),
        ),
        (
            "ungrouped",
            shown.group_with_room,
            draft.ungrouped != default.ungrouped,
            draft.ungrouped.to_string(),
        ),
        (
            "showAs",
            shown.show_as,
            draft.show_as != default.show_as,
            text(&draft.show_as),
        ),
        (
            "lightOnPreset",
            shown.light_on,
            draft.light_on_preset != default.light_on_preset,
            text(&draft.light_on_preset),
        ),
        (
            "lightOnBrightness",
            shown.light_on_brightness,
            draft.light_on_brightness != default.light_on_brightness,
            draft
                .light_on_brightness
                .map(|b| b.to_string())
                .unwrap_or_default(),
        ),
        (
            "climatePresets",
            shown.climate_presets,
            draft.climate_presets != default.climate_presets,
            text(&draft.climate_presets),
        ),
        (
            "mainControl",
            shown.main_control,
            draft.main_control != default.main_control,
            text(&draft.main_control),
        ),
        (
            "invert",
            shown.show_as,
            draft.invert != default.invert,
            draft.invert.to_string(),
        ),
    ];
    candidates
        .into_iter()
        .filter(|(_, visible, set, _)| !visible && *set)
        .map(|(key, _, _, value)| (key, value))
        .collect()
}

/// Resets one field, by its camelCase key, to its default; unknown keys are ignored.
pub fn clear_field(draft: &mut UiOverride, key: &str) {
    let default = UiOverride::default();
    match key {
        "cardStyle" => draft.card_style = None,
        "energyRole" => draft.energy_role = None,
        "energyInvert" => draft.energy_invert = default.energy_invert,
        "securityHidden" => draft.security_hidden = default.security_hidden,
        "energyIcon" => draft.energy_icon = None,
        "energyMeter" => draft.energy_meter = None,
        "securityAction" => draft.security_action = default.security_action,
        "securityAsk" => draft.security_ask = default.security_ask,
        "ungrouped" => draft.ungrouped = default.ungrouped,
        "showAs" => draft.show_as = None,
        "lightOnPreset" => draft.light_on_preset = None,
        "lightOnBrightness" => draft.light_on_brightness = None,
        "climatePresets" => draft.climate_presets = None,
        "mainControl" => draft.main_control = None,
        "invert" => draft.invert = default.invert,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::override_editor::dirty_patch;
    use nidoo_api::EntityView;
    use nidoo_core::model::{
        Capabilities, EntityCategory, EntityState, Kind, SwitchType, UiOverride,
    };

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
}
