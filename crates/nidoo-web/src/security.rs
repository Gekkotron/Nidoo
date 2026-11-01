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
mod tests {
    use std::collections::BTreeMap;

    use nidoo_api::{AreaView, EntityView};
    use nidoo_core::model::{
        Capabilities, Command, EntityCategory, EntityState, Kind, SensorSection, StateValue,
        UiOverride,
    };

    use super::{SecurityUi, panel_state_text, panel_tone};
    use crate::i18n::Lang;

    fn entity(
        id: &str,
        kind: Kind,
        class: Option<&str>,
        area: &str,
        value: StateValue,
    ) -> EntityView {
        EntityView {
            id: id.into(),
            device_id: None,
            name: id.into(),
            device_name: None,
            label: None,
            kind,
            component: kind.component().to_string(),
            protocol_kind: kind,
            device_class: class.map(Into::into),
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: area.into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride::default(),
            state: EntityState {
                value,
                ..Default::default()
            },
        }
    }

    fn door(id: &str, area: &str, open: bool) -> EntityView {
        entity(
            id,
            Kind::BinarySensor,
            Some("door"),
            area,
            StateValue::OnOff(open),
        )
    }

    fn house(list: Vec<EntityView>) -> BTreeMap<String, EntityView> {
        list.into_iter().map(|e| (e.id.clone(), e)).collect()
    }

    fn area(id: &str, name: &str) -> AreaView {
        AreaView {
            id: id.into(),
            name: name.into(),
            icon: None,
            order: None,
            entity_count: 0,
            primary_count: 0,
            group_names: Default::default(),
            group_favorites: Default::default(),
        }
    }

    fn build(list: Vec<EntityView>) -> SecurityUi {
        SecurityUi::build(&house(list), &[area("garage", "Garage")])
    }

