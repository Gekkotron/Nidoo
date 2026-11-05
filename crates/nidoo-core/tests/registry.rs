//! The registry against the recorded house: routing, effective entities, folding, state and
//! the subscription set.

mod common;

use std::collections::BTreeSet;

use nidoo_core::discovery::DiscoveryTopic;
use nidoo_core::model::{Availability, EntityCategory, Kind, StateValue, UNASSIGNED_AREA};
use nidoo_core::registry::{Change, Inbound, Registry, RegistryConfig};
use serde_json::json;

const AT: u64 = 1_700_000_000;

const ALARM_SWITCH: &str = "0xbc33acfffe36db49_switch_alarm_zigbee2mqtt";
const STAIR_LIGHT: &str = "0x847127fffe24e322_switch_zigbee2mqtt";
const STAIR_LIGHT_TOPIC: &str = "nidoo/switch/0x847127fffe24e322/switch/config";
const THERMOMETER_TOPIC: &str = "zigbee2mqtt/rooms/bedroom2/thermometer";
const SHUTTER_COVER: &str = "0x847127fffe08106c_cover_zigbee2mqtt";
const SHUTTER_MOVING: &str = "0x847127fffe08106c_moving_zigbee2mqtt";
const LINKQUALITY: &str = "0x5c0272fffea35cfb_linkquality_zigbee2mqtt";
const HOUSE_ALARM: &str = "jeedom_house_alarm";
const HOUSE_ALARM_TOPIC: &str = "nidoo/alarm_control_panel/jeedom/house_alarm/config";

fn config() -> RegistryConfig {
    RegistryConfig {
        discovery_prefixes: common::strings(&["nidoo"]),
        ui_prefix: "nidoo/ui".to_owned(),
        z2m_base: "zigbee2mqtt".to_owned(),
        reserved: common::strings(&["ui", "automation"]),
    }
}

/// Files of a fixture sub-directory as `(stem, bytes)`, sorted by stem.
fn files(sub: &str) -> Vec<(String, Vec<u8>)> {
    let dir = common::fixture_dir().join(sub);
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("fixture dir") {
        let path = entry.expect("dir entry").path();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("fixture stem")
            .to_owned();
        out.push((stem, std::fs::read(&path).expect("fixture")));
    }
    out.sort();
    out
}

fn feed(registry: &mut Registry, topic: &str, payload: Vec<u8>) -> Vec<Change> {
    let inbound = registry.route(topic, payload, AT);
    registry.apply(inbound)
}

/// The whole recorded house: bridge, discovery, overrides and areas, then every state.
fn load_house() -> Registry {
    let mut registry = Registry::new(config());
    for (file, topic) in [
        ("bridge_info.json", "zigbee2mqtt/bridge/info"),
        ("bridge_devices.json", "zigbee2mqtt/bridge/devices"),
        ("bridge_state.json", "zigbee2mqtt/bridge/state"),
    ] {
        feed(&mut registry, topic, common::fixture(file).into_bytes());
    }
    for (stem, payload) in files("discovery") {
        let topic = format!("nidoo/{}/config", stem.replace("__", "/"));
        feed(&mut registry, &topic, payload);
    }
    for (stem, payload) in files("ui") {
        feed(&mut registry, &format!("nidoo/ui/{stem}"), payload);
    }
    for (stem, payload) in files("states") {
        let topic = format!("zigbee2mqtt/{}", stem.replace("__", "/"));
        feed(&mut registry, &topic, payload);
    }
    registry
}

fn discovery_topic(topic: &str) -> DiscoveryTopic {
    common::discovery_topic(topic)
}

fn override_payload(unique_id: &str, payload: &str) -> Inbound {
    Inbound::Override {
        unique_id: unique_id.to_owned(),
        payload: payload.as_bytes().to_vec(),
    }
}

#[test]
fn house_loads() {
    let registry = load_house();
    let entities = registry.entities().count();
    assert!(entities >= 400, "entities: {entities}");
    assert_eq!(registry.devices().count(), 66);
    let areas: Vec<String> = registry
        .effective_areas()
        .iter()
        .take(5)
        .map(|a| a.id.clone())
        .collect();
    assert_eq!(
        areas,
        common::strings(&["living", "bedroom1", "bedroom2", "bedroom3", "office"])
    );
    assert_eq!(registry.bridge().version.as_deref(), Some("2.14.1"));
    assert!(registry.template_failures().is_empty());
    assert!(!registry.subscription_topics().is_empty());
    assert!(registry.subscription_topics().contains(THERMOMETER_TOPIC));
    eprintln!(
        "house: {} entities, {} devices, {} areas, {} ignored, {} template failures, {} subscriptions",
        entities,
        registry.devices().count(),
        registry.effective_areas().len(),
        registry.ignored().len(),
        registry.template_failures().len(),
        registry.subscription_topics().len()
    );
}

#[test]
fn overrides_apply() {
    let registry = load_house();
    let entity = registry.entity(ALARM_SWITCH).expect("alarm switch");
    assert_eq!(entity.area, "living");
    assert!(entity.ui.security_action);
    assert!(registry.raw_override(ALARM_SWITCH).is_some());
}

