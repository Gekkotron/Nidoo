//! Command mapping against the recorded house fixtures and hand-written configs.

mod common;

use nidoo_core::command::{CommandError, Publish, map_command};
use nidoo_core::model::{
    AlarmMode, Command, Entity, EntityState, Hs, Kind, LightState, StateValue,
};
use nidoo_core::template::Templates;
use serde_json::{Value, json};

fn map(entity: &Entity, state: &EntityState, cmd: &Command) -> Result<Vec<Publish>, CommandError> {
    map_command(entity, state, cmd, &Templates::new())
}

fn ok(entity: &Entity, cmd: &Command) -> Vec<Publish> {
    map(entity, &EntityState::default(), cmd)
        .unwrap_or_else(|e| panic!("{} {cmd:?}: {e}", entity.unique_id))
}

fn err(entity: &Entity, state: &EntityState, cmd: &Command) -> CommandError {
    match map(entity, state, cmd) {
        Ok(publishes) => panic!("{} {cmd:?} mapped to {publishes:?}", entity.unique_id),
        Err(e) => e,
    }
}

fn publish(topic: &str, payload: &str) -> Publish {
    Publish {
        topic: topic.to_owned(),
        payload: payload.to_owned(),
        retain: false,
    }
}

fn json_payload(publish: &Publish) -> Value {
    serde_json::from_str(&publish.payload)
        .unwrap_or_else(|e| panic!("payload {:?} is not JSON: {e}", publish.payload))
}

fn turn_on(brightness: Option<u8>, color: Option<Hs>, effect: Option<&str>) -> Command {
    Command::TurnOn {
        brightness,
        color,
        effect: effect.map(str::to_owned),
    }
}

fn with_value(value: StateValue) -> EntityState {
    EntityState {
        value,
        ..EntityState::default()
    }
}

#[test]
fn json_light_turn_on_with_brightness_and_color() {
    // Recorded Zigbee2MQTT LED controller: `schema: json`, `brightness_scale: 254`.
    let leds = common::parse_fixture("light__0x1051dbfffe1a4058__light");
    let publishes = ok(
        &leds,
        &turn_on(Some(255), Some(Hs { h: 30.0, s: 50.0 }), Some("rainbow")),
    );
    assert_eq!(publishes.len(), 1);
    assert_eq!(publishes[0].topic, "zigbee2mqtt/rooms/bedroom1/leds/set");
    assert!(!publishes[0].retain);
    assert_eq!(
        json_payload(&publishes[0]),
        json!({"state": "ON", "brightness": 254, "color": {"h": 30.0, "s": 50.0}, "effect": "rainbow"})
    );

    // Half brightness scales to the device range: 128 * 254 / 255 = 127.498 rounds to 127.
    let publishes = ok(&leds, &turn_on(Some(128), None, None));
    assert_eq!(
        json_payload(&publishes[0]),
        json!({"state": "ON", "brightness": 127})
    );

    // Plain on / off.
    assert_eq!(
        json_payload(&ok(&leds, &turn_on(None, None, None))[0]),
        json!({"state": "ON"})
    );
    assert_eq!(
        json_payload(&ok(&leds, &Command::TurnOff)[0]),
        json!({"state": "OFF"})
    );

    // An effect the light does not list is out of range.
    assert!(matches!(
        err(
            &leds,
            &EntityState::default(),
            &turn_on(None, None, Some("disco"))
        ),
        CommandError::OutOfRange(_)
    ));
}

