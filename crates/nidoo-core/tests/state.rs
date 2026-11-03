//! State mapping against the recorded house fixtures and hand-written configs.

mod common;

use std::collections::BTreeMap;

use nidoo_core::model::{
    Availability, ClimateState, CoverState, Entity, EntityState, GenericValue, Hs, LightState,
    StateValue, VacuumState, WaterHeaterState,
};
use nidoo_core::state::color::{rgb_to_hs, xy_to_hs};
use nidoo_core::state::{Applied, StateError, apply_message};
use nidoo_core::template::{TemplateError, Templates};
use serde_json::{Value, json};

const AT: u64 = 1_700_000_000;

/// Every recorded device state as `(topic, payload)`.
fn all_states() -> Vec<(String, String)> {
    let dir = common::fixture_dir().join("states");
    let mut states = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("states fixture dir") {
        let path = entry.expect("dir entry").path();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("fixture stem")
            .to_owned();
        let topic = format!("zigbee2mqtt/{}", stem.replace("__", "/"));
        states.push((topic, std::fs::read_to_string(&path).expect("fixture")));
    }
    states
}

fn state_fixture(name: &str) -> (String, String) {
    (
        format!("zigbee2mqtt/{}", name.replace("__", "/")),
        common::fixture(&format!("states/{name}.json")),
    )
}

fn apply(entity: &Entity, state: &mut EntityState, topic: &str, payload: &str) -> Applied {
    apply_message(entity, state, topic, payload, AT, &Templates::new())
        .unwrap_or_else(|e| panic!("{} on {topic}: {e}", entity.unique_id))
}

#[test]
fn recorded_states_apply_to_every_entity() {
    let templates = Templates::new();
    let entities = common::all_entities();
    let states = all_states();
    assert!(entities.len() > 400, "entities: {}", entities.len());
    assert_eq!(states.len(), 64);
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut errors = Vec::new();
    for (topic, payload) in &states {
        for entity in &entities {
            let bound = entity.bindings.topics().contains(topic);
            let mut state = EntityState::default();
            match apply_message(entity, &mut state, topic, payload, AT, &templates) {
                Ok(Applied::Changed) => {
                    assert!(bound, "{} changed on unbound {topic}", entity.unique_id);
                    assert_eq!(state.last_update, Some(AT));
                    *counts.entry("changed").or_default() += 1;
                }
                Ok(Applied::Unchanged) => {
                    assert!(bound, "{} unchanged on unbound {topic}", entity.unique_id);
                    assert_eq!(state, EntityState::default());
                    *counts.entry("unchanged").or_default() += 1;
                }
                Ok(Applied::NotForThisEntity) => {
                    assert!(!bound, "{} ignored bound {topic}", entity.unique_id);
                    assert_eq!(state, EntityState::default());
                    *counts.entry("not_for_this_entity").or_default() += 1;
                }
                Err(e) => errors.push(format!("{} on {topic}: {e}", entity.unique_id)),
            }
        }
    }
    eprintln!("state mapping on fixtures: {counts:?}");
    assert!(errors.is_empty(), "errors: {errors:#?}");
    let changed = counts.get("changed").copied().unwrap_or_default();
    assert!(changed >= 300, "changed: {changed}");
}

#[test]
fn thermometer_temperature_is_number_with_unit() {
    let entity = common::parse_fixture("sensor__0x5c0272fffebbe0c4__temperature");
    let (topic, payload) = state_fixture("rooms__bedroom1__thermometer");
    let mut state = EntityState::default();
    assert_eq!(
        apply(&entity, &mut state, &topic, &payload),
        Applied::Changed
    );
    assert_eq!(state.value, StateValue::Number(18.4));
    assert_eq!(entity.unit.as_deref(), Some("°C"));
    assert_eq!(state.last_update, Some(AT));
}

