//! What an entity accepts and reports, read only from what discovery recorded (`Kind`,
//! component, `Capabilities`, unit) and the type table. The editors draw these lists; they hold
//! no list of their own per device type.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::flow::Scalar;
use crate::model::{Capabilities, Command, Kind};
use crate::state::{ALARM_STATES, COVER_STATES, VACUUM_STATES};
use crate::types::{Shape, type_def};

/// What an entity can be told: a command template plus the inputs it takes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub command: Command,
    pub params: Vec<Param>,
}

/// One input of an action; `key` is the field inside the command's JSON (`position`,
/// `temperature`, `value`, `brightness`, `color`, `effect`, `code`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Param {
    /// Shown value × `factor` = stored value (brightness: shown 1–100 %, stored 0–255).
    Number {
        key: String,
        min: f64,
        max: f64,
        step: f64,
        unit: Option<String>,
        factor: f64,
        optional: bool,
    },
    Choice {
        key: String,
        options: Vec<String>,
        optional: bool,
    },
    Colour {
        key: String,
    },
    Text {
        key: String,
        secret: bool,
        optional: bool,
    },
}

impl Param {
    /// The field of the command's JSON this input writes.
    pub fn key(&self) -> &str {
        match self {
            Param::Number { key, .. }
            | Param::Choice { key, .. }
            | Param::Colour { key }
            | Param::Text { key, .. } => key,
        }
    }
}

/// What an entity can report, as the flow engine compares it (`Scalar::of_state` /
/// `Scalar::of_generic`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reports {
    Values {
        values: Vec<Scalar>,
    },
    Number {
        unit: Option<String>,
        min: Option<f64>,
        max: Option<f64>,
    },
    Text,
    Nothing,
}

fn words<'a>(list: impl IntoIterator<Item = &'a String>) -> Reports {
    Reports::Values {
        values: list.into_iter().map(|w| Scalar::Text(w.clone())).collect(),
    }
}

fn fixed_words(list: &[&str]) -> Reports {
    Reports::Values {
        values: list.iter().map(|w| Scalar::Text((*w).to_owned())).collect(),
    }
}

fn on_off() -> Reports {
    Reports::Values {
        values: vec![Scalar::Bool(true), Scalar::Bool(false)],
    }
}

/// What an entity of `kind` (discovery `component`) can report.
pub fn reports(kind: Kind, component: &str, caps: &Capabilities, unit: Option<&str>) -> Reports {
    match kind {
        Kind::Light | Kind::Switch | Kind::BinarySensor => on_off(),
        Kind::Cover => fixed_words(COVER_STATES),
        Kind::Vacuum => fixed_words(VACUUM_STATES),
        Kind::Climate | Kind::WaterHeater => words(&caps.hvac_modes),
        Kind::Select => words(&caps.options),
        Kind::AlarmPanel => {
            let armed: Vec<String> = caps
                .alarm_modes
                .iter()
                // The mode's serde name (`away`, `custom_bypass`) is the state's suffix.
                .filter_map(|mode| serde_json::to_value(mode).ok())
                .filter_map(|v| v.as_str().map(|m| format!("armed_{m}")))
                .collect();
            fixed_words(
                &ALARM_STATES
                    .iter()
                    .copied()
                    .filter(|s| !s.starts_with("armed_") || armed.iter().any(|a| a == s))
                    .collect::<Vec<_>>(),
            )
        }
        Kind::Number => Reports::Number {
            unit: unit.map(str::to_owned),
            min: caps.min,
            max: caps.max,
        },
        Kind::Sensor => match unit {
            Some(unit) => Reports::Number {
                unit: Some(unit.to_owned()),
                min: None,
                max: None,
            },
            None => Reports::Text,
        },
        Kind::Generic => match type_def(component).and_then(|def| def.primary.as_deref()) {
            Some(primary) => field_reports(component, caps, unit, primary),
            None => Reports::Nothing,
        },
        Kind::Button => Reports::Nothing,
    }
}