#[test]
fn default_schema_light_publishes_three_topics() {
    let light = common::entity(
        "light",
        json!({
            "command_topic": "home/lamp/set",
            "brightness_command_topic": "home/lamp/brightness/set",
            "brightness_scale": 100,
            "hs_command_topic": "home/lamp/hs/set",
            "effect_command_topic": "home/lamp/effect/set",
            "effect_list": ["blink"],
            "state_topic": "home/lamp"
        }),
    );
    let cmd = turn_on(Some(255), Some(Hs { h: 120.0, s: 100.0 }), None);
    // `on_command_type` defaults to `last`: attributes first, then ON.
    assert_eq!(
        ok(&light, &cmd),
        vec![
            publish("home/lamp/hs/set", "120,100"),
            publish("home/lamp/brightness/set", "100"),
            publish("home/lamp/set", "ON"),
        ]
    );
    assert_eq!(
        ok(&light, &turn_on(None, None, Some("blink"))),
        vec![
            publish("home/lamp/effect/set", "blink"),
            publish("home/lamp/set", "ON"),
        ]
    );
    assert_eq!(
        ok(&light, &Command::TurnOff),
        vec![publish("home/lamp/set", "OFF")]
    );

    // `hs_command_template` sees `hue` and `sat`, as Home Assistant documents.
    let templated = common::entity(
        "light",
        json!({
            "command_topic": "home/lamp/set",
            "hs_command_topic": "home/lamp/hs/set",
            "hs_command_template": "{\"hue\": {{ hue }}, \"sat\": {{ sat }}}",
            "brightness_command_topic": "home/lamp/brightness/set",
            "brightness_command_template": "{\"level\": {{ value }}}",
            "effect_command_topic": "home/lamp/effect/set",
            "effect_command_template": "{\"fx\": \"{{ value }}\"}",
            "effect_list": ["blink"],
            "state_topic": "home/lamp"
        }),
    );
    assert_eq!(
        ok(
            &templated,
            &turn_on(Some(255), Some(Hs { h: 120.0, s: 100.0 }), Some("blink"))
        ),
        vec![
            publish("home/lamp/hs/set", "{\"hue\": 120.0, \"sat\": 100.0}"),
            publish("home/lamp/brightness/set", "{\"level\": 255}"),
            publish("home/lamp/effect/set", "{\"fx\": \"blink\"}"),
            publish("home/lamp/set", "ON"),
        ]
    );

    // `first` sends ON before the attributes.
    let first = common::entity(
        "light",
        json!({
            "command_topic": "home/lamp/set",
            "brightness_command_topic": "home/lamp/brightness/set",
            "on_command_type": "first",
            "payload_on": "1",
            "payload_off": "0",
            "state_topic": "home/lamp"
        }),
    );
    assert_eq!(
        ok(&first, &turn_on(Some(51), None, None)),
        vec![
            publish("home/lamp/set", "1"),
            publish("home/lamp/brightness/set", "51"),
        ]
    );
    assert_eq!(
        ok(&first, &Command::TurnOff),
        vec![publish("home/lamp/set", "0")]
    );

    // `brightness` sends only the brightness: the given one, else the current one, else full.
    let brightness_only = common::entity(
        "light",
        json!({
            "command_topic": "home/lamp/set",
            "brightness_command_topic": "home/lamp/brightness/set",
            "on_command_type": "brightness",
            "state_topic": "home/lamp"
        }),
    );
    assert_eq!(
        ok(&brightness_only, &turn_on(Some(10), None, None)),
        vec![publish("home/lamp/brightness/set", "10")]
    );
    let dimmed = with_value(StateValue::Light(LightState {
        on: false,
        brightness: Some(77),
        ..LightState::default()
    }));
    assert_eq!(
        map(&brightness_only, &dimmed, &turn_on(None, None, None)).unwrap(),
        vec![publish("home/lamp/brightness/set", "77")]
    );
    assert_eq!(
        ok(&brightness_only, &turn_on(None, None, None)),
        vec![publish("home/lamp/brightness/set", "255")]
    );

    // A colour without a colour topic cannot be sent.
    let no_hs = err(
        &first,
        &EntityState::default(),
        &turn_on(None, Some(Hs { h: 0.0, s: 0.0 }), None),
    );
    assert_eq!(
        no_hs,
        CommandError::MissingTopic("hs_command_topic".to_owned())
    );
}

#[test]
fn toggle_uses_current_state() {
    let switch = common::parse_fixture("switch__0x00124b0024bc601e__switch");
    let topic = "zigbee2mqtt/rooms/garage/watering_stove/set";
    let on = with_value(StateValue::OnOff(true));
    let off = with_value(StateValue::OnOff(false));
    assert_eq!(
        map(&switch, &on, &Command::Toggle).unwrap(),
        vec![publish(topic, "OFF")]
    );
    assert_eq!(
        map(&switch, &off, &Command::Toggle).unwrap(),
        vec![publish(topic, "ON")]
    );
    // Unknown turns on.
    assert_eq!(ok(&switch, &Command::Toggle), vec![publish(topic, "ON")]);

    let leds = common::parse_fixture("light__0x1051dbfffe1a4058__light");
    let lit = with_value(StateValue::Light(LightState {
        on: true,
        brightness: Some(200),
        ..LightState::default()
    }));
    let publishes = map(&leds, &lit, &Command::Toggle).unwrap();
    assert_eq!(json_payload(&publishes[0]), json!({"state": "OFF"}));
    // Toggling on carries no attributes.
    assert_eq!(
        json_payload(&ok(&leds, &Command::Toggle)[0]),
        json!({"state": "ON"})
    );
}