#[test]
fn path_area_and_light_type() {
    let mut registry = load_house();
    let entity = registry.entity(STAIR_LIGHT).expect("stair light");
    assert_eq!(entity.area, "stair");
    assert_eq!(entity.kind, Kind::Switch);

    let changes = registry.apply(override_payload(STAIR_LIGHT, r#"{"asLight":true}"#));
    assert!(changes.contains(&Change::EntityUpserted(STAIR_LIGHT.to_owned())));
    let entity = registry.entity(STAIR_LIGHT).expect("stair light");
    assert_eq!(entity.kind, Kind::Light);
    assert_eq!(entity.protocol_kind, Kind::Switch);
    assert_eq!(
        entity.ui.show_as.as_deref(),
        Some("light"),
        "legacy asLight reads as Light"
    );

    registry.apply(override_payload(STAIR_LIGHT, r#"{"showAs":"light"}"#));
    assert_eq!(registry.entity(STAIR_LIGHT).unwrap().kind, Kind::Light);
    registry.apply(override_payload(STAIR_LIGHT, r#"{"showAs":"door"}"#));
    assert_eq!(registry.entity(STAIR_LIGHT).unwrap().kind, Kind::Switch);

    let changes = registry.apply(override_payload(STAIR_LIGHT, ""));
    assert!(changes.contains(&Change::EntityUpserted(STAIR_LIGHT.to_owned())));
    let entity = registry.entity(STAIR_LIGHT).expect("stair light");
    assert_eq!(entity.kind, Kind::Switch);
    assert!(entity.ui.is_default());
    assert!(registry.raw_override(STAIR_LIGHT).is_none());
}

#[test]
fn removal_and_subscriptions() {
    let mut registry = load_house();

    // The stair light shares its state topic with its linkquality sensor: no subscription change.
    let before = registry.subscription_topics().clone();
    let changes = registry.apply(Inbound::Discovery(
        discovery_topic(STAIR_LIGHT_TOPIC),
        Vec::new(),
    ));
    assert!(changes.contains(&Change::EntityRemoved(STAIR_LIGHT.to_owned())));
    assert!(!changes.contains(&Change::SubscriptionsChanged));
    assert!(registry.entity(STAIR_LIGHT).is_none());
    assert!(registry.state(STAIR_LIGHT).is_none());
    assert_eq!(registry.subscription_topics(), &before);
    assert!(
        !registry
            .entities_on_topic("zigbee2mqtt/rooms/stair/light")
            .contains(&STAIR_LIGHT.to_owned())
    );

    // The house alarm is alone on its topics: removing it shrinks the subscription set.
    assert!(
        registry
            .subscription_topics()
            .contains("jeedom/alarm/state")
    );
    let changes = registry.apply(Inbound::Discovery(
        discovery_topic(HOUSE_ALARM_TOPIC),
        Vec::new(),
    ));
    assert!(changes.contains(&Change::EntityRemoved(HOUSE_ALARM.to_owned())));
    assert!(changes.contains(&Change::SubscriptionsChanged));
    assert!(
        !registry
            .subscription_topics()
            .contains("jeedom/alarm/state")
    );
    assert!(registry.entities_on_topic("jeedom/alarm/state").is_empty());

    // Removing it again changes nothing.
    let changes = registry.apply(Inbound::Discovery(
        discovery_topic(HOUSE_ALARM_TOPIC),
        Vec::new(),
    ));
    assert!(changes.is_empty(), "{changes:?}");
}

const THERMOMETER_KEYS: [&str; 5] = [
    "temperature",
    "humidity",
    "battery",
    "linkquality",
    "voltage",
];

/// The house already holds the recorded thermometer state, so a re-delivery is a report for
/// each of the five entities on the topic (and still moves `last_update`); a payload where
/// only the temperature differs changes that entity and reports the four others.
#[test]
fn state_routes_to_all_entities_on_shared_topic() {
    let mut registry = load_house();
    let payload = common::fixture("states/rooms__bedroom2__thermometer.json");
    let inbound = registry.route(THERMOMETER_TOPIC, payload.clone().into_bytes(), AT + 1);
    assert!(matches!(inbound, Inbound::State { .. }), "{inbound:?}");
    let changes = registry.apply(inbound);
    for key in THERMOMETER_KEYS {
        let id = format!("0x5c0272fffea35cfb_{key}_zigbee2mqtt");
        assert!(
            changes.contains(&Change::StateReported(id.clone())),
            "{id}: {changes:?}"
        );
        assert!(
            !changes.contains(&Change::StateChanged(id.clone())),
            "{id}: {changes:?}"
        );
        let state = registry.state(&id).expect("state");
        assert!(
            matches!(state.value, StateValue::Number(_)),
            "{id}: {state:?}"
        );
        assert_eq!(state.last_update, Some(AT + 1));
    }
    // The five sensors plus the device's firmware `update` entity, which shares the topic.
    assert_eq!(
        registry.entities_on_topic(THERMOMETER_TOPIC).len(),
        6,
        "{:?}",
        registry.entities_on_topic(THERMOMETER_TOPIC)
    );

    let mut edited: serde_json::Value = serde_json::from_str(&payload).expect("json");
    edited["temperature"] = json!(99.5);
    let inbound = registry.route(THERMOMETER_TOPIC, edited.to_string().into_bytes(), AT + 2);
    let changes = registry.apply(inbound);
    for key in THERMOMETER_KEYS {
        let id = format!("0x5c0272fffea35cfb_{key}_zigbee2mqtt");
        let expected = if key == "temperature" {
            Change::StateChanged(id.clone())
        } else {
            Change::StateReported(id.clone())
        };
        assert!(changes.contains(&expected), "{id}: {changes:?}");
        assert_eq!(
            registry.state(&id).expect("state").last_update,
            Some(AT + 2)
        );
    }
    assert_eq!(
        registry
            .state("0x5c0272fffea35cfb_temperature_zigbee2mqtt")
            .expect("state")
            .value,
        StateValue::Number(99.5)
    );
}

/// A report equal to the current state is `StateReported`, never `StateChanged`: the value,
/// the attributes and the availability are compared, `last_update` alone moving is not a
/// change. An availability flip is a change; the same availability again is nothing at all.
#[test]
fn equal_value_report_is_reported_not_changed() {
    let mut registry = Registry::new(config());
    let config = json!({
        "unique_id": "test_switch",
        "state_topic": "zigbee2mqtt/test",
        "command_topic": "zigbee2mqtt/test/set",
        "availability": [{"topic": "zigbee2mqtt/bridge/state", "value_template": "{{ value_json.state }}"}],
        "device": {"identifiers": ["zigbee2mqtt_0xtest"], "name": "test"}
    });
    registry.apply(Inbound::Discovery(
        discovery_topic("nidoo/switch/test/config"),
        config.to_string().into_bytes(),
    ));
    let state = |payload: &str, at: u64| Inbound::State {
        topic: "zigbee2mqtt/test".to_owned(),
        payload: payload.as_bytes().to_vec(),
        at,
    };

    let changes = registry.apply(state("ON", AT));
    assert_eq!(
        changes,
        vec![Change::StateChanged("test_switch".to_owned())]
    );

    let changes = registry.apply(state("ON", AT + 1));
    assert_eq!(
        changes,
        vec![Change::StateReported("test_switch".to_owned())]
    );
    let current = registry.state("test_switch").expect("state");
    assert_eq!(current.value, StateValue::OnOff(true));
    assert_eq!(current.last_update, Some(AT + 1));

    let changes = registry.apply(state("OFF", AT + 2));
    assert_eq!(
        changes,
        vec![Change::StateChanged("test_switch".to_owned())]
    );

    // A message the mapper cannot read (neither ON nor OFF) is no report: the topic spoke
    // (touched) but `last_update` stays.
    let changes = registry.apply(state("maybe", AT + 3));
    assert_eq!(
        changes,
        vec![Change::StateTouched("test_switch".to_owned())]
    );
    assert_eq!(
        registry.state("test_switch").expect("state").last_update,
        Some(AT + 2)
    );

    let bridge = |payload: &str, at: u64| Inbound::State {
        topic: "zigbee2mqtt/bridge/state".to_owned(),
        payload: payload.as_bytes().to_vec(),
        at,
    };
    let changes = registry.apply(bridge(r#"{"state":"offline"}"#, AT + 4));
    assert!(
        changes.contains(&Change::StateChanged("test_switch".to_owned())),
        "{changes:?}"
    );
    let changes = registry.apply(bridge(r#"{"state":"offline"}"#, AT + 5));
    assert!(
        !changes.contains(&Change::StateChanged("test_switch".to_owned()))
            && !changes.contains(&Change::StateReported("test_switch".to_owned())),
        "{changes:?}"
    );
}

#[test]
fn folding_moving_into_cover() {
    let registry = load_house();
    let moving = registry.entity(SHUTTER_MOVING).expect("moving sensor");
    assert_eq!(moving.part_of.as_deref(), Some(SHUTTER_COVER));
    let cover = registry.entity(SHUTTER_COVER).expect("cover");
    assert_eq!(cover.part_of, None);
    // A device with one light folds its numbers and selects into it.
    let light = "0x1051dbfffe1a4058_light_zigbee2mqtt";
    for id in [
        "0x1051dbfffe1a4058_effect_zigbee2mqtt",
        "0x1051dbfffe1a4058_effect_speed_zigbee2mqtt",
        "0x1051dbfffe1a4058_palette_zigbee2mqtt",
    ] {
        let entity = registry.entity(id).expect("folded entity");
        assert_eq!(entity.part_of.as_deref(), Some(light), "{id}");
    }
    assert_eq!(registry.entity(light).expect("light").part_of, None);
}

#[test]
fn sound_satellite_controls_fold_into_volume() {
    // A satellite (`nidoo-sound`) is one device with a Volume number, Mute and Allowed switches
    // and a Connected sensor: one card, the volume, holds the rest.
    let mut registry = Registry::new(config());
    let entity = |device: &str, object: &str, extra: serde_json::Value| {
        let mut v = json!({
            "name": object,
            "unique_id": format!("{device}_{object}"),
            "state_topic": format!("test/{device}/{object}"),
            "device": { "identifiers": [device], "name": device },
        });
        if let (Some(v), serde_json::Value::Object(extra)) = (v.as_object_mut(), extra) {
            v.extend(extra);
        }
        v.to_string().into_bytes()
    };
    let command = |device: &str, object: &str| json!({ "command_topic": format!("test/{device}/{object}/set") });
    for (component, object, extra) in [
        ("number", "volume", command("nidoo_sound_x", "volume")),
        ("switch", "mute", command("nidoo_sound_x", "mute")),
        ("switch", "allowed", command("nidoo_sound_x", "allowed")),
        (
            "binary_sensor",
            "connected",
            json!({ "device_class": "connectivity" }),
        ),
        (
            "sensor",
            "address",
            json!({ "entity_category": "diagnostic" }),
        ),
    ] {
        feed(
            &mut registry,
            &format!("nidoo/{component}/nidoo_sound_x/{object}/config"),
            entity("nidoo_sound_x", object, extra),
        );
    }
    for id in [
        "nidoo_sound_x_mute",
        "nidoo_sound_x_allowed",
        "nidoo_sound_x_connected",
        "nidoo_sound_x_address",
    ] {
        let folded = registry.entity(id).expect("satellite entity");
        assert_eq!(
            folded.part_of.as_deref(),
            Some("nidoo_sound_x_volume"),
            "{id}"
        );
    }
    let volume = registry.entity("nidoo_sound_x_volume").expect("volume");
    assert_eq!(volume.part_of, None);

    // Any other device with one number and one switch keeps both as cards.
    for (component, object) in [("number", "level"), ("switch", "power")] {
        feed(
            &mut registry,
            &format!("nidoo/{component}/plain_dev/{object}/config"),
            entity("plain_dev", object, command("plain_dev", object)),
        );
    }
    for id in ["plain_dev_level", "plain_dev_power"] {
        assert_eq!(
            registry.entity(id).expect("plain entity").part_of,
            None,
            "{id}"
        );
    }
}

#[test]
fn satellite_with_speech_still_folds() {
    // A satellite now has two numbers, Music (`_volume`, the anchor) and Speech: both the
    // switches and the second number fold into the `_volume` card.
    let mut registry = Registry::new(config());
    for (component, object, extra) in [
        ("number", "speech", Some("speech")),
        ("number", "volume", Some("volume")),
        ("switch", "mute", Some("mute")),
        ("switch", "allowed", Some("allowed")),
        ("binary_sensor", "connected", None),
    ] {
        let mut v = json!({
            "name": object,
            "unique_id": format!("nidoo_sound_x_{object}"),
            "state_topic": format!("test/nidoo_sound_x/{object}"),
            "device": { "identifiers": ["nidoo_sound_x"], "name": "nidoo_sound_x" },
        });
        if let (Some(v), Some(object)) = (v.as_object_mut(), extra) {
            v.insert(
                "command_topic".into(),
                json!(format!("test/nidoo_sound_x/{object}/set")),
            );
        }
        feed(
            &mut registry,
            &format!("nidoo/{component}/nidoo_sound_x/{object}/config"),
            v.to_string().into_bytes(),
        );
    }
    for id in [
        "nidoo_sound_x_speech",
        "nidoo_sound_x_mute",
        "nidoo_sound_x_allowed",
        "nidoo_sound_x_connected",
    ] {
        let folded = registry.entity(id).expect("satellite entity");
        assert_eq!(
            folded.part_of.as_deref(),
            Some("nidoo_sound_x_volume"),
            "{id}"
        );
    }
    let volume = registry.entity("nidoo_sound_x_volume").expect("volume");
    assert_eq!(volume.part_of, None);
}

#[test]
fn groups_never_fold_into_each_other() {
    // Every group is on the hub's one `nidoo_group` device: with a single light group, a number
    // group must stay a card of its own.
    let mut registry = Registry::new(config());
    let group = |id: &str| {
        json!({
            "name": id,
            "unique_id": format!("group_{id}"),
            "device": { "identifiers": ["nidoo_group"], "name": "Groups" },
            "command_topic": format!("nidoo/group/{id}/set"),
        })
    };
    let mut light = group("light_living");
    light["schema"] = json!("json");
    light["state_topic"] = json!("nidoo/group/light_living/state");
    feed(
        &mut registry,
        "nidoo/light/nidoo_group/light_living/config",
        light.to_string().into_bytes(),
    );
    feed(
        &mut registry,
        "nidoo/number/nidoo_group/water_heater_power/config",
        group("water_heater_power").to_string().into_bytes(),
    );
    let number = registry
        .entity("group_water_heater_power")
        .expect("number group");
    assert_eq!(number.device_id.as_deref(), Some("nidoo_group"));
    assert_eq!(number.part_of, None);
}

#[test]
fn diagnostic_flag_present() {
    let registry = load_house();
    let entity = registry.entity(LINKQUALITY).expect("linkquality");
    assert_eq!(entity.category, EntityCategory::Diagnostic);
    assert!(!entity.enabled_by_default);
}

#[test]
fn garbage_never_panics() {
    let mut registry = load_house();
    let before: Vec<String> = registry.entities().map(|e| e.unique_id.clone()).collect();
    let ignored_before = registry.ignored().len();

    let changes = registry.apply(Inbound::Discovery(
        discovery_topic("nidoo/sensor/garbage/x/config"),
        vec![0xff, 0xfe, b'{'],
    ));
    assert!(changes.contains(&Change::IgnoredChanged), "{changes:?}");
    assert_eq!(registry.ignored().len(), ignored_before + 1);
    assert!(
        registry
            .ignored()
            .iter()
            .any(|i| i.topic == "nidoo/sensor/garbage/x/config")
    );

    let _ = registry.apply(override_payload(STAIR_LIGHT, "{"));
    let _ = registry.apply(Inbound::Areas(Vec::new()));
    let _ = registry.apply(Inbound::Z2mDevices(b"null".to_vec()));
    let _ = registry.apply(Inbound::Z2mBridgeInfo(b"[1,2".to_vec()));
    let _ = registry.apply(Inbound::Z2mBridgeState(vec![0xff]));
    let _ = registry.apply(Inbound::State {
        topic: "zigbee2mqtt/rooms/stair/light".to_owned(),
        payload: vec![0xff, 0xfe],
        at: AT,
    });
    let _ = registry.apply(Inbound::State {
        topic: "zigbee2mqtt/rooms/stair/light".to_owned(),
        payload: b"[".to_vec(),
        at: AT,
    });
    let _ = registry.apply(Inbound::Irrelevant);
    let _ = registry.route("", Vec::new(), AT);
    let _ = registry.route("nidoo/ui/", Vec::new(), AT);
    let _ = registry.route("nidoo//config", vec![0], AT);

    let after: Vec<String> = registry.entities().map(|e| e.unique_id.clone()).collect();
    assert_eq!(before, after);
    assert_eq!(registry.devices().count(), 66);
}

#[test]
fn identical_payload_is_noop() {
    let mut registry = load_house();
    let payload = common::fixture("discovery/switch__0x847127fffe24e322__switch.json");
    let changes = registry.apply(Inbound::Discovery(
        discovery_topic(STAIR_LIGHT_TOPIC),
        payload.clone().into_bytes(),
    ));
    assert!(changes.is_empty(), "{changes:?}");

    let changes = registry.apply(Inbound::Areas(
        common::fixture("ui/_areas.json").into_bytes(),
    ));
    assert!(changes.is_empty(), "{changes:?}");

    let changes = registry.apply(override_payload(
        ALARM_SWITCH,
        &common::fixture("ui/0xbc33acfffe36db49_switch_alarm_zigbee2mqtt.json"),
    ));
    assert!(changes.is_empty(), "{changes:?}");

    let changes = registry.apply(Inbound::Z2mDevices(
        common::fixture("bridge_devices.json").into_bytes(),
    ));
    assert!(changes.is_empty(), "{changes:?}");

    // A re-delivered state is a report, not a change (and not a no-op: the device spoke).
    let changes = feed(
        &mut registry,
        THERMOMETER_TOPIC,
        common::fixture("states/rooms__bedroom2__thermometer.json").into_bytes(),
    );
    assert!(
        !changes.iter().any(|c| matches!(c, Change::StateChanged(_))),
        "{changes:?}"
    );
    assert_eq!(
        changes
            .iter()
            .filter(|c| matches!(c, Change::StateReported(_)))
            .count(),
        6, // the five sensors and the firmware `update`, which reads the same payload
        "{changes:?}"
    );

    // A re-published config with a different body is an upsert.
    let edited = payload.replace("\"name\":null", "\"name\":\"Stair\"");
    assert_ne!(edited, payload);
    let changes = registry.apply(Inbound::Discovery(
        discovery_topic(STAIR_LIGHT_TOPIC),
        edited.into_bytes(),
    ));
    assert_eq!(
        changes,
        vec![Change::EntityUpserted(STAIR_LIGHT.to_owned())]
    );
}

#[test]
fn area_resolution_by_name_case_insensitive() {
    let mut registry = load_house();
    registry.apply(override_payload(STAIR_LIGHT, r#"{"area":"Living"}"#));
    assert_eq!(registry.entity(STAIR_LIGHT).expect("entity").area, "living");

    // An unknown area becomes an implicit one, listed after the declared ones with name = id.
    registry.apply(override_payload(STAIR_LIGHT, r#"{"area":"attic"}"#));
    assert_eq!(registry.entity(STAIR_LIGHT).expect("entity").area, "attic");
    let areas = registry.effective_areas();
    let declared = registry.areas_document().areas.len();
    let attic = areas
        .iter()
        .position(|a| a.id == "attic")
        .expect("implicit area");
    assert!(attic >= declared);
    assert_eq!(areas[attic].name, "attic");
    assert_eq!(areas.last().map(|a| a.id.as_str()), Some(UNASSIGNED_AREA));
}

/// A blank `area` in a retained override (written by another client) is as good as absent:
/// the entity keeps its discovered area instead of joining an implicit area with a blank id.
#[test]
fn blank_override_area_is_ignored() {
    let mut registry = load_house();
    for blank in ["", "  "] {
        registry.apply(override_payload(
            STAIR_LIGHT,
            &format!(r#"{{"area":"{blank}"}}"#),
        ));
        assert_eq!(
            registry.entity(STAIR_LIGHT).expect("entity").area,
            "stair",
            "{blank:?}"
        );
        assert!(
            registry
                .effective_areas()
                .iter()
                .all(|a| !a.id.trim().is_empty()),
            "{blank:?}"
        );
    }
}

#[test]
fn override_before_discovery_applies_when_the_entity_appears() {
    let mut registry = Registry::new(config());
    let changes = registry.apply(override_payload("test_switch", r#"{"area":"Garage"}"#));
    assert!(changes.is_empty(), "{changes:?}");
    assert!(registry.raw_override("test_switch").is_some());

    let config = json!({
        "unique_id": "test_switch",
        "state_topic": "zigbee2mqtt/test",
        "command_topic": "zigbee2mqtt/test/set",
        "device": {"identifiers": ["zigbee2mqtt_0xtest"], "name": "test"}
    });
    let changes = registry.apply(Inbound::Discovery(
        discovery_topic("nidoo/switch/test/config"),
        config.to_string().into_bytes(),
    ));
    assert_eq!(
        changes,
        vec![
            Change::EntityUpserted("test_switch".to_owned()),
            Change::SubscriptionsChanged
        ]
    );
    assert_eq!(
        registry.entity("test_switch").expect("entity").area,
        "Garage"
    );
    assert_eq!(
        registry.subscription_topics(),
        &BTreeSet::from(["zigbee2mqtt/test".to_owned()])
    );

    // Declaring the area afterwards resolves the override's name to the declared id.
    let changes = registry.apply(Inbound::Areas(
        br#"{"areas":[{"id":"garage","name":"Garage"}]}"#.to_vec(),
    ));
    assert_eq!(
        changes,
        vec![
            Change::EntityUpserted("test_switch".to_owned()),
            Change::AreasChanged
        ]
    );
    assert_eq!(
        registry.entity("test_switch").expect("entity").area,
        "garage"
    );
}

#[test]
fn device_based_discovery_expands_and_removes() {
    let mut registry = Registry::new(config());
    let topic = discovery_topic("nidoo/device/0xdev/config");
    let config = json!({
        "dev": {"ids": ["zigbee2mqtt_0xdev"], "name": "rooms/office/strip"},
        "o": {"name": "test"},
        "~": "zigbee2mqtt/rooms/office/strip",
        "cmps": {
            "light": {"p": "light", "uniq_id": "dev_light", "stat_t": "~", "cmd_t": "~/set", "schema": "json"},
            "speed": {"p": "number", "uniq_id": "dev_speed", "stat_t": "~", "cmd_t": "~/set"}
        }
    });
    let changes = registry.apply(Inbound::Discovery(
        topic.clone(),
        config.to_string().into_bytes(),
    ));
    assert_eq!(
        changes,
        vec![
            Change::EntityUpserted("dev_light".to_owned()),
            Change::EntityUpserted("dev_speed".to_owned()),
            Change::SubscriptionsChanged
        ]
    );
    let light = registry.entity("dev_light").expect("light");
    assert_eq!(light.area, "office");
    assert_eq!(
        light.discovery_topic,
        "nidoo/light/0xdev/light/config".to_owned()
    );
    assert_eq!(
        registry
            .entity("dev_speed")
            .expect("number")
            .part_of
            .as_deref(),
        Some("dev_light")
    );
    assert!(registry.ignored().is_empty());

    // A re-publish without the number (HA marks removals with a platform-only entry) drops it.
    let config = json!({
        "dev": {"ids": ["zigbee2mqtt_0xdev"], "name": "rooms/office/strip"},
        "o": {"name": "test"},
        "~": "zigbee2mqtt/rooms/office/strip",
        "cmps": {
            "light": {"p": "light", "uniq_id": "dev_light", "stat_t": "~", "cmd_t": "~/set", "schema": "json"},
            "speed": {"p": "number"}
        }
    });
    let changes = registry.apply(Inbound::Discovery(
        topic.clone(),
        config.to_string().into_bytes(),
    ));
    assert_eq!(changes, vec![Change::EntityRemoved("dev_speed".to_owned())]);
    assert!(registry.entity("dev_light").is_some());

    // An empty payload on the device topic removes everything it produced.
    let changes = registry.apply(Inbound::Discovery(topic, Vec::new()));
    assert_eq!(
        changes,
        vec![
            Change::EntityRemoved("dev_light".to_owned()),
            Change::SubscriptionsChanged
        ]
    );
    assert_eq!(registry.entities().count(), 0);
    assert!(registry.subscription_topics().is_empty());
}

#[test]
fn template_failure_marks_unknown_and_recovers() {
    let mut registry = Registry::new(config());
    let broken = json!({
        "unique_id": "broken",
        "state_topic": "zigbee2mqtt/broken",
        "value_template": "{{ value_json.x | no_such_filter }}"
    });
    registry.apply(Inbound::Discovery(
        discovery_topic("nidoo/sensor/broken/config"),
        broken.to_string().into_bytes(),
    ));
    let changes = registry.apply(Inbound::State {
        topic: "zigbee2mqtt/broken".to_owned(),
        payload: br#"{"x":1}"#.to_vec(),
        at: AT,
    });
    assert!(!changes.contains(&Change::StateChanged("broken".to_owned())));
    assert_eq!(
        registry.state("broken").expect("state").value,
        StateValue::Unknown
    );
    assert_eq!(
        registry.template_failures(),
        &BTreeSet::from(["broken".to_owned()])
    );

    let fixed = json!({
        "unique_id": "broken",
        "state_topic": "zigbee2mqtt/broken",
        "value_template": "{{ value_json.x }}"
    });
    registry.apply(Inbound::Discovery(
        discovery_topic("nidoo/sensor/broken/config"),
        fixed.to_string().into_bytes(),
    ));
    let changes = registry.apply(Inbound::State {
        topic: "zigbee2mqtt/broken".to_owned(),
        payload: br#"{"x":1}"#.to_vec(),
        at: AT,
    });
    assert_eq!(changes, vec![Change::StateChanged("broken".to_owned())]);
    assert_eq!(
        registry.state("broken").expect("state").value,
        StateValue::Number(1.0)
    );
    assert!(registry.template_failures().is_empty());
}

#[test]
fn route_classifies_every_topic_kind() {
    let registry = load_house();
    let route = |topic: &str| registry.route(topic, b"x".to_vec(), AT);
    assert!(matches!(route("nidoo/ui/_areas"), Inbound::Areas(_)));
    assert!(matches!(route("nidoo/ui/_types"), Inbound::Ignored));
    assert!(matches!(
        route("nidoo/ui/some_id"),
        Inbound::Override { unique_id, .. } if unique_id == "some_id"
    ));
    assert!(matches!(
        route("zigbee2mqtt/bridge/devices"),
        Inbound::Z2mDevices(_)
    ));
    assert!(matches!(
        route("zigbee2mqtt/bridge/info"),
        Inbound::Z2mBridgeInfo(_)
    ));
    assert!(matches!(
        route("nidoo/switch/x/config"),
        Inbound::Discovery(DiscoveryTopic { component, .. }, _) if component == "switch"
    ));
    assert!(matches!(
        route(THERMOMETER_TOPIC),
        Inbound::State { at, .. } if at == AT
    ));
    assert!(matches!(route("nidoo/automation/x"), Inbound::Irrelevant));
    assert!(matches!(
        route("zigbee2mqtt/nobody/here"),
        Inbound::Irrelevant
    ));
    assert!(matches!(
        route("homeassistant/switch/x/config"),
        Inbound::Irrelevant
    ));
}

#[test]
fn bridge_state_feeds_entity_availability() {
    let mut registry = load_house();
    // Every Z2M entity declares `zigbee2mqtt/bridge/state` as an availability topic, so once
    // subscribed the bridge state is a state message for them as well as the bridge.
    let inbound = registry.route(
        "zigbee2mqtt/bridge/state",
        br#"{"state":"offline"}"#.to_vec(),
        AT,
    );
    let changes = registry.apply(inbound);
    assert!(changes.contains(&Change::BridgeChanged), "{changes:?}");
    assert!(changes.contains(&Change::StateChanged(STAIR_LIGHT.to_owned())));
    assert_eq!(
        registry.bridge().state,
        nidoo_core::model::Availability::Offline
    );
    assert_eq!(
        registry.state(STAIR_LIGHT).expect("state").availability,
        nidoo_core::model::Availability::Offline
    );
}

const REMOTE: &str = "device_automation.0xd4fe12345";

fn remote_topic(object: &str) -> String {
    format!("nidoo/device_automation/0xd4fe12345/{object}/config")
}

fn remote_config(payload: &str) -> Vec<u8> {
    json!({
        "automation_type": "trigger",
        "topic": "zigbee2mqtt/rooms/hall/remote/action",
        "payload": payload,
        "type": "action",
        "subtype": payload,
        "device": {"identifiers": ["zigbee2mqtt_0xd4fe12345"], "name": "rooms/hall/remote"}
    })
    .to_string()
    .into_bytes()
}

fn remote_registry() -> Registry {
    let mut registry = Registry::new(config());
    for object in ["action_on", "action_off", "action_brightness_move_up"] {
        let payload = object.trim_start_matches("action_");
        feed(&mut registry, &remote_topic(object), remote_config(payload));
    }
    registry
}

fn signals(registry: &Registry) -> Vec<String> {
    registry
        .entity(REMOTE)
        .and_then(|e| e.capabilities.generic.clone())
        .map(|g| g.signals)
        .unwrap_or_default()
}

#[test]
fn actions_of_one_device_fold_into_one_remote() {
    let registry = remote_registry();
    let remotes: Vec<_> = registry
        .entities()
        .filter(|e| e.component == "device_automation")
        .collect();
    assert_eq!(remotes.len(), 1);
    assert_eq!(remotes[0].unique_id, REMOTE);
    assert_eq!(signals(&registry), ["on", "off", "brightness_move_up"]);
}

#[test]
fn removing_one_action_keeps_the_others() {
    let mut registry = remote_registry();
    let changes = feed(&mut registry, &remote_topic("action_off"), Vec::new());
    assert!(changes.contains(&Change::EntityUpserted(REMOTE.to_owned())));
    assert!(!changes.contains(&Change::EntityRemoved(REMOTE.to_owned())));
    assert_eq!(signals(&registry), ["on", "brightness_move_up"]);
}

#[test]
fn removing_last_action_removes_the_remote() {
    let mut registry = Registry::new(config());
    feed(
        &mut registry,
        &remote_topic("action_on"),
        remote_config("on"),
    );
    assert!(registry.entity(REMOTE).is_some());
    let changes = feed(&mut registry, &remote_topic("action_on"), Vec::new());
    assert!(changes.contains(&Change::EntityRemoved(REMOTE.to_owned())));
    assert!(registry.entity(REMOTE).is_none());
}

#[test]
fn remote_subscribes_to_its_action_topics() {
    let registry = remote_registry();
    let topic = "zigbee2mqtt/rooms/hall/remote/action";
    assert!(registry.subscription_topics().contains(topic));
    assert!(
        registry
            .entity(REMOTE)
            .unwrap()
            .bindings
            .topics()
            .contains(topic)
    );
}

#[test]
fn solar_is_reserved() {
    assert!(RegistryConfig::default_reserved().contains(&"solar".to_owned()));
}

const REMOTE_ACTION_TOPIC: &str = "zigbee2mqtt/rooms/hall/remote/action";

/// A `bridge/devices` list with one device named `rooms/hall/remote` exposing `exposes`.
fn remote_devices(exposes: serde_json::Value) -> Inbound {
    Inbound::Z2mDevices(
        json!([{
            "ieee_address": "0xd4fe12345",
            "friendly_name": "rooms/hall/remote",
            "type": "EndDevice",
            "supported": true,
            "interview_state": "SUCCESSFUL",
            "definition": {"model": "TS0043", "vendor": "Tuya", "description": "Wireless remote", "exposes": exposes}
        }])
        .to_string()
        .into_bytes(),
    )
}

fn action_exposes(values: &[&str]) -> serde_json::Value {
    json!([
        {"type": "numeric", "name": "battery", "property": "battery", "access": 1},
        {"type": "enum", "name": "action", "property": "action", "access": 1, "values": values}
    ])
}

fn remote_actions(registry: &Registry) -> Vec<serde_json::Value> {
    registry
        .entity(REMOTE)
        .and_then(|e| e.bindings.config.get("actions"))
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default()
}

fn is_seeded(action: &serde_json::Value) -> bool {
    action.get("seeded").is_some()
}

#[test]
fn exposed_actions_create_the_remote() {
    let mut registry = Registry::new(config());
    let changes = registry.apply(remote_devices(action_exposes(&[
        "1_single", "1_double", "1_hold",
    ])));
    assert!(changes.contains(&Change::EntityUpserted(REMOTE.to_owned())));
    assert!(changes.contains(&Change::SubscriptionsChanged));
    assert_eq!(signals(&registry), ["1_single", "1_double", "1_hold"]);
    let first = &remote_actions(&registry)[0];
    assert_eq!(first["topic"], REMOTE_ACTION_TOPIC);
    assert_eq!(first["object"], "action_1_single");
    assert!(is_seeded(first));
    assert!(registry.subscription_topics().contains(REMOTE_ACTION_TOPIC));
}

#[test]
fn nested_and_duplicate_exposed_actions_are_read_once() {
    let mut registry = Registry::new(config());
    registry.apply(remote_devices(json!([
        {"type": "composite", "features": [
            {"type": "enum", "name": "action", "values": ["a", "", "b"]}
        ]},
        {"type": "enum", "property": "action", "values": ["b", "c"]}
    ])));
    assert_eq!(signals(&registry), ["a", "b", "c"]);
}

#[test]
fn discovered_action_replaces_its_seeded_twin() {
    let mut registry = Registry::new(config());
    registry.apply(remote_devices(action_exposes(&["on", "off", "toggle"])));
    feed(
        &mut registry,
        &remote_topic("action_off"),
        remote_config("off"),
    );
    assert_eq!(signals(&registry), ["on", "off", "toggle"]);
    let actions = remote_actions(&registry);
    assert_eq!(actions.len(), 3);
    assert!(is_seeded(&actions[0]));
    assert!(!is_seeded(&actions[1]));
    assert!(is_seeded(&actions[2]));
}

#[test]
fn devices_after_discovery_keep_discovered_and_seed_the_rest() {
    let mut registry = Registry::new(config());
    feed(
        &mut registry,
        &remote_topic("action_off"),
        remote_config("off"),
    );
    registry.apply(remote_devices(action_exposes(&["on", "off", "toggle"])));
    assert_eq!(signals(&registry), ["on", "off", "toggle"]);
    let seeded: Vec<bool> = remote_actions(&registry).iter().map(is_seeded).collect();
    assert_eq!(seeded, [true, false, true]);
}

#[test]
fn removing_an_exposed_discovered_action_keeps_it_seeded() {
    let mut registry = Registry::new(config());
    registry.apply(remote_devices(action_exposes(&["on", "off"])));
    feed(
        &mut registry,
        &remote_topic("action_off"),
        remote_config("off"),
    );
    feed(&mut registry, &remote_topic("action_off"), Vec::new());
    assert_eq!(signals(&registry), ["on", "off"]);
    assert!(remote_actions(&registry).iter().all(is_seeded));
}

#[test]
fn device_leaving_removes_a_seeded_only_remote() {
    let mut registry = Registry::new(config());
    registry.apply(remote_devices(action_exposes(&["on", "off"])));
    let changes = registry.apply(Inbound::Z2mDevices(b"[]".to_vec()));
    assert!(changes.contains(&Change::EntityRemoved(REMOTE.to_owned())));
    assert!(registry.entity(REMOTE).is_none());
}

#[test]
fn device_leaving_keeps_discovered_actions() {
    let mut registry = Registry::new(config());
    registry.apply(remote_devices(action_exposes(&["on", "off"])));
    feed(
        &mut registry,
        &remote_topic("action_off"),
        remote_config("off"),
    );
    registry.apply(remote_devices(json!([])));
    assert_eq!(signals(&registry), ["off"]);
    assert!(!is_seeded(&remote_actions(&registry)[0]));
}

#[test]
fn device_without_action_expose_creates_nothing() {
    let mut registry = Registry::new(config());
    registry.apply(remote_devices(json!([
        {"type": "numeric", "name": "battery", "property": "battery"}
    ])));
    assert!(registry.entity(REMOTE).is_none());
}

#[test]
fn seeded_action_is_a_signal() {
    use nidoo_core::template::Templates;
    use nidoo_core::types::{signals::signal_of, type_def};
    let mut registry = Registry::new(config());
    registry.apply(remote_devices(action_exposes(&["1_double"])));
    let entity = registry.entity(REMOTE).expect("remote");
    let def = type_def("device_automation").unwrap();
    let templates = Templates::new();
    let of = |topic: &str, payload: &[u8]| signal_of(entity, def, topic, payload, &templates);
    assert_eq!(
        of(REMOTE_ACTION_TOPIC, b"1_double"),
        Some("1_double".to_owned())
    );
    assert_eq!(of(REMOTE_ACTION_TOPIC, b"1_hold"), None);
    assert_eq!(of("zigbee2mqtt/other/action", b"1_double"), None);
}

#[test]
fn invert_flips_the_switch_state_both_ways() {
    let mut registry = load_house();
    let state = |r: &Registry| r.state(STAIR_LIGHT).expect("state").value.clone();
    let relay = |r: &mut Registry, word: &str| {
        feed(
            r,
            "zigbee2mqtt/rooms/stair/light",
            format!(r#"{{"state":"{word}"}}"#).into_bytes(),
        )
    };
    relay(&mut registry, "ON");
    assert_eq!(state(&registry), StateValue::OnOff(true));

    // Turning `invert` on flips the stored value at once, without waiting for a report.
    let changes = registry.apply(override_payload(STAIR_LIGHT, r#"{"invert":true}"#));
    assert!(
        changes.contains(&Change::StateChanged(STAIR_LIGHT.to_owned())),
        "{changes:?}"
    );
    assert_eq!(state(&registry), StateValue::OnOff(false));
    assert!(registry.entity(STAIR_LIGHT).expect("entity").ui.invert);

    // Reports decode inverted.
    relay(&mut registry, "OFF");
    assert_eq!(state(&registry), StateValue::OnOff(true));

    // A re-announce rebuilds the entity; the stored value must not flip again.
    let mut config: serde_json::Value = serde_json::from_str(&common::fixture(
        "discovery/switch__0x847127fffe24e322__switch.json",
    ))
    .expect("json");
    config["name"] = json!("renamed");
    feed(
        &mut registry,
        STAIR_LIGHT_TOPIC,
        config.to_string().into_bytes(),
    );
    assert_eq!(state(&registry), StateValue::OnOff(true));

    // Clearing the override flips back.
    registry.apply(override_payload(STAIR_LIGHT, ""));
    assert_eq!(state(&registry), StateValue::OnOff(false));
}

#[test]
fn invert_is_ignored_on_a_non_switch() {
    let mut registry = load_house();
    let before = registry.state(SHUTTER_COVER).expect("state").value.clone();
    let changes = registry.apply(override_payload(SHUTTER_COVER, r#"{"invert":true}"#));
    assert!(!changes.contains(&Change::StateChanged(SHUTTER_COVER.to_owned())));
    assert_eq!(registry.state(SHUTTER_COVER).expect("state").value, before);
}

/// A switch with one state topic, as `equal_value_report_is_reported_not_changed` builds it.
fn switch_registry() -> Registry {
    let mut registry = Registry::new(config());
    let config = json!({
        "unique_id": "test_switch",
        "state_topic": "zigbee2mqtt/test",
        "command_topic": "zigbee2mqtt/test/set",
        "device": {"identifiers": ["zigbee2mqtt_0xtest"], "name": "test"}
    });
    registry.apply(Inbound::Discovery(
        discovery_topic("nidoo/switch/test/config"),
        config.to_string().into_bytes(),
    ));
    registry
}

/// A recorded value goes back on an entity that never reported, stamped with its recorded
/// time; a real message after that overrides it.
#[test]
fn restore_state_fills_a_never_reported_entity() {
    let mut registry = switch_registry();
    let changes = registry.restore_state("test_switch", StateValue::OnOff(true), AT);
    assert_eq!(
        changes,
        vec![Change::StateChanged("test_switch".to_owned())]
    );
    let state = registry.state("test_switch").expect("state");
    assert_eq!(state.value, StateValue::OnOff(true));
    assert_eq!(state.last_update, Some(AT));

    let inbound = registry.route("zigbee2mqtt/test", b"OFF".to_vec(), AT + 5);
    registry.apply(inbound);
    let state = registry.state("test_switch").expect("state");
    assert_eq!(state.value, StateValue::OnOff(false));
    assert_eq!(state.last_update, Some(AT + 5));
}

#[test]
fn restore_state_ignores_an_unknown_id() {
    let mut registry = switch_registry();
    assert!(
        registry
            .restore_state("nope", StateValue::OnOff(true), AT)
            .is_empty()
    );
}

/// A message that reached the entity but read nothing (still unknown) means it has spoken
/// since boot: an old recorded value must not come back over it.
#[test]
fn restore_state_skips_an_entity_touched_since_boot() {
    let mut registry = switch_registry();
    let inbound = registry.route("zigbee2mqtt/test", b"maybe".to_vec(), AT);
    assert_eq!(
        registry.apply(inbound),
        vec![Change::StateTouched("test_switch".to_owned())]
    );
    let changes = registry.restore_state("test_switch", StateValue::OnOff(true), AT - 100);
    assert!(changes.is_empty(), "{changes:?}");
    let state = registry.state("test_switch").expect("state");
    assert_eq!(state.value, StateValue::Unknown);
}

/// Every Z2M entity reads `zigbee2mqtt/bridge/state` for its availability, and the bridge
/// says `online` at boot before any device speaks. That stamps `last_update` but is not a
/// report: a sensor without a retained state must still get its recorded value back.
#[test]
fn restore_state_fills_an_entity_that_only_heard_its_availability() {
    let mut registry = Registry::new(config());
    let config = json!({
        "unique_id": "test_motion",
        "state_topic": "zigbee2mqtt/test_motion",
        "value_template": "{{ value_json.occupancy }}",
        "payload_on": true,
        "payload_off": false,
        "device_class": "occupancy",
        "availability": [{"topic": "zigbee2mqtt/bridge/state", "value_template": "{{ value_json.state }}"}],
        "device": {"identifiers": ["zigbee2mqtt_0xmotion"], "name": "test_motion"}
    });
    registry.apply(Inbound::Discovery(
        discovery_topic("nidoo/binary_sensor/test_motion/config"),
        config.to_string().into_bytes(),
    ));
    let inbound = registry.route(
        "zigbee2mqtt/bridge/state",
        br#"{"state":"online"}"#.to_vec(),
        AT,
    );
    registry.apply(inbound);

    let changes = registry.restore_state("test_motion", StateValue::OnOff(false), AT - 100);
    assert_eq!(
        changes,
        vec![Change::StateChanged("test_motion".to_owned())]
    );
    let state = registry.state("test_motion").expect("state");
    assert_eq!(state.value, StateValue::OnOff(false));
    assert_eq!(state.last_update, Some(AT - 100));
}

#[test]
fn restore_state_keeps_a_value_already_read() {
    let mut registry = switch_registry();
    let inbound = registry.route("zigbee2mqtt/test", b"ON".to_vec(), AT);
    registry.apply(inbound);
    let changes = registry.restore_state("test_switch", StateValue::OnOff(false), AT - 100);
    assert!(changes.is_empty(), "{changes:?}");
    let state = registry.state("test_switch").expect("state");
    assert_eq!(state.value, StateValue::OnOff(true));
    assert_eq!(state.last_update, Some(AT));
}

/// A satellite of the sound service, reading the service's shared `nidoo-sound/status`.
fn sound_satellite(sid: &str) -> Inbound {
    let config = json!({
        "unique_id": format!("nidoo_sound_{sid}_connected"),
        "state_topic": format!("nidoo-sound/{sid}/connected"),
        "availability_topic": "nidoo-sound/status",
        "device_class": "connectivity",
        "device": {"identifiers": [format!("nidoo_sound_{sid}")], "name": sid}
    });
    Inbound::Discovery(
        discovery_topic(&format!(
            "nidoo/binary_sensor/nidoo_sound_{sid}/connected/config"
        )),
        config.to_string().into_bytes(),
    )
}

/// The broker sends a retained message once per subscription, and the hub is already
/// subscribed to a shared availability topic when a second entity reading it is discovered:
/// the newcomer gets the word the topic last said instead of staying unknown.
#[test]
fn late_entity_gets_the_last_word_of_a_shared_availability_topic() {
    let mut registry = Registry::new(config());
    registry.apply(sound_satellite("a"));
    let inbound = registry.route("nidoo-sound/status", b"online".to_vec(), AT);
    registry.apply(inbound);

    let changes = registry.apply(sound_satellite("b"));
    assert!(
        changes.contains(&Change::StateChanged("nidoo_sound_b_connected".to_owned())),
        "{changes:?}"
    );
    let late = registry.state("nidoo_sound_b_connected").expect("state");
    assert_eq!(late.availability, Availability::Online);
    assert_eq!(
        late.value,
        StateValue::Unknown,
        "only the availability is replayed"
    );

    // The service's last will reaches both.
    let inbound = registry.route("nidoo-sound/status", b"offline".to_vec(), AT + 1);
    registry.apply(inbound);
    for id in ["nidoo_sound_a_connected", "nidoo_sound_b_connected"] {
        assert_eq!(
            registry.state(id).expect("state").availability,
            Availability::Offline
        );
    }
}

/// Once no entity reads a topic any more the hub unsubscribes, and the next subscription brings
/// the retained word back: the remembered one is dropped with the topic.
#[test]
fn availability_word_is_forgotten_with_its_last_reader() {
    let mut registry = Registry::new(config());
    registry.apply(sound_satellite("a"));
    let inbound = registry.route("nidoo-sound/status", b"online".to_vec(), AT);
    registry.apply(inbound);
    registry.apply(Inbound::Discovery(
        discovery_topic("nidoo/binary_sensor/nidoo_sound_a/connected/config"),
        Vec::new(),
    ));
    assert!(
        !registry
            .subscription_topics()
            .contains("nidoo-sound/status")
    );

    registry.apply(sound_satellite("b"));
    let fresh = registry.state("nidoo_sound_b_connected").expect("state");
    assert_eq!(fresh.availability, Availability::Unknown);
}

/// A re-discovery of the only entity reading a topic keeps the subscription, so the broker
/// sends nothing again: the remembered word survives for a later reader.
#[test]
fn availability_word_survives_a_rediscovery_of_its_only_reader() {
    let mut registry = Registry::new(config());
    registry.apply(sound_satellite("a"));
    let inbound = registry.route("nidoo-sound/status", b"online".to_vec(), AT);
    registry.apply(inbound);
    let renamed = json!({
        "unique_id": "nidoo_sound_a_connected",
        "name": "Renamed",
        "state_topic": "nidoo-sound/a/connected",
        "availability_topic": "nidoo-sound/status",
        "device_class": "connectivity",
        "device": {"identifiers": ["nidoo_sound_a"], "name": "a"}
    });
    registry.apply(Inbound::Discovery(
        discovery_topic("nidoo/binary_sensor/nidoo_sound_a/connected/config"),
        renamed.to_string().into_bytes(),
    ));

    registry.apply(sound_satellite("b"));
    let late = registry.state("nidoo_sound_b_connected").expect("state");
    assert_eq!(late.availability, Availability::Online);
}
