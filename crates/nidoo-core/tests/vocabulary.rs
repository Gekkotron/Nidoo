//! Every action `vocabulary::actions` offers for a recorded entity is accepted by the command
//! mapper, so the editors never offer what the hub would refuse.

mod common;

use nidoo_core::command::map_command;
use nidoo_core::model::{Command, Entity, EntityState};
use nidoo_core::template::Templates;
use nidoo_core::vocabulary::{Action, Param, actions};
use serde_json::{Value, json};

/// A value for every param (optional ones included), so the mapper sees each input.
fn default_value(entity: &Entity, param: &Param) -> Option<Value> {
    match param {
        Param::Number {
            min, step, factor, ..
        } => {
            // A scaled input (brightness % → 0..=255) is stored rounded.
            let stored = if *factor == 1.0 {
                *min
            } else {
                (min * factor).round()
            };
            // Whole-number fields (`position`, `brightness`) only parse from JSON integers.
            Some(if step.fract() == 0.0 && stored.fract() == 0.0 {
                json!(stored as i64)
            } else {
                json!(stored)
            })
        }
        Param::Choice { options, .. } => options.first().map(|o| json!(o)),
        Param::Text {
            key,
            optional: false,
            ..
        } if key == "code" => {
            let literal = entity
                .bindings
                .config
                .get("code")
                .and_then(Value::as_str)
                .filter(|c| !c.starts_with("REMOTE_CODE"));
            Some(json!(literal.unwrap_or("1234")))
        }
        Param::Text {
            optional: false, ..
        } => Some(json!("x")),
        Param::Colour { .. } => Some(json!({"h": 0.0, "s": 100.0})),
        _ => None,
    }
}

fn filled(entity: &Entity, action: &Action) -> Command {
    let mut command = serde_json::to_value(&action.command).expect("command JSON");
    for param in &action.params {
        if let Some(value) = default_value(entity, param) {
            command[param.key()] = value;
        }
    }
    serde_json::from_value(command).expect("filled command parses")
}

#[test]
fn every_fixture_action_maps() {
    let entities = common::all_entities();
    let mut checked = 0;
    for entity in &entities {
        for action in actions(entity.kind, &entity.component, &entity.capabilities) {
            let command = filled(entity, &action);
            map_command(entity, &EntityState::default(), &command, &Templates::new())
                .unwrap_or_else(|e| panic!("{} {command:?}: {e}", entity.unique_id));
            checked += 1;
        }
    }
    assert!(checked > 0, "no action was checked");
}

/// Kinds the recording lacks: a coded alarm panel, a water heater and a type-table fan.
#[test]
fn hand_written_actions_map() {
    let entities = [
        common::entity(
            "alarm_control_panel",
            json!({"command_topic": "home/alarm/set", "state_topic": "home/alarm", "code": "1234"}),
        ),
        common::entity(
            "water_heater",
            json!({"mode_command_topic": "home/boiler/mode/set",
                   "temperature_command_topic": "home/boiler/temp/set",
                   "modes": ["off", "eco"], "min_temp": 30, "max_temp": 60}),
        ),
        common::entity(
            "fan",
            json!({"name": "Fan", "unique_id": "f1", "state_topic": "f/s", "command_topic": "f/set",
                   "percentage_command_topic": "f/pct/set", "percentage_state_topic": "f/pct"}),
        ),
    ];
    for entity in &entities {
        let list = actions(entity.kind, &entity.component, &entity.capabilities);
        assert!(!list.is_empty(), "{} offers nothing", entity.component);
        for action in list {
            let command = filled(entity, &action);
            map_command(entity, &EntityState::default(), &command, &Templates::new())
                .unwrap_or_else(|e| panic!("{} {command:?}: {e}", entity.unique_id));
        }
    }
}