#[test]
fn turn_on_at_zero_brightness_turns_off() {
    let leds = common::parse_fixture("light__0x1051dbfffe1a4058__light");
    let publishes = ok(&leds, &turn_on(Some(0), Some(Hs { h: 1.0, s: 1.0 }), None));
    assert_eq!(publishes.len(), 1);
    assert_eq!(json_payload(&publishes[0]), json!({"state": "OFF"}));

    let lamp = common::entity(
        "light",
        json!({
            "command_topic": "home/lamp/set",
            "brightness_command_topic": "home/lamp/brightness/set",
            "state_topic": "home/lamp"
        }),
    );
    assert_eq!(
        ok(&lamp, &turn_on(Some(0), None, None)),
        vec![publish("home/lamp/set", "OFF")]
    );
    // A switch shown as a light: off as well.
    let switch = common::parse_fixture("switch__0x00124b0024bc601e__switch");
    assert_eq!(
        ok(&switch, &turn_on(Some(0), None, None)),
        vec![publish(
            "zigbee2mqtt/rooms/garage/watering_stove/set",
            "OFF"
        )]
    );
}

#[test]
fn cover_open_close_stop_and_position_template() {
    let cover = common::entity(
        "cover",
        json!({
            "command_topic": "home/blind/set",
            "set_position_topic": "home/blind/position/set",
            "set_position_template": "{{ position }}",
            "state_topic": "home/blind",
            "position_topic": "home/blind/position"
        }),
    );
    assert_eq!(
        ok(&cover, &Command::Open),
        vec![publish("home/blind/set", "OPEN")]
    );
    assert_eq!(
        ok(&cover, &Command::Close),
        vec![publish("home/blind/set", "CLOSE")]
    );
    assert_eq!(
        ok(&cover, &Command::Stop),
        vec![publish("home/blind/set", "STOP")]
    );
    assert_eq!(
        ok(&cover, &Command::SetPosition { position: 42 }),
        vec![publish("home/blind/position/set", "42")]
    );
    assert!(matches!(
        err(
            &cover,
            &EntityState::default(),
            &Command::SetPosition { position: 101 }
        ),
        CommandError::OutOfRange(_)
    ));

    // Custom words, an inverted device range and no template: the bare scaled integer.
    let inverted = common::entity(
        "cover",
        json!({
            "command_topic": "home/shade/set",
            "payload_open": "up",
            "payload_close": "down",
            "payload_stop": "halt",
            "set_position_topic": "home/shade/position/set",
            "position_open": 0,
            "position_closed": 100,
            "state_topic": "home/shade"
        }),
    );
    assert_eq!(
        ok(&inverted, &Command::Open),
        vec![publish("home/shade/set", "up")]
    );
    assert_eq!(
        ok(&inverted, &Command::Close),
        vec![publish("home/shade/set", "down")]
    );
    assert_eq!(
        ok(&inverted, &Command::Stop),
        vec![publish("home/shade/set", "halt")]
    );
    assert_eq!(
        ok(&inverted, &Command::SetPosition { position: 25 }),
        vec![publish("home/shade/position/set", "75")]
    );

    // Without a position topic the position cannot be set.
    let plain = common::entity(
        "cover",
        json!({"command_topic": "home/door/set", "state_topic": "home/door"}),
    );
    assert_eq!(
        err(
            &plain,
            &EntityState::default(),
            &Command::SetPosition { position: 50 }
        ),
        CommandError::MissingTopic("set_position_topic".to_owned())
    );

    // Recorded Zigbee2MQTT shutter: `set_position_template` wraps the position in JSON.
    let shutter = common::parse_fixture("cover__0x000d6ffffe9d44e4__cover");
    let publishes = ok(&shutter, &Command::SetPosition { position: 60 });
    assert_eq!(publishes.len(), 1);
    assert_eq!(publishes[0].topic, "zigbee2mqtt/rooms/office/shutter/set");
    assert_eq!(json_payload(&publishes[0]), json!({"position": 60}));
}