/// What field `name` of a type-table entity reports.
pub fn field_reports(
    component: &str,
    caps: &Capabilities,
    unit: Option<&str>,
    name: &str,
) -> Reports {
    let Some(field) = type_def(component).and_then(|def| def.fields.get(name)) else {
        return Reports::Nothing;
    };
    let generic = caps.generic.as_ref();
    match field.shape {
        Shape::OnOff => on_off(),
        Shape::Choice => match generic.and_then(|g| g.options.get(name)) {
            Some(options) => words(options),
            None => words(field.values.iter().map(|w| &w.value)),
        },
        Shape::Number => {
            let range = generic.and_then(|g| g.ranges.get(name));
            Reports::Number {
                unit: unit.map(str::to_owned),
                min: range.map(|r| r.min),
                max: range.map(|r| r.max),
            }
        }
        Shape::Text => Reports::Text,
        Shape::Picture | Shape::Version => Reports::Nothing,
    }
}

/// Light colour modes the colour input can drive.
const COLOUR_MODES: &[&str] = &["hs", "xy", "rgb", "rgbw", "rgbww"];

fn bare(command: Command) -> Action {
    Action {
        command,
        params: Vec::new(),
    }
}

fn number(key: &str, min: f64, max: f64, step: f64, unit: Option<&str>) -> Param {
    Param::Number {
        key: key.to_owned(),
        min,
        max,
        step,
        unit: unit.map(str::to_owned),
        factor: 1.0,
        optional: false,
    }
}

fn code(required: bool) -> Vec<Param> {
    if required {
        vec![Param::Text {
            key: "code".to_owned(),
            secret: true,
            optional: false,
        }]
    } else {
        Vec::new()
    }
}

/// The commands an entity of `kind` (discovery `component`) accepts, in the order the editors
/// offer them.
pub fn actions(kind: Kind, component: &str, caps: &Capabilities) -> Vec<Action> {
    match kind {
        Kind::Light => {
            let mut on = bare(Command::TurnOn {
                brightness: None,
                color: None,
                effect: None,
            });
            if caps.brightness {
                on.params.push(Param::Number {
                    key: "brightness".to_owned(),
                    min: 1.0,
                    max: 100.0,
                    step: 1.0,
                    unit: Some("%".to_owned()),
                    factor: 2.55,
                    optional: true,
                });
            }
            if caps
                .color_modes
                .iter()
                .any(|m| COLOUR_MODES.contains(&m.as_str()))
            {
                on.params.push(Param::Colour {
                    key: "color".to_owned(),
                });
            }
            if !caps.effects.is_empty() {
                on.params.push(Param::Choice {
                    key: "effect".to_owned(),
                    options: caps.effects.clone(),
                    optional: true,
                });
            }
            vec![on, bare(Command::TurnOff), bare(Command::Toggle)]
        }
        Kind::Switch => vec![
            bare(Command::TurnOn {
                brightness: None,
                color: None,
                effect: None,
            }),
            bare(Command::TurnOff),
            bare(Command::Toggle),
        ],
        Kind::Cover => {
            let mut list = Vec::new();
            if caps.open_close {
                list.extend([bare(Command::Open), bare(Command::Close)]);
            }
            if caps.stop {
                list.push(bare(Command::Stop));
            }
            if caps.position {
                list.push(Action {
                    command: Command::SetPosition { position: 0 },
                    params: vec![number("position", 0.0, 100.0, 1.0, Some("%"))],
                });
            }
            list
        }
        Kind::Climate | Kind::WaterHeater => {
            let (min, max) = (caps.min_temp.unwrap_or(5.0), caps.max_temp.unwrap_or(35.0));
            let step = caps.temp_step.unwrap_or(0.5);
            // A new action starts mid-range, on a step, rather than at the coldest setting.
            let start = (min + ((max - min) / 2.0 / step).round() * step).clamp(min, max);
            let mut list = vec![Action {
                command: Command::SetTemperature { temperature: start },
                params: vec![number("temperature", min, max, step, Some("°C"))],
            }];
            list.extend(
                caps.hvac_modes
                    .iter()
                    .map(|mode| bare(Command::SetHvacMode { mode: mode.clone() })),
            );
            list.extend(
                caps.fan_modes
                    .iter()
                    .map(|mode| bare(Command::SetFanMode { mode: mode.clone() })),
            );
            list
        }
        Kind::Number => {
            let min = caps.min.unwrap_or(0.0);
            vec![Action {
                command: Command::SetNumber { value: min },
                params: vec![number(
                    "value",
                    min,
                    caps.max.unwrap_or(100.0),
                    caps.step.unwrap_or(1.0),
                    None,
                )],
            }]
        }
        Kind::Select => caps
            .options
            .iter()
            .map(|option| {
                bare(Command::SelectOption {
                    option: option.clone(),
                })
            })
            .collect(),
        Kind::Button => vec![bare(Command::Press)],
        Kind::AlarmPanel => {
            let mut list = vec![Action {
                command: Command::Disarm { code: None },
                params: code(caps.code_disarm_required),
            }];
            list.extend(caps.alarm_modes.iter().map(|mode| Action {
                command: Command::Arm {
                    mode: *mode,
                    code: None,
                },
                params: code(caps.code_arm_required),
            }));
            list
        }
        Kind::Vacuum => caps
            .vacuum_actions
            .iter()
            .map(|action| {
                bare(Command::VacuumAction {
                    action: action.clone(),
                })
            })
            .chain(caps.fan_speeds.iter().map(|speed| {
                bare(Command::SetFanSpeed {
                    speed: speed.clone(),
                })
            }))
            .collect(),
        Kind::Generic => generic(component, caps),
        Kind::Sensor | Kind::BinarySensor => Vec::new(),
    }
}

