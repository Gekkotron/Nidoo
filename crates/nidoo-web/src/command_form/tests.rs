use nidoo_core::model::{
    AlarmMode, Capabilities, EntityCategory, EntityState, Kind, StateValue, UiOverride,
};
use serde_json::json;

use super::*;
use nidoo_core::vocabulary::Param;

fn entity(kind: Kind, caps: Capabilities) -> EntityView {
    EntityView {
        id: "e".into(),
        device_id: None,
        name: "E".into(),
        device_name: None,
        label: None,
        kind,
        component: kind.component().to_string(),
        protocol_kind: kind,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "a".into(),
        part_of: None,
        capabilities: caps,
        ui: UiOverride::default(),
        state: EntityState::default(),
    }
}

fn bare(command: Command) -> Action {
    Action {
        command,
        params: vec![],
    }
}

#[test]
fn command_labels_follow_the_language() {
    let on = Command::TurnOn {
        brightness: None,
        color: None,
        effect: None,
    };
    assert_eq!(command_label(Lang::En, &on), "Turn on");
    assert_eq!(command_label(Lang::Fr, &on), "Allumer");
    assert_eq!(command_label(Lang::Fr, &Command::Stop), "Arrêter");
    assert_eq!(command_label(Lang::Fr, &Command::Press), "Appuyer");
    let pos = Command::SetPosition { position: 50 };
    assert_eq!(command_label(Lang::Fr, &pos), "Régler la position");
    let pick = Command::SelectOption {
        option: "low".into(),
    };
    assert_eq!(command_label(Lang::En, &pick), "Select low");
    assert_eq!(command_label(Lang::Fr, &pick), "Choisir low");
    let mode = Command::SetHvacMode {
        mode: "heat".into(),
    };
    assert_eq!(command_label(Lang::Fr, &mode), "Régler le mode chauffage");
}

#[test]
fn new_command_labels_follow_the_language() {
    let dock = Command::VacuumAction {
        action: "return_to_base".into(),
    };
    assert_eq!(command_label(Lang::Fr, &dock), "Retour à la base");
    let fan = Command::SetFanMode { mode: "low".into() };
    assert_eq!(command_label(Lang::En, &fan), "Fan: Low");
    let arm = Command::Arm {
        mode: AlarmMode::Away,
        code: None,
    };
    assert_eq!(command_label(Lang::Fr, &arm), "Activer (absent)");
    let lock = Command::Control {
        control: "locked".into(),
        value: Some(json!(true)),
    };
    assert_eq!(command_label(Lang::En, &lock), "Lock");
}

#[test]
fn setting_one_param_keeps_the_others() {
    let on = Command::TurnOn {
        brightness: Some(100),
        color: Some(SWATCHES[1].1),
        effect: None,
    };
    assert_eq!(
        with_param(&on, "effect", Some(json!("blink"))),
        Command::TurnOn {
            brightness: Some(100),
            color: Some(SWATCHES[1].1),
            effect: Some("blink".into())
        }
    );
    assert_eq!(
        with_param(&on, "color", None),
        Command::TurnOn {
            brightness: Some(100),
            color: None,
            effect: None
        }
    );
    assert_eq!(param_value(&on, "brightness"), Some(json!(100)));
}

#[test]
fn a_param_that_does_not_parse_is_ignored() {
    let pos = Command::SetPosition { position: 10 };
    assert_eq!(with_param(&pos, "position", Some(json!("abc"))), pos);
}

#[test]
fn whole_numbers_are_written_as_integers() {
    let pos = Command::SetPosition { position: 10 };
    assert_eq!(
        with_param(&pos, "position", Some(number_json(40.0, 1.0, 1.0))),
        Command::SetPosition { position: 40 }
    );
    assert_eq!(number_json(60.0, 2.55, 1.0), json!(153));
    assert_eq!(number_json(19.5, 1.0, 0.5), json!(19.5));
}