#[test]
fn climate_set_temperature_with_template() {
    let climate = common::entity(
        "climate",
        json!({
            "temperature_command_topic": "home/hvac/temp/set",
            "temperature_command_template": "{\"target\": {{ value }}}",
            "mode_command_topic": "home/hvac/mode/set",
            "mode_command_template": "{{ value | upper }}",
            "fan_mode_command_topic": "home/hvac/fan/set",
            "modes": ["off", "heat", "cool"],
            "fan_modes": ["auto", "low"],
            "min_temp": 7,
            "max_temp": 30
        }),
    );
    assert_eq!(
        ok(&climate, &Command::SetTemperature { temperature: 21.5 }),
        vec![publish("home/hvac/temp/set", "{\"target\": 21.5}")]
    );
    assert_eq!(
        ok(&climate, &Command::SetTemperature { temperature: 20.0 }),
        vec![publish("home/hvac/temp/set", "{\"target\": 20}")]
    );
    assert_eq!(
        ok(
            &climate,
            &Command::SetHvacMode {
                mode: "heat".to_owned()
            }
        ),
        vec![publish("home/hvac/mode/set", "HEAT")]
    );
    assert_eq!(
        ok(
            &climate,
            &Command::SetFanMode {
                mode: "low".to_owned()
            }
        ),
        vec![publish("home/hvac/fan/set", "low")]
    );
    let state = EntityState::default();
    assert!(matches!(
        err(
            &climate,
            &state,
            &Command::SetTemperature { temperature: 35.0 }
        ),
        CommandError::OutOfRange(_)
    ));
    assert!(matches!(
        err(
            &climate,
            &state,
            &Command::SetHvacMode {
                mode: "dry".to_owned()
            }
        ),
        CommandError::OutOfRange(_)
    ));
    assert!(matches!(
        err(
            &climate,
            &state,
            &Command::SetFanMode {
                mode: "turbo".to_owned()
            }
        ),
        CommandError::OutOfRange(_)
    ));

    // Recorded Daikin bridge: bare values, no templates.
    let daikin = common::parse_fixture("climate__daikin_192_168_1_150__climate");
    assert_eq!(
        ok(&daikin, &Command::SetTemperature { temperature: 23.5 }),
        vec![publish("daikin/192.168.1.150/ha/temperature/set", "23.5")]
    );
    assert_eq!(
        ok(
            &daikin,
            &Command::SetHvacMode {
                mode: "cool".to_owned()
            }
        ),
        vec![publish("daikin/192.168.1.150/ha/mode/set", "cool")]
    );
}

#[test]
fn select_option_with_command_template() {
    let select = common::entity(
        "select",
        json!({
            "command_topic": "home/fan/set",
            "command_template": "{\"speed\": \"{{ value }}\"}",
            "options": ["low", "high"],
            "state_topic": "home/fan"
        }),
    );
    assert_eq!(
        ok(
            &select,
            &Command::SelectOption {
                option: "high".to_owned()
            }
        ),
        vec![publish("home/fan/set", "{\"speed\": \"high\"}")]
    );
    assert!(matches!(
        err(
            &select,
            &EntityState::default(),
            &Command::SelectOption {
                option: "medium".to_owned()
            }
        ),
        CommandError::OutOfRange(_)
    ));

    // Recorded Zigbee2MQTT select: bare option on its own command topic.
    let indicator = common::parse_fixture("select__0x000d6ffffe9d44e4__indicator_mode");
    assert_eq!(
        ok(
            &indicator,
            &Command::SelectOption {
                option: "on/off".to_owned()
            }
        ),
        vec![publish(
            "zigbee2mqtt/rooms/office/shutter/set/indicator_mode",
            "on/off"
        )]
    );
}

#[test]
fn number_set_with_range_and_template() {
    let number = common::entity(
        "number",
        json!({
            "command_topic": "home/dimmer/set",
            "command_template": "{\"level\": {{ value }}}",
            "min": 0,
            "max": 10,
            "state_topic": "home/dimmer"
        }),
    );
    assert_eq!(
        ok(&number, &Command::SetNumber { value: 7.5 }),
        vec![publish("home/dimmer/set", "{\"level\": 7.5}")]
    );
    assert_eq!(
        ok(&number, &Command::SetNumber { value: 3.0 }),
        vec![publish("home/dimmer/set", "{\"level\": 3}")]
    );
    let state = EntityState::default();
    assert!(matches!(
        err(&number, &state, &Command::SetNumber { value: 11.0 }),
        CommandError::OutOfRange(_)
    ));
    assert!(matches!(
        err(&number, &state, &Command::SetNumber { value: f64::NAN }),
        CommandError::OutOfRange(_)
    ));

    // Recorded Zigbee2MQTT number.
    let count = common::parse_fixture("number__0x1051dbfffe1a4058__active_count");
    let publishes = ok(&count, &Command::SetNumber { value: 3.0 });
    assert_eq!(publishes.len(), 1);
    assert_eq!(
        publishes[0].topic,
        count.bindings.config["command_topic"].as_str().unwrap()
    );
}

