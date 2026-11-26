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
    /// "Shutter or blind": covers with no `device_class`, which a class would otherwise decide.
    pub shutter_mark: bool,
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
        shutter_mark: entity.protocol_kind == Kind::Cover && class.is_none(),
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

/// Turns the shutter mark on or off in the draft: `showAs` is `shutter` or unset.
pub fn set_shutter(draft: &mut UiOverride, on: bool) {
    draft.show_as = on.then(|| nidoo_core::model::SHUTTER_MARK.to_owned());
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
            shown.show_as || shown.shutter_mark,
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
mod tests;
