use std::collections::BTreeMap;

use nidoo_api::EntityView;
use nidoo_core::flow::{Node, NodeKind, TriggerSignalParams};
use nidoo_core::model::{Capabilities, Command, EntityCategory, EntityState, Kind, UiOverride};
use serde_json::json;

use super::*;

fn ent(id: &str, name: &str) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: name.into(),
        device_name: None,
        label: None,
        kind: Kind::Generic,
        component: "lock".into(),
        protocol_kind: Kind::Generic,
        device_class: None,
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

fn ctx_of(entities: &BTreeMap<String, EntityView>) -> Ctx<'_> {
    Ctx {
        entities,
        areas: &[],
        flows: &[],
        labels: &crate::flow_text::NO_LABELS,
        lang: crate::i18n::Lang::En,
    }
}

#[test]
fn state_trigger_without_entity_reads_the_device_sentence() {
    let entities = BTreeMap::new();
    let node = Node {
        id: "t".into(),
        name: None,
        position: None,
        kind: NodeKind::TriggerState(Default::default()),
        extra: Default::default(),
    };
    assert_eq!(
        sentence(&node, &ctx_of(&entities)),
        "When a device changes or sends an event"
    );
}

#[test]
fn signal_trigger_without_entity_reads_the_device_sentence() {
    let entities = BTreeMap::new();
    let node = Node {
        id: "t".into(),
        name: None,
        position: None,
        kind: NodeKind::TriggerSignal(TriggerSignalParams::default()),
        extra: Default::default(),
    };
    assert_eq!(
        sentence(&node, &ctx_of(&entities)),
        "When a device changes or sends an event"
    );
}

#[test]
fn control_command_sentence_reads_naturally() {
    let entities = BTreeMap::new();
    let ctx = ctx_of(&entities);
    let c = |control: &str, value| Command::Control {
        control: control.into(),
        value,
    };
    assert_eq!(
        command_sentence(&c("locked", Some(json!(false))), "Front door", &ctx),
        "Unlock Front door"
    );
    assert_eq!(
        command_sentence(&c("percentage", Some(json!(60))), "Bedroom fan", &ctx),
        "Set Bedroom fan percentage to 60"
    );
    assert_eq!(
        command_sentence(&c("activate", None), "Scene 1", &ctx),
        "Activate Scene 1"
    );
    let fr = Ctx {
        lang: crate::i18n::Lang::Fr,
        ..ctx_of(&entities)
    };
    assert_eq!(
        command_sentence(&c("locked", Some(json!(true))), "Porte", &fr),
        "Verrouiller Porte"
    );
    assert_eq!(
        command_sentence(&c("percentage", Some(json!(60))), "Ventilo", &fr),
        "Régler Ventilo sur 60 (percentage)"
    );
    assert_eq!(
        command_sentence(
            &Command::SetHvacMode {
                mode: "heat".into()
            },
            "Clim",
            &fr
        ),
        "Régler Clim en mode chauffage"
    );
}

#[test]
fn string_values_follow_the_entity_kind() {
    let mut climate = ent("c", "Clim");
    climate.kind = Kind::Climate;
    let mut cover = ent("v", "Volet");
    cover.kind = Kind::Cover;
    let value = |lang, v: &str, e: &EntityView| value_words(lang, &json!(v), None, Some(e));
    assert_eq!(value(crate::i18n::Lang::Fr, "heat", &climate), "chauffage");
    assert_eq!(value(crate::i18n::Lang::Fr, "open", &cover), "ouvert");
    assert_eq!(value(crate::i18n::Lang::Fr, "whatever", &cover), "whatever");
}

#[test]
fn signal_trigger_sentence_names_the_signal() {
    let entities: BTreeMap<_, _> = [("r1".to_owned(), ent("r1", "Remote"))].into();
    let ctx = ctx_of(&entities);
    let node = |signal: Option<&str>| Node {
        id: String::new(),
        name: None,
        position: None,
        kind: NodeKind::TriggerSignal(TriggerSignalParams {
            entity: "r1".into(),
            signal: signal.map(Into::into),
            ..Default::default()
        }),
        extra: Default::default(),
    };
    assert_eq!(sentence(&node(Some("on")), &ctx), "When Remote sends on");
    assert_eq!(sentence(&node(None), &ctx), "When Remote sends a signal");
    let fr = Ctx {
        lang: crate::i18n::Lang::Fr,
        ..ctx_of(&entities)
    };
    assert_eq!(sentence(&node(Some("on")), &fr), "Quand Remote envoie on");
    assert_eq!(sentence(&node(None), &fr), "Quand Remote envoie un signal");
    assert_eq!(
        sentence(&node(Some("brightness_up")), &ctx),
        "When Remote sends brightness_up"
    );
    assert_eq!(sentence(&node(Some("1_hold")), &ctx), "When button 1 held");
    assert_eq!(sentence(&node(Some("double")), &fr), "Quand appuyé 2×");
}