#[test]
fn button_press_and_switch_payloads() {
    let button = common::parse_fixture("button__0x00158d0005422e73__identify");
    let publishes = ok(&button, &Command::Press);
    assert_eq!(publishes.len(), 1);
    assert_eq!(
        publishes[0].topic,
        button.bindings.config["command_topic"].as_str().unwrap()
    );
    let expected = button.bindings.config["payload_press"]
        .as_str()
        .unwrap_or("PRESS");
    assert_eq!(publishes[0].payload, expected);

    let custom = common::entity(
        "switch",
        json!({
            "command_topic": "home/relay/set",
            "payload_on": "go",
            "payload_off": "halt",
            "command_template": "{\"relay\": \"{{ value }}\"}",
            "state_topic": "home/relay"
        }),
    );
    assert_eq!(
        ok(&custom, &turn_on(None, None, None)),
        vec![publish("home/relay/set", "{\"relay\": \"go\"}")]
    );
    assert_eq!(
        ok(&custom, &Command::TurnOff),
        vec![publish("home/relay/set", "{\"relay\": \"halt\"}")]
    );
}

#[test]
fn retain_option_makes_command_publishes_retained() {
    let switch = common::entity(
        "switch",
        json!({
            "command_topic": "home/virtual/set",
            "state_topic": "home/virtual",
            "retain": true
        }),
    );
    let publishes = ok(&switch, &turn_on(None, None, None));
    assert!(!publishes.is_empty());
    assert!(publishes.iter().all(|p| p.retain), "{publishes:?}");
}

#[test]
fn commands_are_not_retained_without_the_retain_option() {
    for config in [
        json!({"command_topic": "home/virtual/set", "state_topic": "home/virtual"}),
        json!({"command_topic": "home/virtual/set", "state_topic": "home/virtual", "retain": false}),
        json!({"command_topic": "home/virtual/set", "state_topic": "home/virtual", "retain": "true"}),
    ] {
        let switch = common::entity("switch", config);
        let publishes = ok(&switch, &turn_on(None, None, None));
        assert!(!publishes.is_empty());
        assert!(publishes.iter().all(|p| !p.retain), "{publishes:?}");
    }
}

#[test]
fn alarm_arm_requires_code_when_configured() {
    let alarm = common::entity(
        "alarm_control_panel",
        json!({
            "command_topic": "home/alarm/set",
            "state_topic": "home/alarm",
            "code": "1234",
            "code_disarm_required": false
        }),
    );
    let state = EntityState::default();
    assert_eq!(
        err(
            &alarm,
            &state,
            &Command::Arm {
                mode: AlarmMode::Away,
                code: None
            }
        ),
        CommandError::MissingCode
    );
    assert_eq!(
        ok(
            &alarm,
            &Command::Arm {
                mode: AlarmMode::Away,
                code: Some("1234".to_owned())
            }
        ),
        vec![publish("home/alarm/set", "ARM_AWAY")]
    );
    assert_eq!(
        ok(
            &alarm,
            &Command::Arm {
                mode: AlarmMode::CustomBypass,
                code: Some("1234".to_owned())
            }
        ),
        vec![publish("home/alarm/set", "ARM_CUSTOM_BYPASS")]
    );
    // A code that differs from the configured literal is refused.
    assert_eq!(
        err(
            &alarm,
            &state,
            &Command::Arm {
                mode: AlarmMode::Away,
                code: Some("0000".to_owned())
            }
        ),
        CommandError::WrongCode
    );
    // Disarming does not need the code here, so none is checked.
    assert_eq!(
        ok(&alarm, &Command::Disarm { code: None }),
        vec![publish("home/alarm/set", "DISARM")]
    );
    assert_eq!(
        ok(
            &alarm,
            &Command::Disarm {
                code: Some("0000".to_owned())
            }
        ),
        vec![publish("home/alarm/set", "DISARM")]
    );

    let strict = common::entity(
        "alarm_control_panel",
        json!({
            "command_topic": "home/alarm/set",
            "state_topic": "home/alarm",
            "code": "REMOTE_CODE",
            "payload_disarm": "off",
            "payload_arm_home": "stay",
            "supported_features": ["arm_home"]
        }),
    );
    assert_eq!(
        err(&strict, &state, &Command::Disarm { code: None }),
        CommandError::MissingCode
    );
    assert_eq!(
        ok(
            &strict,
            &Command::Disarm {
                code: Some("0000".to_owned())
            }
        ),
        vec![publish("home/alarm/set", "off")]
    );
    assert_eq!(
        ok(
            &strict,
            &Command::Arm {
                mode: AlarmMode::Home,
                code: Some("0000".to_owned())
            }
        ),
        vec![publish("home/alarm/set", "stay")]
    );
    // `REMOTE_CODE` leaves the check to the device: any code goes through.
    assert_eq!(
        ok(
            &strict,
            &Command::Disarm {
                code: Some("whatever".to_owned())
            }
        ),
        vec![publish("home/alarm/set", "off")]
    );

    // A literal code is checked on disarm too.
    let literal = common::entity(
        "alarm_control_panel",
        json!({
            "command_topic": "home/alarm/set",
            "state_topic": "home/alarm",
            "code": "4321"
        }),
    );
    assert_eq!(
        err(
            &literal,
            &state,
            &Command::Disarm {
                code: Some("1234".to_owned())
            }
        ),
        CommandError::WrongCode
    );
    assert_eq!(
        err(&literal, &state, &Command::Disarm { code: None }),
        CommandError::MissingCode
    );
    assert_eq!(
        ok(
            &literal,
            &Command::Disarm {
                code: Some("4321".to_owned())
            }
        ),
        vec![publish("home/alarm/set", "DISARM")]
    );

    // A mode the panel does not support is out of range.
    assert!(matches!(
        err(
            &strict,
            &state,
            &Command::Arm {
                mode: AlarmMode::Night,
                code: Some("0000".to_owned())
            }
        ),
        CommandError::OutOfRange(_)
    ));
}

