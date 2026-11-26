//! Pure presentation of the Security tab: entities and areas → panels, actions, sensor rows.

use std::collections::BTreeMap;

use nidoo_api::{AreaView, EntityView};
use nidoo_core::model::{
    Availability, Capabilities, Command, Kind, Placement, SensorGroup, SensorSection, StateValue,
    SwitchType, UNASSIGNED_AREA,
};

use crate::i18n::Lang;
use crate::i18n::security::{
    SecurityText, action_question, group_count, panel_state, security_text, sensor_label,
    sensor_state,
};
use crate::i18n::states::on_off;

/// An alarm panel. `state` is the raw alarm state string; `None`: nothing received yet.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelUi {
    pub id: String,
    pub name: String,
    pub state: Option<String>,
    pub available: bool,
    pub capabilities: Capabilities,
    pub order: Option<i32>,
    /// Unix seconds the exit or entry delay ends (the native alarm's `deadline` attribute).
    pub deadline: Option<u64>,
    /// What set the alarm off, as display names (the native alarm's `cause` attribute).
    pub cause: Vec<String>,
    /// How the alarm was armed: `presence`, `remote`, `sos` or `command`.
    pub source: Option<String>,
}

/// The panel's `cause` attribute with each entity id replaced by its display name. An id the
/// house lost stays as it is, and `sos:<remote>` stays: the hero words it.
fn cause_names(
    attributes: &serde_json::Map<String, serde_json::Value>,
    entities: &BTreeMap<String, EntityView>,
) -> Vec<String> {
    attributes
        .get("cause")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .map(|id| {
            entities
                .get(id)
                .map_or_else(|| id.to_owned(), |e| e.name.clone())
        })
        .collect()
}

/// A security binary sensor. `active` is `None` until a state arrives; `area_name` is `None`
/// for Unassigned.
#[derive(Clone, Debug, PartialEq)]
pub struct SensorRow {
    pub id: String,
    pub name: String,
    pub area_name: Option<String>,
    pub device_class: Option<String>,
    pub group: SensorGroup,
    pub active: Option<bool>,
    pub available: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SectionUi {
    pub section: SensorSection,
    pub rows: Vec<SensorRow>,
}

/// A switch or button opted onto Security. `on` is `None` for a button or before a state.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionRow {
    pub id: String,
    pub name: String,
    pub area_name: Option<String>,
    pub kind: Kind,
    /// What a switch is shown as; `None` for a button.
    pub switch_type: Option<SwitchType>,
    pub on: Option<bool>,
    pub available: bool,
    pub ask: bool,
}

/// What a tap on an action does, and whether it asks first.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionTap {
    pub command: Command,
    pub confirm: bool,
}

impl ActionRow {
    pub fn tap(&self) -> ActionTap {
        let turn_on = || ActionTap {
            command: Command::TurnOn {
                brightness: None,
                color: None,
                effect: None,
            },
            confirm: self.ask,
        };
        // A door opener or a chime fires each time: there is nothing to turn off.
        if matches!(self.switch_type, Some(SwitchType::Door | SwitchType::Chime)) {
            return turn_on();
        }
        match (self.kind, self.on) {
            (Kind::Switch, Some(true)) => ActionTap {
                command: Command::TurnOff,
                confirm: false,
            },
            (Kind::Switch, _) => ActionTap {
                command: Command::TurnOn {
                    brightness: None,
                    color: None,
                    effect: None,
                },
                confirm: self.ask,
            },
            _ => ActionTap {
                command: Command::Press,
                confirm: self.ask,
            },
        }
    }

    /// What a tap does to this kind of device, as the word on the confirm button.
    pub fn verb_key(&self) -> SecurityText {
        match (self.kind, self.switch_type) {
            (Kind::Button, _) => SecurityText::VerbPress,
            (_, Some(SwitchType::Door)) => SecurityText::VerbOpen,
            (_, Some(SwitchType::Siren)) => SecurityText::VerbRing,
            (_, Some(SwitchType::Chime)) => SecurityText::VerbPlay,
            _ => SecurityText::VerbTurnOn,
        }
    }