#[test]
fn press_signals_read_as_button_words() {
    use crate::i18n::Lang::{En, Fr};
    use crate::i18n::flows::{press_label, signal_chip_label};
    let cases = [
        ("1_single", "button 1 pressed", "bouton 1 appuyé"),
        ("1_double", "button 1 pressed 2×", "bouton 1 appuyé 2×"),
        ("2_triple", "button 2 pressed 3×", "bouton 2 appuyé 3×"),
        ("3_hold", "button 3 held", "bouton 3 maintenu"),
        ("1_long", "button 1 held", "bouton 1 maintenu"),
        ("2_release", "button 2 released", "bouton 2 relâché"),
        ("double", "pressed 2×", "appuyé 2×"),
    ];
    for (signal, en, fr) in cases {
        assert_eq!(press_label(En, signal).as_deref(), Some(en), "{signal}");
        assert_eq!(press_label(Fr, signal).as_deref(), Some(fr), "{signal}");
    }
    assert_eq!(press_label(En, "brightness_up"), None);
    assert_eq!(press_label(En, "_hold"), None);
    assert_eq!(signal_chip_label(En, "1_hold"), "Button 1 held");
    assert_eq!(signal_chip_label(Fr, "single"), "Appuyé");
    assert_eq!(signal_chip_label(Fr, "brightness_up"), "brightness_up");
}

#[test]
fn if_with_an_average_names_the_window() {
    let entities = BTreeMap::new();
    let node = |average: Option<&str>| Node {
        id: "n".into(),
        name: None,
        position: None,
        kind: NodeKind::If(nidoo_core::flow::IfParams {
            op: Op::Lte,
            value: Some(json!(19)),
            entity: Some("Temperature".into()),
            field: None,
            missing: None,
            average: average.map(Into::into),
            offset_minutes: None,
            result: None,
            extra: Default::default(),
        }),
        extra: Default::default(),
    };
    let en = ctx_of(&entities);
    let fr = Ctx {
        lang: crate::i18n::Lang::Fr,
        ..ctx_of(&entities)
    };
    assert_eq!(sentence(&node(None), &en), "Temperature \u{2264} 19?");
    assert_eq!(
        sentence(&node(Some("15m")), &en),
        "Temperature avg 15\u{a0}min \u{2264} 19?"
    );
    assert_eq!(
        sentence(&node(Some("1h30m")), &fr),
        "Temperature moy. 1\u{a0}h 30\u{a0}min \u{2264} 19\u{a0}?"
    );
}