#[test]
fn recorded_alarm_renders_command_template() {
    // Jeedom bridge: no code configured, JSON `command_template` with `action` and `code`.
    let alarm = common::parse_fixture("alarm_control_panel__jeedom__house_alarm");
    let publishes = ok(
        &alarm,
        &Command::Arm {
            mode: AlarmMode::Night,
            code: Some("4321".to_owned()),
        },
    );
    assert_eq!(publishes.len(), 1);
    assert_eq!(publishes[0].topic, "jeedom/alarm/set");
    assert_eq!(
        json_payload(&publishes[0]),
        json!({"action": "ARM_NIGHT", "code": "4321"})
    );
    // No code required: arming without one still goes out.
    let publishes = ok(
        &alarm,
        &Command::Arm {
            mode: AlarmMode::Home,
            code: None,
        },
    );
    assert_eq!(
        json_payload(&publishes[0]),
        json!({"action": "ARM_HOME", "code": ""})
    );
    let publishes = ok(&alarm, &Command::Disarm { code: None });
    assert_eq!(
        json_payload(&publishes[0]),
        json!({"action": "DISARM", "code": ""})
    );
}

#[test]
fn vacuum_actions_and_fan_speed() {
    let vacuum = common::parse_fixture("vacuum__roborock_40313ca8b964__roborock_40313ca8b964");
    for action in ["start", "pause", "stop", "return_to_base", "locate"] {
        assert_eq!(
            ok(
                &vacuum,
                &Command::VacuumAction {
                    action: action.to_owned()
                }
            ),
            vec![publish("roborock/ha/command", action)],
            "{action}"
        );
    }
    assert_eq!(
        ok(
            &vacuum,
            &Command::SetFanSpeed {
                speed: "Turbo".to_owned()
            }
        ),
        vec![publish("roborock/ha/fan_speed/set", "Turbo")]
    );
    let state = EntityState::default();
    assert!(matches!(
        err(
            &vacuum,
            &state,
            &Command::VacuumAction {
                action: "clean_spot".to_owned()
            }
        ),
        CommandError::OutOfRange(_)
    ));
    assert!(matches!(
        err(
            &vacuum,
            &state,
            &Command::SetFanSpeed {
                speed: "Warp".to_owned()
            }
        ),
        CommandError::OutOfRange(_)
    ));

    let limited = common::entity(
        "vacuum",
        json!({
            "command_topic": "home/vac/cmd",
            "state_topic": "home/vac",
            "payload_start": "go",
            "supported_features": ["start", "stop"]
        }),
    );
    assert_eq!(
        ok(
            &limited,
            &Command::VacuumAction {
                action: "start".to_owned()
            }
        ),
        vec![publish("home/vac/cmd", "go")]
    );
    assert!(matches!(
        err(
            &limited,
            &state,
            &Command::VacuumAction {
                action: "locate".to_owned()
            }
        ),
        CommandError::OutOfRange(_)
    ));
    assert_eq!(
        err(
            &limited,
            &state,
            &Command::SetFanSpeed {
                speed: "max".to_owned()
            }
        ),
        CommandError::MissingTopic("set_fan_speed_topic".to_owned())
    );
}