    /// The word on the confirm button: what a tap does to this kind of device.
    pub fn verb(&self, lang: Lang) -> &'static str {
        security_text(lang, self.verb_key())
    }

    /// "Open Gate?": what the tile asks before acting.
    pub fn question(&self, lang: Lang) -> String {
        action_question(lang, self.verb_key(), &self.name)
    }

    /// "On" / "Off" / "Unavailable" / "—"; `None` for an available button, which has no state.
    pub fn state_word(&self, lang: Lang) -> Option<String> {
        if !self.available {
            return Some(security_text(lang, SecurityText::Unavailable).to_owned());
        }
        if self.kind == Kind::Button {
            return None;
        }
        Some(match self.on {
            Some(on) => on_off(lang, false, on).to_owned(),
            None => "—".to_owned(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupCount {
    pub group: SensorGroup,
    pub count: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SecurityUi {
    pub panels: Vec<PanelUi>,
    pub summary: Vec<GroupCount>,
    pub sensors: Vec<SensorRow>,
    pub sections: Vec<SectionUi>,
    pub actions: Vec<ActionRow>,
}

/// The stable tile order: area name (Unassigned last), then name, then id.
type SortKey = (bool, String, String, String);

fn sort_key(area_name: &Option<String>, name: &str, id: &str) -> SortKey {
    (
        area_name.is_none(),
        area_name.as_deref().unwrap_or_default().to_lowercase(),
        name.to_lowercase(),
        id.to_owned(),
    )
}

fn area_name(e: &EntityView, names: &BTreeMap<&str, &str>) -> Option<String> {
    if e.area == UNASSIGNED_AREA {
        None
    } else {
        Some(
            names
                .get(e.area.as_str())
                .map_or_else(|| e.area.clone(), |n| (*n).to_owned()),
        )
    }
}

fn is_on(e: &EntityView) -> Option<bool> {
    match e.state.value {
        StateValue::OnOff(on) => Some(on),
        _ => None,
    }
}

fn is_available(e: &EntityView) -> bool {
    e.state.availability != Availability::Offline
}

impl SecurityUi {
    pub fn build(entities: &BTreeMap<String, EntityView>, areas: &[AreaView]) -> SecurityUi {
        let names: BTreeMap<&str, &str> = areas
            .iter()
            .map(|a| (a.id.as_str(), a.name.as_str()))
            .collect();
        // Hidden on Home still shows here: only `securityHidden` takes an entity off Security.
        let visible: Vec<&EntityView> = entities
            .values()
            .filter(|e| {
                e.part_of.is_none()
                    && !e.id.starts_with("group_")
                    && Placement::on_security(e.kind, e.device_class.as_deref(), &e.ui)
            })
            .collect();

        let mut panels: Vec<PanelUi> = visible
            .iter()
            .filter(|e| e.kind == Kind::AlarmPanel)
            .map(|e| PanelUi {
                id: e.id.clone(),
                name: e.name.clone(),
                state: match &e.state.value {
                    StateValue::Alarm(s) => Some(s.clone()),
                    _ => None,
                },
                available: is_available(e),
                capabilities: e.capabilities.clone(),
                order: e.ui.order,
                deadline: e.state.attributes.get("deadline").and_then(|v| v.as_u64()),
                cause: cause_names(&e.state.attributes, entities),
                source: e
                    .state
                    .attributes
                    .get("source")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned),
            })
            .collect();
        panels.sort_by_cached_key(|p| {
            (
                p.order.unwrap_or(i32::MAX),
                p.name.to_lowercase(),
                p.id.clone(),
            )
        });

        let mut sensors: Vec<SensorRow> = visible
            .iter()
            .filter(|e| e.kind == Kind::BinarySensor)
            .filter_map(|e| {
                let group = SensorGroup::of(e.device_class.as_deref())?;
                Some(SensorRow {
                    id: e.id.clone(),
                    name: e.name.clone(),
                    area_name: area_name(e, &names),
                    device_class: e.device_class.clone(),
                    group,
                    active: is_on(e),
                    available: is_available(e),
                })
            })
            .collect();
        sensors.sort_by_cached_key(|s| sort_key(&s.area_name, &s.name, &s.id));

        let mut actions: Vec<ActionRow> = visible
            .iter()
            .filter(|e| Placement::action_kind(e.kind))
            .map(|e| ActionRow {
                id: e.id.clone(),
                name: e.name.clone(),
                area_name: area_name(e, &names),
                kind: e.kind,
                switch_type: SwitchType::of(e.protocol_kind, &e.ui),
                on: is_on(e),
                available: is_available(e),
                ask: e.ui.security_ask,
            })
            .collect();
        actions.sort_by_cached_key(|a| sort_key(&a.area_name, &a.name, &a.id));

        let summary = SensorGroup::ALL
            .iter()
            .filter_map(|&group| {
                let count = sensors
                    .iter()
                    .filter(|s| s.group == group && s.available && s.active == Some(true))
                    .count();
                (count > 0).then_some(GroupCount { group, count })
            })
            .collect();

        let sections = SensorSection::ALL
            .iter()
            .filter_map(|&section| {
                let rows: Vec<SensorRow> = sensors
                    .iter()
                    .filter(|s| SensorSection::of(s.group) == section)
                    .cloned()
                    .collect();
                (!rows.is_empty()).then_some(SectionUi { section, rows })
            })
            .collect();

        SecurityUi {
            panels,
            summary,
            sensors,
            sections,
            actions,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.panels.is_empty() && self.sensors.is_empty() && self.actions.is_empty()
    }

    /// "1 smoke · 2 open · 1 motion", or "All clear".
    pub fn summary_text(&self, lang: Lang) -> String {
        if self.summary.is_empty() {
            return security_text(lang, SecurityText::AllClear).to_owned();
        }
        self.summary
            .iter()
            .map(|c| group_count(lang, c.group, c.count))
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

impl SensorRow {
    pub fn state_word(&self, lang: Lang) -> String {
        if !self.available {
            return security_text(lang, SecurityText::Unavailable).to_owned();
        }
        let Some(active) = self.active else {
            return "—".to_owned();
        };
        sensor_state(lang, self.group, self.device_class.as_deref(), active)
    }

    fn active_now(&self) -> bool {
        self.available && self.active == Some(true)
    }

    pub fn danger_active(&self) -> bool {
        self.active_now() && self.group.danger()
    }

    pub fn intrusion_active(&self) -> bool {
        self.active_now() && !self.group.danger()
    }

    /// "Garage door, garage, open".
    pub fn aria_label(&self, lang: Lang) -> String {
        sensor_label(
            &self.name,
            self.area_name.as_deref(),
            &self.state_word(lang),
        )
    }
}

/// "Disarmed", "Armed away", "Exit delay…", "Entry delay…", "Triggered!", "Disarming…",
/// "Unavailable", "—" (French in French).
pub fn panel_state_text(lang: Lang, state: Option<&str>, available: bool) -> String {
    panel_state(lang, state, available)
}

/// The hero's modifier class: "disarmed", "armed", "arming", "pending", "triggered",
/// "disarming" or "off" (unavailable or unknown).
pub fn panel_tone(state: Option<&str>, available: bool) -> &'static str {
    if !available {
        return "off";
    }
    match state {
        Some("disarmed") => "disarmed",
        Some(s) if s.starts_with("armed_") => "armed",
        Some("arming") => "arming",
        Some("pending") => "pending",
        Some("triggered") => "triggered",
        Some("disarming") => "disarming",
        _ => "off",
    }
}

#[cfg(test)]
mod tests;