#[test]
fn unlisted_command_is_offered_first() {
    let list = vec![bare(Command::TurnOff)];
    let odd = Command::SetPosition { position: 10 };
    assert_eq!(command_options(&odd, &list)[0].command, odd);
    assert_eq!(command_options(&Command::TurnOff, &list).len(), 1);
}

#[test]
fn retarget_keeps_offered_command_else_first() {
    let cover = vec![bare(Command::Open), bare(Command::Close)];
    assert_eq!(retarget(&Command::Close, &cover), Command::Close);
    assert_eq!(retarget(&Command::TurnOff, &cover), Command::Open);
    assert_eq!(retarget(&Command::TurnOff, &[]), Command::TurnOff);
}

fn text_action() -> Action {
    Action {
        command: Command::Control {
            control: "value".into(),
            value: Some(json!("")),
        },
        params: vec![Param::Text {
            key: "value".into(),
            secret: false,
            optional: false,
        }],
    }
}

#[test]
fn a_typed_text_control_keeps_its_input() {
    let actions = vec![text_action()];
    let typed = Command::Control {
        control: "value".into(),
        value: Some(json!("h")),
    };
    assert_eq!(command_options(&typed, &actions).len(), 1);
    assert_eq!(
        matching_action(&typed, &actions).map(|a| a.params.len()),
        Some(1)
    );
    assert_eq!(retarget(&typed, &actions), typed);
}

#[test]
fn retarget_drops_inputs_the_new_entity_lacks() {
    let on = Command::TurnOn {
        brightness: Some(100),
        color: Some(SWATCHES[1].1),
        effect: Some("blink".into()),
    };
    let dimmer = vec![Action {
        command: Command::TurnOn {
            brightness: None,
            color: None,
            effect: None,
        },
        params: vec![Param::Number {
            key: "brightness".into(),
            min: 1.0,
            max: 100.0,
            step: 1.0,
            unit: Some("%".into()),
            factor: 2.55,
            optional: true,
        }],
    }];
    assert_eq!(
        retarget(&on, &dimmer),
        Command::TurnOn {
            brightness: Some(100),
            color: None,
            effect: None
        }
    );
}

#[test]
fn a_number_is_kept_on_its_step() {
    assert_eq!(number_json(40.4, 1.0, 1.0), json!(40));
    assert_eq!(number_json(19.3, 1.0, 0.5), json!(19.5));
}

#[test]
fn swatches_read_back_as_their_own_colour_name() {
    for (name, hs) in SWATCHES {
        assert_eq!(crate::flow_text::colour_name(hs), name);
    }
    assert_eq!(swatch_of(SWATCHES[2].1), Some(2));
    assert_eq!(swatch_of(Hs { h: 12.0, s: 80.0 }), None);
}

#[test]
fn lock_and_unlock_are_two_choices() {
    let lock = Command::Control {
        control: "locked".into(),
        value: Some(json!(true)),
    };
    let unlock = Command::Control {
        control: "locked".into(),
        value: Some(json!(false)),
    };
    assert_ne!(command_key(&lock), command_key(&unlock));
}

#[test]
fn a_number_entity_starts_at_its_current_value() {
    let mut n = entity(Kind::Number, Capabilities::default());
    n.state.value = StateValue::Number(42.0);
    assert_eq!(
        entity_actions(&n)[0].command,
        Command::SetNumber { value: 42.0 }
    );
}

#[test]
fn select_options_are_listed_from_core() {
    let caps = Capabilities {
        options: vec!["low".into(), "high".into()],
        ..Default::default()
    };
    let list = entity_actions(&entity(Kind::Select, caps));
    assert_eq!(
        list.into_iter().map(|a| a.command).collect::<Vec<_>>(),
        vec![
            Command::SelectOption {
                option: "low".into()
            },
            Command::SelectOption {
                option: "high".into()
            }
        ]
    );
}