#[test]
fn water_heater_mode_and_temperature() {
    let heater = common::entity(
        "water_heater",
        json!({
            "mode_command_topic": "home/boiler/mode/set",
            "mode_command_template": "{{ value | upper }}",
            "temperature_command_topic": "home/boiler/temp/set",
            "modes": ["off", "eco", "performance"],
            "min_temp": 30,
            "max_temp": 60
        }),
    );
    assert_eq!(
        ok(
            &heater,
            &Command::SetHvacMode {
                mode: "eco".to_owned()
            }
        ),
        vec![publish("home/boiler/mode/set", "ECO")]
    );
    assert_eq!(
        ok(&heater, &Command::SetTemperature { temperature: 55.0 }),
        vec![publish("home/boiler/temp/set", "55")]
    );
    let state = EntityState::default();
    assert!(matches!(
        err(
            &heater,
            &state,
            &Command::SetTemperature { temperature: 95.0 }
        ),
        CommandError::OutOfRange(_)
    ));
    assert!(matches!(
        err(
            &heater,
            &state,
            &Command::SetHvacMode {
                mode: "boost".to_owned()
            }
        ),
        CommandError::OutOfRange(_)
    ));
}

#[test]
fn unsupported_command_for_sensor() {
    let sensor = common::parse_fixture("sensor__0x5c0272fffebbe0c4__temperature");
    let state = EntityState::default();
    assert_eq!(
        err(&sensor, &state, &Command::SetPosition { position: 50 }),
        CommandError::Unsupported {
            kind: Kind::Sensor,
            command: "set_position".to_owned()
        }
    );
    assert_eq!(
        err(&sensor, &state, &Command::TurnOff),
        CommandError::Unsupported {
            kind: Kind::Sensor,
            command: "turn_off".to_owned()
        }
    );
    // A switch does not have a position either.
    let switch = common::parse_fixture("switch__0x00124b0024bc601e__switch");
    assert_eq!(
        err(&switch, &state, &Command::SetPosition { position: 50 }),
        CommandError::Unsupported {
            kind: Kind::Switch,
            command: "set_position".to_owned()
        }
    );
    // A button only presses.
    let button = common::parse_fixture("button__0x00158d0005422e73__identify");
    assert!(matches!(
        err(&button, &state, &turn_on(None, None, None)),
        CommandError::Unsupported {
            kind: Kind::Button,
            ..
        }
    ));
}

#[test]
fn recorded_switch_turn_on() {
    // ZBMINI two-way switch; the expectation follows its own config.
    let switch = common::parse_fixture("switch__0x00124b0024bc601e__switch");
    let config = &switch.bindings.config;
    let topic = config["command_topic"].as_str().expect("command_topic");
    let on_word = config["payload_on"].as_str().unwrap_or("ON");
    let expected = match config.get("command_template").and_then(Value::as_str) {
        Some(template) => Templates::new()
            .render(template, on_word)
            .expect("template"),
        None => on_word.to_owned(),
    };
    assert_eq!(
        ok(&switch, &turn_on(None, None, None)),
        vec![publish(topic, &expected)]
    );
    let off_word = config["payload_off"].as_str().unwrap_or("OFF");
    assert_eq!(
        ok(&switch, &Command::TurnOff),
        vec![publish(topic, off_word)]
    );
}

/// Every command on every recorded entity either maps to at least one publish or fails
/// cleanly; nothing panics and nothing is retained.
#[test]
fn every_command_on_every_fixture_is_mapped_or_rejected() {
    let templates = Templates::new();
    let entities = common::all_entities();
    assert!(entities.len() > 400);
    let commands = [
        turn_on(None, None, None),
        turn_on(Some(100), Some(Hs { h: 10.0, s: 10.0 }), Some("x")),
        Command::TurnOff,
        Command::Toggle,
        Command::SetPosition { position: 50 },
        Command::Open,
        Command::Close,
        Command::Stop,
        Command::SetTemperature { temperature: 21.0 },
        Command::SetHvacMode {
            mode: "heat".to_owned(),
        },
        Command::SetFanMode {
            mode: "auto".to_owned(),
        },
        Command::SetNumber { value: 1.0 },
        Command::SelectOption {
            option: "on".to_owned(),
        },
        Command::Press,
        Command::Arm {
            mode: AlarmMode::Away,
            code: Some("1".to_owned()),
        },
        Command::Disarm { code: None },
        Command::VacuumAction {
            action: "start".to_owned(),
        },
        Command::SetFanSpeed {
            speed: "Auto".to_owned(),
        },
    ];
    let mut mapped = 0;
    for entity in &entities {
        for command in &commands {
            match map_command(entity, &EntityState::default(), command, &templates) {
                Ok(publishes) => {
                    assert!(
                        !publishes.is_empty(),
                        "{} {command:?} mapped to nothing",
                        entity.unique_id
                    );
                    assert!(publishes.iter().all(|p| !p.retain && !p.topic.is_empty()));
                    mapped += 1;
                }
                Err(CommandError::Template(reason)) => {
                    panic!(
                        "{} {command:?}: template failed: {reason}",
                        entity.unique_id
                    )
                }
                Err(_) => {}
            }
        }
    }
    // 48 switches x 4 on/off commands, 9 covers x 4, 7 lights x 3, 4 climates x 3, ... : 305.
    assert!(mapped >= 300, "mapped: {mapped}");
}