fn area(id: &str, name: &str) -> nidoo_api::AreaView {
    nidoo_api::AreaView {
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

fn node_of(kind: NodeKind) -> Node {
    Node {
        id: "n".into(),
        name: None,
        position: None,
        kind,
        extra: Default::default(),
    }
}

#[test]
fn wake_up_sentence_names_alarm_or_all() {
    use nidoo_core::flow::TriggerWakeUpParams;
    let entities = BTreeMap::new();
    let flows = vec![
        ("wake-up-1".to_owned(), "Weekdays".to_owned()),
        ("wake-up-2".to_owned(), "Weekend".to_owned()),
    ];
    let ctx = Ctx {
        flows: &flows,
        ..ctx_of(&entities)
    };
    let all = node_of(NodeKind::TriggerWakeUp(TriggerWakeUpParams {
        before_minutes: 45,
        ..Default::default()
    }));
    assert_eq!(sentence(&all, &ctx), "45\u{a0}min before a wake-up alarm");
    let one = node_of(NodeKind::TriggerWakeUp(TriggerWakeUpParams {
        before_minutes: 45,
        alarms: vec!["wake-up-1".into()],
        ..Default::default()
    }));
    assert_eq!(sentence(&one, &ctx), "45\u{a0}min before Weekdays");
    let two = node_of(NodeKind::TriggerWakeUp(TriggerWakeUpParams {
        before_minutes: 0,
        alarms: vec!["wake-up-1".into(), "gone".into()],
        ..Default::default()
    }));
    assert_eq!(sentence(&two, &ctx), "At Weekdays, gone");
    let fr = Ctx {
        lang: crate::i18n::Lang::Fr,
        ..ctx
    };
    assert_eq!(sentence(&all, &fr), "45\u{a0}min avant un réveil");
}

#[test]
fn hold_sentence_off_and_until() {
    use nidoo_core::flow::{ClimateHoldParams, ClimateSetpoint};
    let entities = BTreeMap::new();
    let areas = vec![area("living", "Living room")];
    let ctx = Ctx {
        areas: &areas,
        ..ctx_of(&entities)
    };
    let hold = |setpoint, minutes, until: Option<&str>| {
        node_of(NodeKind::ClimateHold(ClimateHoldParams {
            zones: vec!["living".into()],
            setpoint,
            mode: None,
            minutes,
            until: until.map(Into::into),
            extra: Default::default(),
        }))
    };
    assert_eq!(
        sentence(&hold(ClimateSetpoint::Temp(21.0), Some(180), None), &ctx),
        "Hold Living room at 21\u{a0}\u{b0}C for 3\u{a0}h"
    );
    assert_eq!(
        sentence(&hold(ClimateSetpoint::Off, None, Some("10:00")), &ctx),
        "Hold Living room off until 10:00"
    );
}

#[test]
fn eco_sentence_lists_zone_names() {
    use nidoo_core::flow::ClimateEcoParams;
    let entities = BTreeMap::new();
    let areas = vec![area("bedroom", "Bedroom"), area("office", "Office")];
    let ctx = Ctx {
        areas: &areas,
        ..ctx_of(&entities)
    };
    let eco = node_of(NodeKind::ClimateEco(ClimateEcoParams {
        zones: vec!["bedroom".into(), "office".into(), "attic".into()],
        minutes: None,
        until: Some("18:00".into()),
        extra: Default::default(),
    }));
    assert_eq!(
        sentence(&eco, &ctx),
        "Eco in Bedroom, Office, attic until 18:00"
    );
}

#[test]
fn catch_up_suffix_only_when_on() {
    use nidoo_core::flow::{TriggerScheduleParams, TriggerSunParams, TriggerWakeUpParams};
    let entities = BTreeMap::new();
    let ctx = ctx_of(&entities);
    let schedule = |c| {
        node_of(NodeKind::TriggerSchedule(TriggerScheduleParams {
            cron: "0 7 * * *".into(),
            catch_up_minutes: c,
            ..Default::default()
        }))
    };
    let base = sentence(&schedule(None), &ctx);
    assert_eq!(sentence(&schedule(Some(0)), &ctx), base);
    assert_eq!(
        sentence(&schedule(Some(60)), &ctx),
        format!("{base}, catches up within 60\u{a0}min")
    );
    let sun = node_of(NodeKind::TriggerSun(TriggerSunParams {
        catch_up_minutes: Some(30),
        ..Default::default()
    }));
    assert!(sentence(&sun, &ctx).ends_with(", catches up within 30\u{a0}min"));
    let wake = node_of(NodeKind::TriggerWakeUp(TriggerWakeUpParams {
        before_minutes: 45,
        catch_up_minutes: Some(60),
        ..Default::default()
    }));
    assert!(sentence(&wake, &ctx).ends_with(", catches up within 60\u{a0}min"));
    let fr = Ctx {
        lang: crate::i18n::Lang::Fr,
        ..ctx
    };
    assert!(sentence(&wake, &fr).ends_with(", rattrape dans les 60\u{a0}min"));
}

#[test]
fn a_step_with_no_rooms_reads_a_placeholder() {
    use nidoo_core::flow::{ClimateEcoParams, ClimateHoldParams, ClimateSetpoint};
    let entities = BTreeMap::new();
    let ctx = ctx_of(&entities);
    let hold = node_of(NodeKind::ClimateHold(ClimateHoldParams {
        zones: Vec::new(),
        setpoint: ClimateSetpoint::Temp(20.0),
        mode: None,
        minutes: Some(120),
        until: None,
        extra: Default::default(),
    }));
    assert_eq!(
        sentence(&hold, &ctx),
        "Hold rooms\u{2026} at 20\u{a0}\u{b0}C for 2\u{a0}h"
    );
    let eco = node_of(NodeKind::ClimateEco(ClimateEcoParams {
        zones: Vec::new(),
        minutes: None,
        until: Some("18:00".into()),
        extra: Default::default(),
    }));
    assert_eq!(sentence(&eco, &ctx), "Eco in rooms\u{2026} until 18:00");
}