/// One action per announced control of a type-table entity: two for an on/off control, one
/// per option for a choice, one with an input for a number or a text.
fn generic(component: &str, caps: &Capabilities) -> Vec<Action> {
    let (Some(def), Some(generic)) = (type_def(component), caps.generic.as_ref()) else {
        return Vec::new();
    };
    let control = |name: &str, value: Option<Value>| Command::Control {
        control: name.to_owned(),
        value,
    };
    let mut list = Vec::new();
    for name in &generic.controls {
        let Some(ctl) = def.controls.get(name) else {
            continue;
        };
        if ctl.press {
            list.push(bare(control(name, None)));
            continue;
        }
        let field = ctl.field.as_deref().unwrap_or(name.as_str());
        match ctl.shape {
            Shape::OnOff => {
                list.extend([true, false].map(|on| bare(control(name, Some(Value::Bool(on))))))
            }
            Shape::Choice => list.extend(
                generic
                    .options
                    .get(field)
                    .into_iter()
                    .flatten()
                    .map(|option| bare(control(name, Some(Value::String(option.clone()))))),
            ),
            Shape::Number => {
                let range = generic.ranges.get(field);
                let min = range.map_or(0.0, |r| r.min);
                list.push(Action {
                    command: control(name, Some(serde_json::json!(min))),
                    params: vec![number(
                        "value",
                        min,
                        range.map_or(100.0, |r| r.max),
                        range.map_or(1.0, |r| r.step),
                        None,
                    )],
                });
            }
            Shape::Text => list.push(Action {
                command: control(name, Some(Value::String(String::new()))),
                params: vec![Param::Text {
                    key: "value".to_owned(),
                    secret: generic.secret,
                    optional: false,
                }],
            }),
            Shape::Picture | Shape::Version => {}
        }
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::Scalar;
    use crate::model::AlarmMode;

    #[test]
    fn cover_without_stop_payload_offers_no_stop() {
        let caps = Capabilities {
            open_close: true,
            stop: false,
            position: true,
            ..Default::default()
        };
        let cmds: Vec<_> = actions(Kind::Cover, "cover", &caps)
            .into_iter()
            .map(|a| a.command)
            .collect();
        assert_eq!(
            cmds,
            vec![
                Command::Open,
                Command::Close,
                Command::SetPosition { position: 0 }
            ]
        );
    }

    #[test]
    fn colour_light_turn_on_has_brightness_colour_and_effect() {
        let caps = Capabilities {
            brightness: true,
            color_modes: ["xy".to_owned()].into(),
            effects: vec!["blink".into()],
            ..Default::default()
        };
        let list = actions(Kind::Light, "light", &caps);
        assert_eq!(
            list[0].params.iter().map(Param::key).collect::<Vec<_>>(),
            ["brightness", "color", "effect"]
        );
    }

    #[test]
    fn alarm_with_code_has_a_secret_code_param() {
        let caps = Capabilities {
            alarm_modes: [AlarmMode::Away].into(),
            code_arm_required: true,
            ..Default::default()
        };
        let list = actions(Kind::AlarmPanel, "alarm_control_panel", &caps);
        assert_eq!(list[0].command, Command::Disarm { code: None });
        assert_eq!(list[0].params, vec![]);
        assert_eq!(
            list[1].params,
            vec![Param::Text {
                key: "code".into(),
                secret: true,
                optional: false
            }]
        );
    }

    #[test]
    fn select_gives_one_action_per_option() {
        let caps = Capabilities {
            options: vec!["a".into(), "b".into()],
            ..Default::default()
        };
        assert_eq!(actions(Kind::Select, "select", &caps).len(), 2);
    }

    #[test]
    fn temperature_starts_mid_range_on_a_step() {
        let caps = Capabilities {
            min_temp: Some(5.0),
            max_temp: Some(35.0),
            temp_step: Some(0.5),
            ..Default::default()
        };
        assert_eq!(
            actions(Kind::Climate, "climate", &caps)[0].command,
            Command::SetTemperature { temperature: 20.0 }
        );
        let heater = Capabilities {
            min_temp: Some(30.0),
            max_temp: Some(61.0),
            temp_step: Some(1.0),
            ..Default::default()
        };
        assert_eq!(
            actions(Kind::WaterHeater, "water_heater", &heater)[0].command,
            Command::SetTemperature { temperature: 46.0 }
        );
    }

    #[test]
    fn sensors_accept_nothing() {
        assert!(actions(Kind::Sensor, "sensor", &Capabilities::default()).is_empty());
    }

    #[test]
    fn cover_reports_the_words_core_keeps() {
        let Reports::Values { values } =
            reports(Kind::Cover, "cover", &Capabilities::default(), None)
        else {
            panic!("cover reports words")
        };
        assert_eq!(
            values,
            ["open", "closed", "opening", "closing"].map(|w| Scalar::Text(w.into()))
        );
    }

    #[test]
    fn switch_reports_booleans() {
        assert_eq!(
            reports(Kind::Switch, "switch", &Capabilities::default(), None),
            Reports::Values {
                values: vec![Scalar::Bool(true), Scalar::Bool(false)]
            }
        );
    }

    #[test]
    fn alarm_reports_only_announced_armed_states() {
        let caps = Capabilities {
            alarm_modes: [AlarmMode::Away].into(),
            ..Default::default()
        };
        let Reports::Values { values } =
            reports(Kind::AlarmPanel, "alarm_control_panel", &caps, None)
        else {
            panic!("alarm reports words")
        };
        assert!(values.contains(&Scalar::Text("disarmed".into())));
        assert!(values.contains(&Scalar::Text("armed_away".into())));
        assert!(values.contains(&Scalar::Text("triggered".into())));
        assert!(!values.contains(&Scalar::Text("armed_home".into())));
    }

    #[test]
    fn sensor_with_unit_is_a_number() {
        assert_eq!(
            reports(Kind::Sensor, "sensor", &Capabilities::default(), Some("°C")),
            Reports::Number {
                unit: Some("°C".into()),
                min: None,
                max: None
            }
        );
        assert_eq!(
            reports(Kind::Sensor, "sensor", &Capabilities::default(), None),
            Reports::Text
        );
    }

    #[test]
    fn select_and_climate_report_their_announced_lists() {
        let caps = Capabilities {
            options: vec!["eco".into()],
            hvac_modes: vec!["heat".into()],
            ..Default::default()
        };
        assert_eq!(
            reports(Kind::Select, "select", &caps, None),
            Reports::Values {
                values: vec![Scalar::Text("eco".into())]
            }
        );
        assert_eq!(
            reports(Kind::Climate, "climate", &caps, None),
            Reports::Values {
                values: vec![Scalar::Text("heat".into())]
            }
        );
    }

    #[test]
    fn generic_lock_reports_its_primary_choice() {
        let generic = crate::types::GenericCaps {
            options: [("state".to_owned(), vec!["locked".into(), "unlocked".into()])].into(),
            ..Default::default()
        };
        let caps = Capabilities {
            generic: Some(generic),
            ..Default::default()
        };
        assert_eq!(
            reports(Kind::Generic, "lock", &caps, None),
            Reports::Values {
                values: vec![
                    Scalar::Text("locked".into()),
                    Scalar::Text("unlocked".into())
                ]
            }
        );
    }

    #[test]
    fn button_reports_nothing() {
        assert_eq!(
            reports(Kind::Button, "button", &Capabilities::default(), None),
            Reports::Nothing
        );
    }
}