fn generic(component: &str, config: Value) -> Entity {
    common::entity(component, config)
}

fn control(name: &str, value: Option<Value>) -> Command {
    Command::Control {
        control: name.to_owned(),
        value,
    }
}

#[test]
fn lock_control_publishes_lock_and_unlock_words() {
    let lock = generic(
        "lock",
        json!({"name": "Door", "unique_id": "l1", "state_topic": "l/state", "command_topic": "l/set"}),
    );
    assert_eq!(
        ok(&lock, &control("locked", Some(json!(true)))),
        [publish("l/set", "LOCK")]
    );
    assert_eq!(
        ok(&lock, &control("locked", Some(json!(false)))),
        [publish("l/set", "UNLOCK")]
    );
}

#[test]
fn fan_percentage_out_of_range_is_refused() {
    let fan = generic(
        "fan",
        json!({"name": "Fan", "unique_id": "f1", "state_topic": "f/s", "command_topic": "f/set",
               "percentage_command_topic": "f/pct/set", "percentage_state_topic": "f/pct"}),
    );
    assert_eq!(
        ok(&fan, &control("percentage", Some(json!(60)))),
        [publish("f/pct/set", "60")]
    );
    let e = err(
        &fan,
        &EntityState::default(),
        &control("percentage", Some(json!(150))),
    );
    assert!(matches!(e, CommandError::OutOfRange(_)), "{e:?}");
}

#[test]
fn siren_tone_is_sent_as_json() {
    let siren = generic(
        "siren",
        json!({"name": "Siren", "unique_id": "s1", "state_topic": "s/s", "command_topic": "s/set",
               "available_tones": ["fire", "alarm"]}),
    );
    let publishes = ok(&siren, &control("tone", Some(json!("fire"))));
    assert_eq!(publishes[0].topic, "s/set");
    assert_eq!(json_payload(&publishes[0]), json!({"tone": "fire"}));
    let e = err(
        &siren,
        &EntityState::default(),
        &control("tone", Some(json!("nope"))),
    );
    assert!(matches!(e, CommandError::OutOfRange(_)), "{e:?}");
}

#[test]
fn unannounced_control_is_unsupported() {
    let fan = generic(
        "fan",
        json!({"name": "Fan", "unique_id": "f2", "state_topic": "f/s", "command_topic": "f/set"}),
    );
    for name in ["percentage", "no_such_control"] {
        let e = err(
            &fan,
            &EntityState::default(),
            &control(name, Some(json!(10))),
        );
        assert!(matches!(e, CommandError::Unsupported { .. }), "{e:?}");
    }
}

#[test]
fn press_with_value_is_refused() {
    let scene = generic(
        "scene",
        json!({"name": "Movie", "unique_id": "sc1", "command_topic": "sc/set"}),
    );
    assert_eq!(
        ok(&scene, &control("activate", None)),
        [publish("sc/set", "ON")]
    );
    let e = err(
        &scene,
        &EntityState::default(),
        &control("activate", Some(json!(true))),
    );
    assert!(matches!(e, CommandError::OutOfRange(_)), "{e:?}");
}

#[test]
fn text_longer_than_max_is_refused() {
    let text = generic(
        "text",
        json!({"name": "Msg", "unique_id": "t1", "state_topic": "t/s", "command_topic": "t/set",
               "max": 5}),
    );
    assert_eq!(
        ok(&text, &control("value", Some(json!("hello")))),
        [publish("t/set", "hello")]
    );
    let e = err(
        &text,
        &EntityState::default(),
        &control("value", Some(json!("too long"))),
    );
    assert!(matches!(e, CommandError::OutOfRange(_)), "{e:?}");
}

#[test]
fn inverted_switch_sends_the_opposite_word() {
    // A relay wired the other way round: `invert` swaps payload_on and payload_off, and a
    // toggle reads the (already inverted) state.
    let mut relay = common::parse_fixture("switch__0x847127fffe24e322__switch");
    relay.ui.invert = true;
    let topic = "zigbee2mqtt/rooms/stair/light/set";
    assert_eq!(
        ok(&relay, &turn_on(None, None, None)),
        vec![publish(topic, "OFF")]
    );
    assert_eq!(ok(&relay, &Command::TurnOff), vec![publish(topic, "ON")]);
    let on = with_value(StateValue::OnOff(true));
    assert_eq!(
        map(&relay, &on, &Command::Toggle).expect("toggle"),
        vec![publish(topic, "ON")]
    );
}