#[test]
fn contact_sensor_maps_to_on_off() {
    let entity = common::parse_fixture("binary_sensor__0x00158d00054266a8__contact");
    let config = &entity.bindings.config;
    // The Z2M door sensor publishes `contact`: false means open, which HA shows as ON.
    assert_eq!(config["payload_on"], json!(false));
    assert_eq!(config["payload_off"], json!(true));
    assert_eq!(
        config["value_template"],
        json!("{{ value_json[\"contact\"] }}")
    );

    // The recorded payload has contact=true (door closed): what the fixture implies is OFF.
    let (topic, payload) = state_fixture("rooms__living__door_north");
    let recorded: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(recorded["contact"], json!(true));
    let mut state = EntityState::default();
    assert_eq!(
        apply(&entity, &mut state, &topic, &payload),
        Applied::Changed
    );
    assert_eq!(state.value, StateValue::OnOff(false));

    // contact=false (door open) => ON.
    assert_eq!(
        apply(
            &entity,
            &mut state,
            &topic,
            r#"{"contact":false,"battery":100}"#
        ),
        Applied::Changed
    );
    assert_eq!(state.value, StateValue::OnOff(true));

    // A value that matches neither payload is ignored.
    assert_eq!(
        apply(&entity, &mut state, &topic, r#"{"contact":"maybe"}"#),
        Applied::Unchanged
    );
    assert_eq!(state.value, StateValue::OnOff(true));
}

#[test]
fn json_light_state() {
    let light = common::entity(
        "light",
        json!({
            "unique_id": "l1",
            "name": "L1",
            "schema": "json",
            "state_topic": "test/light",
            "command_topic": "test/light/set",
            "brightness": true,
            "supported_color_modes": ["hs"],
            "effect": true,
            "effect_list": ["blink"]
        }),
    );
    let mut state = EntityState::default();
    let payload = r#"{"state":"ON","brightness":127,"color":{"h":30,"s":40},"effect":"blink"}"#;
    assert_eq!(
        apply(&light, &mut state, "test/light", payload),
        Applied::Changed
    );
    assert_eq!(
        state.value,
        StateValue::Light(LightState {
            on: true,
            brightness: Some(127),
            hs: Some(Hs { h: 30.0, s: 40.0 }),
            color_temp: None,
            effect: Some("blink".into()),
        })
    );

    // A partial update keeps the other fields.
    assert_eq!(
        apply(&light, &mut state, "test/light", r#"{"state":"OFF"}"#),
        Applied::Changed
    );
    assert_eq!(
        state.value,
        StateValue::Light(LightState {
            on: false,
            brightness: Some(127),
            hs: Some(Hs { h: 30.0, s: 40.0 }),
            color_temp: None,
            effect: Some("blink".into()),
        })
    );

    // Z2M scale 254 and xy colour, on a recorded light.
    let leds = common::parse_fixture("light__0x1051dbfffe1a4058__light");
    let (topic, payload) = state_fixture("rooms__bedroom1__leds");
    let mut state = EntityState::default();
    assert_eq!(apply(&leds, &mut state, &topic, &payload), Applied::Changed);
    let StateValue::Light(light) = &state.value else {
        panic!("not a light: {:?}", state.value);
    };
    assert!(!light.on);
    assert_eq!(light.brightness, Some(0));
    assert_eq!(light.effect.as_deref(), Some("solid"));
    // xy (0.3688, 0.3684) is a warm white: a yellowish hue with low saturation.
    let hs = light.hs.expect("xy colour converted");
    assert!(hs.s < 30.0, "warm white should have low saturation: {hs:?}");
    assert!((30.0..=60.0).contains(&hs.h), "warm white hue: {hs:?}");
    assert_eq!(
        apply(
            &leds,
            &mut state,
            &topic,
            r#"{"state":"ON","brightness":254}"#
        ),
        Applied::Changed
    );
    let StateValue::Light(light) = &state.value else {
        panic!("not a light: {:?}", state.value);
    };
    assert!(light.on);
    assert_eq!(light.brightness, Some(255));

    // color_mode "color_temp" is exclusive: the stored hs is cleared, color_temp kept.
    assert_eq!(
        apply(
            &leds,
            &mut state,
            &topic,
            r#"{"color_mode":"color_temp","color_temp":370,"color":{"x":0.5,"y":0.4}}"#
        ),
        Applied::Changed
    );
    let StateValue::Light(light) = &state.value else {
        panic!("not a light: {:?}", state.value);
    };
    assert_eq!(light.hs, None);
    assert_eq!(light.color_temp, Some(370));

    // brightness_scale 0 falls back to 255 instead of dividing by zero.
    let zero_scale = common::entity(
        "light",
        json!({
            "unique_id": "l0",
            "name": "L0",
            "schema": "json",
            "state_topic": "test/l0",
            "command_topic": "test/l0/set",
            "brightness": true,
            "brightness_scale": 0
        }),
    );
    let mut state = EntityState::default();
    assert_eq!(
        apply(
            &zero_scale,
            &mut state,
            "test/l0",
            r#"{"state":"ON","brightness":51}"#
        ),
        Applied::Changed
    );
    let StateValue::Light(light) = &state.value else {
        panic!("not a light: {:?}", state.value);
    };
    assert_eq!(light.brightness, Some(51));
}

#[test]
fn default_schema_light_rgb_to_hs() {
    let light = common::entity(
        "light",
        json!({
            "unique_id": "l2",
            "name": "L2",
            "state_topic": "test/l2/state",
            "command_topic": "test/l2/set",
            "brightness_state_topic": "test/l2/brightness",
            "brightness_scale": 100,
            "rgb_state_topic": "test/l2/rgb",
            "color_temp_state_topic": "test/l2/ct",
            "effect_state_topic": "test/l2/effect",
            "effect_list": ["rainbow"]
        }),
    );
    let mut state = EntityState::default();
    assert_eq!(
        apply(&light, &mut state, "test/l2/state", "ON"),
        Applied::Changed
    );
    assert_eq!(
        apply(&light, &mut state, "test/l2/brightness", "100"),
        Applied::Changed
    );
    assert_eq!(
        apply(&light, &mut state, "test/l2/rgb", "255,0,0"),
        Applied::Changed
    );
    assert_eq!(
        apply(&light, &mut state, "test/l2/ct", "370"),
        Applied::Changed
    );
    assert_eq!(
        apply(&light, &mut state, "test/l2/effect", "rainbow"),
        Applied::Changed
    );
    assert_eq!(
        state.value,
        StateValue::Light(LightState {
            on: true,
            brightness: Some(255),
            hs: Some(Hs { h: 0.0, s: 100.0 }),
            color_temp: Some(370),
            effect: Some("rainbow".into()),
        })
    );
    // Half scale rounds to the 0..=255 range.
    assert_eq!(
        apply(&light, &mut state, "test/l2/brightness", "50"),
        Applied::Changed
    );
    let StateValue::Light(l) = &state.value else {
        panic!("not a light: {:?}", state.value);
    };
    assert_eq!(l.brightness, Some(128));
    // Unparseable brightness is ignored.
    assert_eq!(
        apply(&light, &mut state, "test/l2/brightness", "bright"),
        Applied::Unchanged
    );
    // Unknown on/off payload is ignored.
    assert_eq!(
        apply(&light, &mut state, "test/l2/state", "MAYBE"),
        Applied::Unchanged
    );

    // Templated default-schema fields on one shared topic (Philips TV ambilight).
    let philips = common::parse_fixture("light__philips__philips_tv_philips_ambilight");
    let mut state = EntityState::default();
    assert_eq!(
        apply(
            &philips,
            &mut state,
            "philips/status",
            r#"{"ambilight_power":"On","ambilight_brightness":5,"ambilight_mode":"FOLLOW_VIDEO"}"#
        ),
        Applied::Changed
    );
    assert_eq!(
        state.value,
        StateValue::Light(LightState {
            on: true,
            brightness: Some(128),
            hs: None,
            color_temp: None,
            effect: Some("FOLLOW_VIDEO".into()),
        })
    );
}

#[test]
fn color_conversions() {
    assert_eq!(rgb_to_hs(255, 0, 0), Hs { h: 0.0, s: 100.0 });
    assert_eq!(rgb_to_hs(0, 255, 0), Hs { h: 120.0, s: 100.0 });
    assert_eq!(rgb_to_hs(0, 0, 255), Hs { h: 240.0, s: 100.0 });
    assert_eq!(rgb_to_hs(0, 0, 0), Hs { h: 0.0, s: 0.0 });
    assert_eq!(rgb_to_hs(255, 255, 255), Hs { h: 0.0, s: 0.0 });
    let grey = rgb_to_hs(128, 128, 128);
    assert_eq!(grey.s, 0.0);

    // Reference values from Home Assistant's `color_xy_to_RGB` (wide RGB D65 matrix):
    // D65 white (0.3127, 0.3290) -> (245, 254, 255); sRGB red (0.64, 0.33) -> (255, 92, 47);
    // (0.3, 0.6) -> (156, 255, 68). Rounding instead of truncation moves a channel by one.
    let white = xy_to_hs(0.3127, 0.3290);
    assert!(white.s < 5.0, "{white:?}");
    let red = xy_to_hs(0.64, 0.33);
    assert!((red.h - 13.0).abs() < 1.0, "{red:?}");
    assert!((red.s - 81.5).abs() < 1.0, "{red:?}");
    let green = xy_to_hs(0.3, 0.6);
    assert!((green.h - 92.0).abs() < 1.0, "{green:?}");
    assert!((green.s - 73.5).abs() < 1.0, "{green:?}");
    // Degenerate input does not panic.
    let degenerate = xy_to_hs(0.0, 0.0);
    assert!(degenerate.h.is_finite() && degenerate.s.is_finite());
}

#[test]
fn cover_state_and_position() {
    let cover = common::entity(
        "cover",
        json!({
            "unique_id": "c1",
            "name": "C1",
            "state_topic": "test/cover/state",
            "position_topic": "test/cover/position",
            "command_topic": "test/cover/set"
        }),
    );
    let mut state = EntityState::default();
    assert_eq!(
        apply(&cover, &mut state, "test/cover/state", "open"),
        Applied::Changed
    );
    assert_eq!(
        apply(&cover, &mut state, "test/cover/position", "40"),
        Applied::Changed
    );
    assert_eq!(
        state.value,
        StateValue::Cover(CoverState {
            state: Some("open".into()),
            position: Some(40),
        })
    );
    assert_eq!(
        apply(&cover, &mut state, "test/cover/state", "closing"),
        Applied::Changed
    );
    assert_eq!(
        apply(&cover, &mut state, "test/cover/state", "sideways"),
        Applied::Unchanged
    );
    let StateValue::Cover(c) = &state.value else {
        panic!("not a cover: {:?}", state.value);
    };
    assert_eq!(c.state.as_deref(), Some("closing"));

    // Without a position topic, `stopped` keeps a closed cover closed (HA: closed or closing
    // -> closed) and ends an opening move as open.
    let plain = common::entity(
        "cover",
        json!({
            "unique_id": "c2",
            "name": "C2",
            "state_topic": "test/c2/state",
            "command_topic": "test/c2/set"
        }),
    );
    let mut state = EntityState::default();
    let cover_state = |state: &EntityState| match &state.value {
        StateValue::Cover(c) => c.state.clone(),
        other => panic!("not a cover: {other:?}"),
    };
    assert_eq!(
        apply(&plain, &mut state, "test/c2/state", "closed"),
        Applied::Changed
    );
    assert_eq!(
        apply(&plain, &mut state, "test/c2/state", "stopped"),
        Applied::Changed
    );
    assert_eq!(cover_state(&state).as_deref(), Some("closed"));
    assert_eq!(
        apply(&plain, &mut state, "test/c2/state", "closing"),
        Applied::Changed
    );
    assert_eq!(
        apply(&plain, &mut state, "test/c2/state", "stopped"),
        Applied::Changed
    );
    assert_eq!(cover_state(&state).as_deref(), Some("closed"));
    assert_eq!(
        apply(&plain, &mut state, "test/c2/state", "opening"),
        Applied::Changed
    );
    assert_eq!(
        apply(&plain, &mut state, "test/c2/state", "stopped"),
        Applied::Changed
    );
    assert_eq!(cover_state(&state).as_deref(), Some("open"));

    // Inverted range: position_open 0, position_closed 100 -> stored so that 100 is open.
    let inverted = common::entity(
        "cover",
        json!({
            "unique_id": "c3",
            "name": "C3",
            "position_topic": "test/c3/position",
            "position_open": 0,
            "position_closed": 100,
            "command_topic": "test/c3/set"
        }),
    );
    let mut state = EntityState::default();
    assert_eq!(
        apply(&inverted, &mut state, "test/c3/position", "0"),
        Applied::Changed
    );
    assert_eq!(
        state.value,
        StateValue::Cover(CoverState {
            state: Some("open".into()),
            position: Some(100),
        })
    );
    assert_eq!(
        apply(&inverted, &mut state, "test/c3/position", "75"),
        Applied::Changed
    );
    assert_eq!(
        state.value,
        StateValue::Cover(CoverState {
            state: Some("open".into()),
            position: Some(25),
        })
    );
    assert_eq!(
        apply(&inverted, &mut state, "test/c3/position", "100"),
        Applied::Changed
    );
    assert_eq!(
        state.value,
        StateValue::Cover(CoverState {
            state: Some("closed".into()),
            position: Some(0),
        })
    );

    // Recorded Z2M shutter: state and position share one topic and use custom words.
    let shutter = common::parse_fixture("cover__0x000d6ffffe9d44e4__cover");
    let (topic, payload) = state_fixture("rooms__office__shutter");
    let mut state = EntityState::default();
    assert_eq!(
        apply(&shutter, &mut state, &topic, &payload),
        Applied::Changed
    );
    assert_eq!(
        state.value,
        StateValue::Cover(CoverState {
            state: Some("closed".into()),
            position: Some(0),
        })
    );
    assert_eq!(
        apply(
            &shutter,
            &mut state,
            &topic,
            r#"{"moving":"UP","position":30,"state":"OPEN"}"#
        ),
        Applied::Changed
    );
    assert_eq!(
        state.value,
        StateValue::Cover(CoverState {
            state: Some("opening".into()),
            position: Some(30),
        })
    );
}

#[test]
fn climate_fields() {
    let climate = common::parse_fixture("climate__daikin_192_0_2_150__climate");
    let topic = "daikin/192.0.2.150/ha/state";
    let mut state = EntityState::default();
    assert_eq!(
        apply(
            &climate,
            &mut state,
            topic,
            r#"{"current_temperature":20.5,"target_temperature":21,"mode":"heat","fan_mode":"auto","swing_mode":"off"}"#
        ),
        Applied::Changed
    );
    assert_eq!(
        state.value,
        StateValue::Climate(ClimateState {
            current: Some(20.5),
            target: Some(21.0),
            mode: Some("heat".into()),
            fan_mode: Some("auto".into()),
            action: None,
        })
    );
    // Availability through the list form without a template, non-JSON payload.
    assert_eq!(
        apply(
            &climate,
            &mut state,
            "daikin/bridge/availability",
            "offline"
        ),
        Applied::Changed
    );
    assert_eq!(state.availability, Availability::Offline);

    let heater = common::entity(
        "water_heater",
        json!({
            "unique_id": "wh",
            "name": "WH",
            "mode_state_topic": "wh/mode",
            "current_temperature_topic": "wh/current",
            "temperature_state_topic": "wh/target",
            "temperature_state_template": "{{ value_json.t }}",
            "mode_command_topic": "wh/mode/set"
        }),
    );
    let mut state = EntityState::default();
    assert_eq!(
        apply(&heater, &mut state, "wh/mode", "eco"),
        Applied::Changed
    );
    assert_eq!(
        apply(&heater, &mut state, "wh/current", "48.5"),
        Applied::Changed
    );
    assert_eq!(
        apply(&heater, &mut state, "wh/target", r#"{"t": 55}"#),
        Applied::Changed
    );
    assert_eq!(
        state.value,
        StateValue::WaterHeater(WaterHeaterState {
            mode: Some("eco".into()),
            current: Some(48.5),
            target: Some(55.0),
        })
    );
}

#[test]
fn vacuum_and_alarm_states() {
    let vacuum = common::parse_fixture("vacuum__roborock_40313ca8b964__roborock_40313ca8b964");
    let mut state = EntityState::default();
    assert_eq!(
        apply(
            &vacuum,
            &mut state,
            "roborock/ha/state",
            r#"{"state":"cleaning","battery_level":61,"fan_speed":"Turbo"}"#
        ),
        Applied::Changed
    );
    assert_eq!(
        state.value,
        StateValue::Vacuum(VacuumState {
            state: Some("cleaning".into()),
            battery: Some(61),
            fan_speed: Some("Turbo".into()),
        })
    );
    assert_eq!(
        apply(&vacuum, &mut state, "roborock/ha/state", "not json"),
        Applied::Unchanged
    );
    assert_eq!(
        apply(&vacuum, &mut state, "roborock/alive", "0"),
        Applied::Changed
    );
    assert_eq!(state.availability, Availability::Offline);
    assert_eq!(
        apply(&vacuum, &mut state, "roborock/alive", "1"),
        Applied::Changed
    );
    assert_eq!(state.availability, Availability::Online);

    let alarm = common::parse_fixture("alarm_control_panel__jeedom__house_alarm");
    let mut state = EntityState::default();
    assert_eq!(
        apply(&alarm, &mut state, "jeedom/alarm/state", "armed_home"),
        Applied::Changed
    );
    assert_eq!(state.value, StateValue::Alarm("armed_home".into()));
    assert_eq!(
        apply(&alarm, &mut state, "jeedom/alarm/state", "exploded"),
        Applied::Unchanged
    );
    assert_eq!(state.value, StateValue::Alarm("armed_home".into()));
}

#[test]
fn switch_number_and_select_from_recorded_devices() {
    let siren = common::parse_fixture("switch__0xbc33acfffe36db49__switch_alarm");
    let (topic, payload) = state_fixture("alarm__siren_living");
    let mut state = EntityState::default();
    assert_eq!(
        apply(&siren, &mut state, &topic, &payload),
        Applied::Changed
    );
    assert_eq!(state.value, StateValue::OnOff(false));
    assert_eq!(
        apply(&siren, &mut state, &topic, r#"{"alarm":true}"#),
        Applied::Changed
    );
    assert_eq!(state.value, StateValue::OnOff(true));

    let count = common::parse_fixture("number__0x1051dbfffe1a4058__active_count");
    let effect = common::parse_fixture("select__0x1051dbfffe1a4058__effect");
    let (topic, payload) = state_fixture("rooms__bedroom1__leds");
    let mut state = EntityState::default();
    assert_eq!(
        apply(&count, &mut state, &topic, &payload),
        Applied::Changed
    );
    assert_eq!(state.value, StateValue::Number(173.0));
    let mut state = EntityState::default();
    assert_eq!(
        apply(&effect, &mut state, &topic, &payload),
        Applied::Changed
    );
    assert_eq!(state.value, StateValue::Text("solid".into()));

    // A null option renders as none: the value becomes unknown, like HA.
    let indicator = common::parse_fixture("select__0x000d6ffffe9d44e4__indicator_mode");
    let mut state = EntityState {
        value: StateValue::Text("on".into()),
        ..EntityState::default()
    };
    assert_eq!(
        apply(
            &indicator,
            &mut state,
            "zigbee2mqtt/rooms/office/shutter",
            r#"{"indicator_mode":null}"#
        ),
        Applied::Changed
    );
    assert_eq!(state.value, StateValue::Unknown);
}

#[test]
fn availability_from_bridge_state() {
    let entity = common::parse_fixture("sensor__0x5c0272fffebbe0c4__temperature");
    let mut state = EntityState::default();
    assert_eq!(
        apply(
            &entity,
            &mut state,
            "zigbee2mqtt/bridge/state",
            r#"{"state":"offline"}"#
        ),
        Applied::Changed
    );
    assert_eq!(state.availability, Availability::Offline);
    assert_eq!(state.value, StateValue::Unknown);
    // Same availability again: nothing changed.
    assert_eq!(
        apply(
            &entity,
            &mut state,
            "zigbee2mqtt/bridge/state",
            r#"{"state":"offline"}"#
        ),
        Applied::Unchanged
    );
    assert_eq!(
        apply(
            &entity,
            &mut state,
            "zigbee2mqtt/bridge/state",
            r#"{"state":"online"}"#
        ),
        Applied::Changed
    );
    assert_eq!(state.availability, Availability::Online);
    // A payload the template cannot read, or an unknown word, leaves availability alone.
    assert_eq!(
        apply(&entity, &mut state, "zigbee2mqtt/bridge/state", "offline"),
        Applied::Unchanged
    );
    assert_eq!(
        apply(
            &entity,
            &mut state,
            "zigbee2mqtt/bridge/state",
            r#"{"state":"sleeping"}"#
        ),
        Applied::Unchanged
    );
    assert_eq!(state.availability, Availability::Online);
}

#[test]
fn availability_modes_with_several_topics() {
    let config = |mode: &str| {
        json!({
            "unique_id": format!("s_{mode}"),
            "name": "S",
            "state_topic": "dev/state",
            "availability_mode": mode,
            "availability": [
                {"topic": "bridge/state", "value_template": "{{ value_json.state }}"},
                {"topic": "dev/availability", "payload_available": "yes", "payload_not_available": "no"}
            ]
        })
    };

    let all = common::entity("sensor", config("all"));
    let mut state = EntityState::default();
    // The device topic has not reported yet: not every topic is online.
    assert_eq!(
        apply(&all, &mut state, "bridge/state", r#"{"state":"online"}"#),
        Applied::Changed
    );
    assert_eq!(state.availability, Availability::Offline);
    assert_eq!(
        apply(&all, &mut state, "dev/availability", "yes"),
        Applied::Changed
    );
    assert_eq!(state.availability, Availability::Online);
    assert_eq!(
        apply(&all, &mut state, "bridge/state", r#"{"state":"offline"}"#),
        Applied::Changed
    );
    assert_eq!(state.availability, Availability::Offline);
    // The per-topic record is kept in a reserved attribute.
    assert_eq!(
        state.attributes["__availability"],
        json!({"bridge/state": "offline", "dev/availability": "online"})
    );

    let any = common::entity("sensor", config("any"));
    let mut state = EntityState::default();
    assert_eq!(
        apply(&any, &mut state, "dev/availability", "no"),
        Applied::Changed
    );
    assert_eq!(state.availability, Availability::Offline);
    assert_eq!(
        apply(&any, &mut state, "bridge/state", r#"{"state":"online"}"#),
        Applied::Changed
    );
    assert_eq!(state.availability, Availability::Online);
    assert_eq!(
        apply(&any, &mut state, "bridge/state", r#"{"state":"offline"}"#),
        Applied::Changed
    );
    assert_eq!(state.availability, Availability::Offline);

    let latest = common::entity("sensor", config("latest"));
    let mut state = EntityState::default();
    assert_eq!(
        apply(&latest, &mut state, "dev/availability", "no"),
        Applied::Changed
    );
    assert_eq!(state.availability, Availability::Offline);
    assert_eq!(
        apply(&latest, &mut state, "bridge/state", r#"{"state":"online"}"#),
        Applied::Changed
    );
    assert_eq!(state.availability, Availability::Online);
}

#[test]
fn missing_key_on_shared_topic_leaves_value_unchanged() {
    let entity = common::parse_fixture("sensor__0x5c0272fffebbe0c4__temperature");
    let mut state = EntityState {
        value: StateValue::Number(1.0),
        ..EntityState::default()
    };
    assert_eq!(
        apply(
            &entity,
            &mut state,
            "zigbee2mqtt/rooms/bedroom1/thermometer",
            r#"{"other":1}"#
        ),
        Applied::Unchanged
    );
    assert_eq!(state.value, StateValue::Number(1.0));
    assert_eq!(state.last_update, None);
    // A scalar payload has no `value_json` at all: same outcome.
    assert_eq!(
        apply(
            &entity,
            &mut state,
            "zigbee2mqtt/rooms/bedroom1/thermometer",
            "18.4"
        ),
        Applied::Unchanged
    );
    assert_eq!(state.value, StateValue::Number(1.0));
}

#[test]
fn attributes_merge() {
    // state_topic == json_attributes_topic: one message feeds both.
    let front = common::parse_fixture("sensor__double-take__front");
    let topic = "homeassistant/sensor/double-take/front/state";
    let mut state = EntityState::default();
    assert_eq!(
        apply(
            &front,
            &mut state,
            topic,
            r#"{"counts":{"person":2},"camera":"front"}"#
        ),
        Applied::Changed
    );
    assert_eq!(state.value, StateValue::Number(2.0));
    assert_eq!(state.attributes["counts"], json!({"person": 2}));
    assert_eq!(state.attributes["camera"], json!("front"));
    // Shallow merge: keys overwrite, others stay.
    assert_eq!(
        apply(
            &front,
            &mut state,
            topic,
            r#"{"counts":{"person":0},"zone":"door"}"#
        ),
        Applied::Changed
    );
    assert_eq!(state.value, StateValue::Number(0.0));
    assert_eq!(state.attributes["counts"], json!({"person": 0}));
    assert_eq!(state.attributes["camera"], json!("front"));
    assert_eq!(state.attributes["zone"], json!("door"));
    // Not an object: ignored.
    assert_eq!(
        apply(&front, &mut state, topic, r#"[1,2]"#),
        Applied::Unchanged
    );
    assert_eq!(state.attributes.len(), 3);

    // A templated attributes payload on its own topic.
    let templated = common::entity(
        "switch",
        json!({
            "unique_id": "sw",
            "name": "SW",
            "state_topic": "sw/state",
            "command_topic": "sw/set",
            "json_attributes_topic": "sw/attrs",
            "json_attributes_template": "{{ value_json.data | tojson }}"
        }),
    );
    let mut state = EntityState::default();
    assert_eq!(
        apply(
            &templated,
            &mut state,
            "sw/attrs",
            r#"{"data":{"lqi":12,"battery":90}}"#
        ),
        Applied::Changed
    );
    assert_eq!(state.attributes["lqi"], json!(12));
    assert_eq!(state.attributes["battery"], json!(90));
    assert_eq!(state.value, StateValue::Unknown);
    assert_eq!(
        apply(&templated, &mut state, "sw/attrs", r#"{"nope":1}"#),
        Applied::Unchanged
    );
}

#[test]
fn message_for_unrelated_topic() {
    let entity = common::parse_fixture("sensor__0x5c0272fffebbe0c4__temperature");
    let mut state = EntityState::default();
    assert_eq!(
        apply(
            &entity,
            &mut state,
            "zigbee2mqtt/rooms/bath/thermometer",
            r#"{"temperature":1}"#
        ),
        Applied::NotForThisEntity
    );
    assert_eq!(state, EntityState::default());
    // Command topics are not state topics either.
    let switch = common::parse_fixture("switch__0xbc33acfffe36db49__switch_alarm");
    assert_eq!(
        apply(
            &switch,
            &mut state,
            "zigbee2mqtt/alarm/siren_living/set/alarm",
            "ON"
        ),
        Applied::NotForThisEntity
    );
}

#[test]
fn button_never_changes_state() {
    let button = common::parse_fixture("button__0x00158d0005422e73__identify");
    let mut state = EntityState::default();
    assert_eq!(
        apply(
            &button,
            &mut state,
            "zigbee2mqtt/rooms/living/door_entrance",
            r#"{"identify":null}"#
        ),
        Applied::NotForThisEntity
    );
    assert_eq!(
        apply(
            &button,
            &mut state,
            "zigbee2mqtt/bridge/state",
            r#"{"state":"online"}"#
        ),
        Applied::Changed
    );
    assert_eq!(state.availability, Availability::Online);
    assert_eq!(state.value, StateValue::Unknown);
}

#[test]
fn template_syntax_error_is_reported() {
    // `now()` does not exist in the engine: a permanent template error.
    let network_map = common::parse_fixture(
        "sensor__1221051039810110150109113116116_0x00124b0022983b3e__network_map",
    );
    let mut state = EntityState::default();
    let err = apply_message(
        &network_map,
        &mut state,
        "zigbee2mqtt/bridge/response/networkmap",
        r#"{"data":{"value":{"nodes":[]}}}"#,
        AT,
        &Templates::new(),
    )
    .unwrap_err();
    assert!(
        matches!(err, StateError::Template(TemplateError::Syntax(_))),
        "{err:?}"
    );
    // Its attributes topic is the state topic: what was read before the failure stays.
    assert_eq!(state.attributes["nodes"], json!([]));
    assert_eq!(state.value, StateValue::Unknown);
    assert_eq!(state.last_update, None);

    let mut state = EntityState::default();
    let broken = common::entity(
        "sensor",
        json!({
            "unique_id": "broken",
            "name": "B",
            "state_topic": "b/state",
            "value_template": "{{ value | nosuchfilter }}"
        }),
    );
    let err =
        apply_message(&broken, &mut state, "b/state", "1", AT, &Templates::new()).unwrap_err();
    assert!(
        matches!(err, StateError::Template(TemplateError::Syntax(_))),
        "{err:?}"
    );
    assert_eq!(state, EntityState::default());
}

#[test]
fn hostile_payloads_never_panic() {
    let templates = Templates::new();
    let payloads = [
        "",
        " ",
        "{",
        "[",
        "null",
        "{\"state\":null}",
        "{\"brightness\":-5,\"color\":{\"h\":\"x\"},\"color_temp\":1e30}",
        "{\"position\":1e308}",
        "{\"battery_level\":300}",
        "\u{0}\u{ffff}",
        "255,0",
        "a,b,c",
        "1e999",
        "NaN",
    ];
    for entity in common::all_entities() {
        for topic in entity.bindings.topics() {
            for payload in payloads {
                let mut state = EntityState::default();
                if let Err(err) =
                    apply_message(&entity, &mut state, &topic, payload, AT, &templates)
                {
                    assert!(
                        matches!(err, StateError::Template(_)),
                        "{} on {topic} with {payload:?}: {err:?}",
                        entity.unique_id
                    );
                }
            }
        }
    }
}

fn generic_fields(state: &EntityState) -> &BTreeMap<String, GenericValue> {
    match &state.value {
        StateValue::Generic(g) => &g.fields,
        other => panic!("not generic: {other:?}"),
    }
}

fn generic_state(entity: &Entity, messages: &[(&str, &str)]) -> EntityState {
    let mut state = EntityState::default();
    for (topic, payload) in messages {
        assert_eq!(apply(entity, &mut state, topic, payload), Applied::Changed);
    }
    state
}

fn lock(config: Value) -> Entity {
    let mut cfg = json!({"name": "Door", "unique_id": "lock1", "state_topic": "l/state"});
    cfg.as_object_mut()
        .unwrap()
        .extend(config.as_object().unwrap().clone());
    common::entity("lock", cfg)
}

#[test]
fn lock_payload_maps_to_canonical_word() {
    let state = generic_state(&lock(json!({})), &[("l/state", "LOCKED")]);
    assert_eq!(
        generic_fields(&state)["state"],
        GenericValue::Text("locked".into())
    );
}

#[test]
fn custom_state_word_is_honoured() {
    let entity = lock(json!({"state_locked": "closed"}));
    let state = generic_state(&entity, &[("l/state", "closed")]);
    assert_eq!(
        generic_fields(&state)["state"],
        GenericValue::Text("locked".into())
    );
}

#[test]
fn unmapped_payload_is_kept_as_raw_text() {
    let state = generic_state(&lock(json!({})), &[("l/state", "FOO")]);
    assert_eq!(
        generic_fields(&state)["state"],
        GenericValue::Text("FOO".into())
    );
    let StateValue::Generic(g) = &state.value else {
        unreachable!()
    };
    assert!(!nidoo_core::types::alerting(
        nidoo_core::types::type_def("lock").unwrap(),
        g
    ));
}

#[test]
fn fan_fields_merge_across_topics() {
    let entity = common::entity(
        "fan",
        json!({"name": "Fan", "unique_id": "fan1", "command_topic": "f/set",
               "state_topic": "f/state", "percentage_state_topic": "f/pct"}),
    );
    let state = generic_state(&entity, &[("f/state", "ON"), ("f/pct", "60")]);
    let fields = generic_fields(&state);
    assert_eq!(fields["state"], GenericValue::Bool(true));
    assert_eq!(fields["percentage"], GenericValue::Number(60.0));
}

#[test]
fn generic_number_non_finite_is_not_stored() {
    let entity = common::entity(
        "fan",
        json!({"name": "Fan", "unique_id": "fan1", "command_topic": "f/set",
               "state_topic": "f/state", "percentage_state_topic": "f/pct"}),
    );
    let mut state = generic_state(&entity, &[("f/pct", "60")]);
    assert_eq!(
        apply(&entity, &mut state, "f/pct", "inf"),
        Applied::Unchanged
    );
    assert_eq!(
        generic_fields(&state)["percentage"],
        GenericValue::Number(60.0)
    );
    assert_eq!(
        apply(&entity, &mut state, "f/pct", "None"),
        Applied::Changed
    );
    assert!(!generic_fields(&state).contains_key("percentage"));
    assert_eq!(
        apply(&entity, &mut state, "other", "1"),
        Applied::NotForThisEntity
    );
}

#[test]
fn alerting_lock_when_unlocked_or_jammed() {
    use nidoo_core::types::{alerting, type_def};
    let def = type_def("lock").unwrap();
    let entity = lock(json!({}));
    for (payload, expected) in [("LOCKED", false), ("UNLOCKED", true), ("JAMMED", true)] {
        let state = generic_state(&entity, &[("l/state", payload)]);
        let StateValue::Generic(g) = &state.value else {
            unreachable!()
        };
        assert_eq!(alerting(def, g), expected, "{payload}");
    }
}

#[test]
fn history_num_maps_lock_and_tracker() {
    use nidoo_core::types::{history_num, type_def};
    let num = |component: &str, entity: Entity, topic: &str, payload: &str| {
        let state = generic_state(&entity, &[(topic, payload)]);
        let StateValue::Generic(g) = &state.value else {
            unreachable!()
        };
        history_num(type_def(component).unwrap(), g)
    };
    let l = lock(json!({}));
    assert_eq!(num("lock", l.clone(), "l/state", "LOCKED"), Some(1.0));
    assert_eq!(num("lock", l, "l/state", "UNLOCKED"), Some(0.0));
    let tracker = common::entity(
        "device_tracker",
        json!({"name": "T", "unique_id": "t1", "state_topic": "t/state"}),
    );
    assert_eq!(num("device_tracker", tracker, "t/state", "home"), Some(1.0));
    let fan = common::entity(
        "fan",
        json!({"name": "Fan", "unique_id": "fan1", "command_topic": "f/set",
               "percentage_state_topic": "f/pct"}),
    );
    assert_eq!(num("fan", fan, "f/pct", "60"), None);
    let humidifier = common::entity(
        "humidifier",
        json!({"name": "H", "unique_id": "h1", "command_topic": "h/set",
               "target_humidity_command_topic": "h/tset",
               "target_humidity_state_topic": "h/target"}),
    );
    assert_eq!(num("humidifier", humidifier, "h/target", "45"), Some(45.0));
}

#[test]
fn remote_press_is_a_signal() {
    use nidoo_core::types::{signals::signal_of, type_def};
    let mut entity = lock(json!({}));
    entity.component = "device_automation".to_owned();
    entity.bindings.config = json!({"actions": [
        {"object": "action_on", "topic": "z/remote/action", "payload": "on", "type": "action", "subtype": "on"},
        {"object": "action_off", "topic": "z/remote/action", "payload": "off", "type": "action", "subtype": "off"},
    ]});
    let def = type_def("device_automation").unwrap();
    let templates = Templates::new();
    let of = |topic: &str, payload: &[u8]| signal_of(&entity, def, topic, payload, &templates);
    assert_eq!(of("z/remote/action", b"on"), Some("on".to_owned()));
    assert_eq!(of("z/remote/action", b"warp"), None);
    assert_eq!(of("z/other", b"on"), None);
    assert_eq!(of("z/remote/action", &[0xff, 0xfe]), None);
}

#[test]
fn event_and_tag_signals() {
    use nidoo_core::types::{signals::signal_of, type_def};
    let templates = Templates::new();
    let mut event = lock(json!({}));
    event.component = "event".to_owned();
    event.bindings.config = json!({"state_topic": "e/state"});
    let def = type_def("event").unwrap();
    assert_eq!(
        signal_of(
            &event,
            def,
            "e/state",
            br#"{"event_type":"single"}"#,
            &templates
        ),
        Some("single".to_owned())
    );
    assert_eq!(signal_of(&event, def, "e/state", b"nope", &templates), None);
    let mut tag = lock(json!({}));
    tag.component = "tag".to_owned();
    tag.bindings.config = json!({"topic": "t/scan"});
    let def = type_def("tag").unwrap();
    assert_eq!(
        signal_of(&tag, def, "t/scan", b"abc123", &templates),
        Some("abc123".to_owned())
    );
}

#[test]
fn update_json_blob_populates_installed_and_latest() {
    let entity = common::parse_fixture("update__0x5c0272fffea35cfb__update");
    let payload = r#"{"update":{"installed_version":"1.0.0","latest_version":"1.0.1","state":"available"},"linkquality":80}"#;
    let state = generic_state(
        &entity,
        &[("zigbee2mqtt/rooms/bedroom2/thermometer", payload)],
    );
    let fields = generic_fields(&state);
    assert_eq!(fields["installed"], GenericValue::Text("1.0.0".into()));
    assert_eq!(fields["latest"], GenericValue::Text("1.0.1".into()));
}