    #[test]
    fn filters_by_placement() {
        let mut hidden = door("hidden_door", "garage", false);
        hidden.ui.hidden = true;
        let mut off_security = door("off_door", "garage", false);
        off_security.ui.security_hidden = true;
        let mut folded = door("folded_door", "garage", false);
        folded.part_of = Some("parent".into());
        let plain_switch = entity(
            "plain",
            Kind::Switch,
            None,
            "garage",
            StateValue::OnOff(false),
        );
        let mut siren = entity(
            "siren",
            Kind::Switch,
            None,
            "garage",
            StateValue::OnOff(false),
        );
        siren.ui.security_action = true;
        let ui = build(vec![hidden, off_security, folded, plain_switch, siren]);
        let ids: Vec<_> = ui.sensors.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["hidden_door"]);
        let actions: Vec<_> = ui.actions.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(actions, ["siren"]);
    }

    #[test]
    fn group_entities_are_dropped() {
        let ui = build(vec![
            door("front", "hall", false),
            door("group_doors", "hall", false),
        ]);
        let ids: Vec<_> = ui.sensors.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["front"]);
    }

    #[test]
    fn stable_order_area_then_name() {
        let ui = build(vec![
            door("b", "Unassigned", false),
            door("z", "garage", false),
            door("a", "garage", true),
        ]);
        let ids: Vec<_> = ui.sensors.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["a", "z", "b"]);
        assert_eq!(ui.sensors[0].area_name.as_deref(), Some("Garage"));
        assert_eq!(ui.sensors[2].area_name, None);
    }

    #[test]
    fn summary_words() {
        let smoke = |id: &str, on| {
            entity(
                id,
                Kind::BinarySensor,
                Some("smoke"),
                "garage",
                StateValue::OnOff(on),
            )
        };
        let motion = entity(
            "m",
            Kind::BinarySensor,
            Some("motion"),
            "garage",
            StateValue::OnOff(true),
        );
        let mut offline = smoke("offline_smoke", true);
        offline.state.availability = nidoo_core::model::Availability::Offline;
        let ui = build(vec![
            smoke("s1", true),
            smoke("s2", false),
            offline,
            door("d1", "garage", true),
            door("d2", "garage", true),
            motion,
        ]);
        assert_eq!(ui.summary_text(Lang::En), "1 smoke · 2 open · 1 motion");
        assert_eq!(
            build(vec![door("d", "garage", false)]).summary_text(Lang::En),
            "All clear"
        );
    }

    #[test]
    fn sections_in_order_without_empties() {
        let ui = build(vec![
            entity(
                "w",
                Kind::BinarySensor,
                Some("moisture"),
                "garage",
                StateValue::OnOff(false),
            ),
            door("d", "garage", false),
            entity(
                "t",
                Kind::BinarySensor,
                Some("tamper"),
                "garage",
                StateValue::OnOff(false),
            ),
        ]);
        let sections: Vec<_> = ui.sections.iter().map(|s| s.section).collect();
        assert_eq!(
            sections,
            [
                SensorSection::Openings,
                SensorSection::Water,
                SensorSection::Other
            ]
        );
    }

    #[test]
    fn state_words_per_group() {
        let word = |class: &str, on: bool| {
            let ui = build(vec![entity(
                "x",
                Kind::BinarySensor,
                Some(class),
                "garage",
                StateValue::OnOff(on),
            )]);
            ui.sensors[0].state_word(Lang::En)
        };
        assert_eq!(
            (word("door", true), word("door", false)),
            ("Open".to_owned(), "Closed".to_owned())
        );
        assert_eq!(
            (word("motion", true), word("motion", false)),
            ("Motion".to_owned(), "Clear".to_owned())
        );
        assert_eq!(
            (word("gas", true), word("smoke", false)),
            ("Detected".to_owned(), "Clear".to_owned())
        );
        assert_eq!(
            (word("moisture", true), word("moisture", false)),
            ("Leak".to_owned(), "Dry".to_owned())
        );
        assert_eq!(
            (word("safety", true), word("safety", false)),
            ("SOS".to_owned(), "OK".to_owned())
        );
        assert_eq!(
            (word("vibration", true), word("vibration", false)),
            ("Vibration".to_owned(), "Still".to_owned())
        );
        assert_eq!(
            (word("tamper", true), word("tamper", false)),
            ("Tampered".to_owned(), "OK".to_owned())
        );
        let unknown = entity(
            "u",
            Kind::BinarySensor,
            Some("door"),
            "garage",
            StateValue::Unknown,
        );
        assert_eq!(build(vec![unknown]).sensors[0].state_word(Lang::En), "—");
        let mut offline = door("o", "garage", true);
        offline.state.availability = nidoo_core::model::Availability::Offline;
        let ui = build(vec![offline]);
        assert_eq!(ui.sensors[0].state_word(Lang::En), "Unavailable");
        assert!(!ui.sensors[0].danger_active() && !ui.sensors[0].intrusion_active());
    }

    #[test]
    fn aria_label_names_area_and_state() {
        let mut d = door("Garage door", "garage", true);
        d.name = "Garage door".into();
        let ui = build(vec![d]);
        assert_eq!(
            ui.sensors[0].aria_label(Lang::En),
            "Garage door, garage, open"
        );
        assert!(ui.sensors[0].intrusion_active());
    }

    fn action(kind: Kind, value: StateValue) -> super::ActionRow {
        let mut e = entity("a", kind, None, "garage", value);
        e.ui.security_action = true;
        build(vec![e]).actions.remove(0)
    }

    fn typed(show_as: &str, name: &str, value: StateValue) -> super::ActionRow {
        let mut e = entity("a", Kind::Switch, None, "garage", value);
        e.name = name.into();
        e.ui.security_action = true;
        e.ui.show_as = Some(show_as.into());
        build(vec![e]).actions.remove(0)
    }

    #[test]
    fn door_action_always_opens() {
        let row = typed("door", "Gate", StateValue::OnOff(true));
        let tap = row.tap();
        assert!(matches!(tap.command, Command::TurnOn { .. }));
        assert!(tap.confirm);
        assert_eq!(row.verb(Lang::En), "Open");
        assert_eq!(row.question(Lang::En), "Open Gate?");
    }

    #[test]
    fn chime_action_always_plays() {
        let row = typed("chime", "Bell", StateValue::OnOff(true));
        assert!(matches!(row.tap().command, Command::TurnOn { .. }));
        assert_eq!(row.verb(Lang::En), "Play");
    }

    #[test]
    fn siren_action_toggles() {
        let on = typed("siren", "Siren", StateValue::OnOff(true));
        assert_eq!(on.tap().command, Command::TurnOff);
        assert!(!on.tap().confirm);
        let off = typed("siren", "Siren", StateValue::OnOff(false));
        assert!(matches!(off.tap().command, Command::TurnOn { .. }));
        assert!(off.tap().confirm);
        assert_eq!(off.question(Lang::En), "Ring Siren?");
    }

    #[test]
    fn plain_verbs() {
        assert_eq!(
            action(Kind::Switch, StateValue::OnOff(false)).verb(Lang::En),
            "Turn on"
        );
        assert_eq!(
            typed("plug", "P", StateValue::OnOff(false)).verb(Lang::En),
            "Turn on"
        );
        assert_eq!(
            action(Kind::Button, StateValue::Unknown).verb(Lang::En),
            "Press"
        );
    }

    #[test]
    fn button_tap_asks_by_default() {
        let row = action(Kind::Button, StateValue::Unknown);
        let tap = row.tap();
        assert_eq!(tap.command, Command::Press);
        assert!(tap.confirm);
        let mut e = entity("a", Kind::Button, None, "garage", StateValue::Unknown);
        e.ui.security_action = true;
        e.ui.security_ask = false;
        assert!(!build(vec![e]).actions[0].tap().confirm);
    }

    #[test]
    fn button_has_no_state_word() {
        assert_eq!(
            action(Kind::Button, StateValue::Unknown).state_word(Lang::En),
            None
        );
        assert_eq!(
            action(Kind::Switch, StateValue::OnOff(false)).state_word(Lang::En),
            Some("Off".to_owned())
        );
        assert_eq!(
            action(Kind::Switch, StateValue::OnOff(true)).state_word(Lang::En),
            Some("On".to_owned())
        );
    }

    #[test]
    fn switch_on_turns_off_without_asking() {
        let tap = action(Kind::Switch, StateValue::OnOff(true)).tap();
        assert_eq!(tap.command, Command::TurnOff);
        assert!(!tap.confirm);
        let tap = action(Kind::Switch, StateValue::OnOff(false)).tap();
        assert!(matches!(tap.command, Command::TurnOn { .. }));
        assert!(tap.confirm);
    }

    #[test]
    fn panel_text_and_tone() {
        assert_eq!(
            panel_state_text(Lang::En, Some("armed_away"), true),
            "Armed away"
        );
        assert_eq!(panel_tone(Some("armed_away"), true), "armed");
        assert_eq!(
            panel_state_text(Lang::En, Some("triggered"), true),
            "Triggered!"
        );
        assert_eq!(panel_tone(Some("triggered"), true), "triggered");
        assert_eq!(panel_state_text(Lang::En, None, true), "—");
        assert_eq!(panel_tone(None, true), "off");
        assert_eq!(
            panel_state_text(Lang::En, Some("disarmed"), false),
            "Unavailable"
        );
        assert_eq!(panel_tone(Some("disarmed"), false), "off");
        assert_eq!(
            panel_state_text(Lang::En, Some("arming"), true),
            "Exit delay…"
        );
        assert_eq!(
            panel_state_text(Lang::En, Some("pending"), true),
            "Entry delay…"
        );
        assert_eq!(
            panel_state_text(Lang::En, Some("disarming"), true),
            "Disarming…"
        );
    }

    #[test]
    fn panels_sorted_by_order_then_name() {
        let mut a = entity(
            "b",
            Kind::AlarmPanel,
            None,
            "garage",
            StateValue::Alarm("disarmed".into()),
        );
        a.ui.order = Some(2);
        let b = entity(
            "a",
            Kind::AlarmPanel,
            None,
            "garage",
            StateValue::Alarm("armed_home".into()),
        );
        let mut c = entity("c", Kind::AlarmPanel, None, "garage", StateValue::Unknown);
        c.ui.order = Some(1);
        let ui = build(vec![a, b, c]);
        let ids: Vec<_> = ui.panels.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["c", "b", "a"]);
        assert_eq!(ui.panels[0].state, None);
    }
}
