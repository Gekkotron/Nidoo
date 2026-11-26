use std::collections::BTreeMap;

use nidoo_api::{AreaView, EntityView};
use nidoo_core::model::{
    Capabilities, Command, EntityCategory, EntityState, Kind, SensorSection, StateValue, UiOverride,
};

use super::{SecurityUi, panel_state_text, panel_tone};
use crate::i18n::Lang;

fn entity(id: &str, kind: Kind, class: Option<&str>, area: &str, value: StateValue) -> EntityView {
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
