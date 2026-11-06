//! Home Assistant MQTT discovery parsing against the recorded house fixtures.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use nidoo_core::discovery::{
    DiscoveryError, DiscoveryTopic, Parsed, TopicClass, capabilities, classify, expand,
    expand_device_based, naming, parse,
};
use nidoo_core::model::{AlarmMode, Bindings, Entity, EntityCategory, Kind, UNASSIGNED_AREA};
use nidoo_core::registry::RegistryConfig;
use serde_json::{Value, json};

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

/// Rebuilds the discovery topic a fixture was recorded from, under the `nidoo` prefix.
fn fixture_topic(file_stem: &str) -> String {
    format!("nidoo/{}/config", file_stem.replace("__", "/"))
}

fn discovery_topic(topic: &str) -> DiscoveryTopic {
    match classify(topic, &strings(&["nidoo"]), &strings(&["ui", "automation"])) {
        TopicClass::Discovery(t) => t,
        other => panic!("{topic} is not a discovery topic: {other:?}"),
    }
}

/// Parses one recorded discovery fixture (file name without `.json`) into an entity.
fn parse_fixture(file_stem: &str) -> Entity {
    let topic_str = fixture_topic(file_stem);
    let topic = discovery_topic(&topic_str);
    let payload = common::fixture(&format!("discovery/{file_stem}.json"));
    match parse(&topic, payload.as_bytes(), &topic_str) {
        Ok(Parsed::Entity(entity)) => *entity,
        other => panic!("{file_stem} did not parse as an entity: {other:?}"),
    }
}

#[test]
fn network_subtree_is_reserved() {
    let prefixes = strings(&["nidoo"]);
    let reserved = RegistryConfig::default_reserved();
    for topic in ["nidoo/network/nas", "nidoo/network/nas/avail"] {
        assert_eq!(
            classify(topic, &prefixes, &reserved),
            TopicClass::Reserved("network".into())
        );
    }
    assert!(matches!(
        classify(
            "nidoo/binary_sensor/network/nas/config",
            &prefixes,
            &reserved
        ),
        TopicClass::Discovery(_)
    ));
}

#[test]
fn classifies_topics() {
    let prefixes = strings(&["nidoo"]);
    let reserved = strings(&["ui", "automation"]);
    assert_eq!(
        classify("nidoo/light/x/y/config", &prefixes, &reserved),
        TopicClass::Discovery(DiscoveryTopic {
            prefix: "nidoo".into(),
            component: "light".into(),
            node_id: Some("x".into()),
            object_id: "y".into(),
        })
    );
    assert_eq!(
        classify("nidoo/ui/0xabc", &prefixes, &reserved),
        TopicClass::Reserved("ui".into())
    );
    assert_eq!(
        classify("nidoo/ui/_areas", &prefixes, &reserved),
        TopicClass::Reserved("ui".into())
    );
    assert_eq!(
        classify(
            "homeassistant/sensor/x/config",
            &strings(&["nidoo", "homeassistant"]),
            &reserved
        ),
        TopicClass::Discovery(DiscoveryTopic {
            prefix: "homeassistant".into(),
            component: "sensor".into(),
            node_id: None,
            object_id: "x".into(),
        })
    );
    assert_eq!(
        classify("nidoo/light/x/y/state", &prefixes, &reserved),
        TopicClass::Other
    );
    assert_eq!(
        classify("zigbee2mqtt/rooms/a/b", &prefixes, &reserved),
        TopicClass::Other
    );
}

#[test]
fn expands_abbreviations_and_tilde() {
    let raw = json!({
        "~": "zigbee2mqtt/x",
        "stat_t": "~",
        "cmd_t": "~/set",
        "avty": [{"t": "zigbee2mqtt/bridge/state", "val_tpl": "{{ value_json.state }}"}],
        "dev": {"ids": ["zigbee2mqtt_0x1"], "sa": "Living"}
    });
    let expanded = expand(raw);
    assert_eq!(expanded["state_topic"], json!("zigbee2mqtt/x"));
    assert_eq!(expanded["command_topic"], json!("zigbee2mqtt/x/set"));
    assert_eq!(
        expanded["availability"][0]["topic"],
        json!("zigbee2mqtt/bridge/state")
    );
    assert_eq!(
        expanded["availability"][0]["value_template"],
        json!("{{ value_json.state }}")
    );
    assert_eq!(
        expanded["device"]["identifiers"],
        json!(["zigbee2mqtt_0x1"])
    );
    assert_eq!(expanded["device"]["suggested_area"], json!("Living"));
    assert!(expanded.get("~").is_none());
    assert!(expanded.get("stat_t").is_none());
    assert!(expanded.get("dev").is_none());
}

#[test]
fn parses_every_recorded_discovery_file() {
    let dir = common::fixture_dir().join("discovery");
    let mut entities = 0usize;
    let mut ignored = 0usize;
    let mut removed = 0usize;
    let mut reasons = BTreeMap::<String, usize>::new();
    let mut errors = Vec::<String>::new();
    let mut files = 0usize;
    for entry in std::fs::read_dir(&dir).expect("discovery fixture dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        files += 1;
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("utf-8 file stem");
        let topic_str = fixture_topic(stem);
        let topic = discovery_topic(&topic_str);
        let payload = std::fs::read(&path).expect("fixture bytes");
        match parse(&topic, &payload, &topic_str) {
            Ok(Parsed::Entity(entity)) => {
                entities += 1;
                assert!(!entity.unique_id.is_empty(), "{stem}: empty unique_id");
                // Write-only generic entities (a scene, a notifier) have nothing to read.
                assert!(
                    entity.kind == Kind::Generic || !entity.bindings.topics().is_empty(),
                    "{stem}: no topic to subscribe to"
                );
                assert_eq!(entity.discovery_topic, topic_str);
                assert_eq!(entity.area, UNASSIGNED_AREA);
                assert!(entity.part_of.is_none());
            }
            Ok(Parsed::Ignored { reason }) => {
                ignored += 1;
                *reasons.entry(reason).or_default() += 1;
            }
            // A fragment of a remote the registry folds: counted with the entities.
            Ok(Parsed::DeviceAutomation(_)) => entities += 1,
            Ok(Parsed::Removed) => removed += 1,
            Err(e) => errors.push(format!("{topic_str}: {e}")),
        }
    }
    println!(
        "discovery fixtures: {files} files, {entities} entities, {ignored} ignored, {removed} removed"
    );
    println!("ignored reasons: {reasons:?}");
    println!("errors: {errors:?}");
    assert!(errors.is_empty(), "parse errors: {errors:?}");
    assert!(entities >= 400, "only {entities} entities");
    assert_eq!(entities + ignored + removed, files);
    assert_eq!(removed, 0);
}

#[test]
fn light_capabilities_from_recorded_json_schema_light() {
    let light = parse_fixture("light__0x00124b0011b6b80f__light_4");
    assert_eq!(light.kind, Kind::Light);
    assert_eq!(light.protocol_kind, Kind::Light);
    assert!(light.capabilities.brightness);
    assert!(light.capabilities.effects.iter().any(|e| e == "blink"));
    assert_eq!(
        light.capabilities.color_modes,
        BTreeSet::from(["brightness".to_owned()])
    );
    assert_eq!(light.unique_id, "0x00124b0011b6b80f_light_4_zigbee2mqtt");
    assert_eq!(
        light.device_id.as_deref(),
        Some("zigbee2mqtt_0x00124b0011b6b80f")
    );
    assert_eq!(light.name, "outdoor/light_sensors_door 4");
    assert_eq!(light.label.as_deref(), Some("4"));
    assert!(light.path_area.is_none());
}

#[test]
fn cover_number_select_alarm_capabilities() {
    let cover = parse_fixture("cover__0x804b50fffe252151__cover");
    assert_eq!(cover.kind, Kind::Cover);
    assert!(cover.capabilities.position);
    assert_eq!(cover.name, "shutter");
    assert_eq!(cover.device_name.as_deref(), Some("shutter"));
    assert_eq!(cover.path_area.as_deref(), Some("bedroom2"));

    let number = parse_fixture("number__0x1051dbfffe1a4058__effect_speed");
    assert_eq!(number.kind, Kind::Number);
    assert_eq!(number.capabilities.min, Some(0.0));
    assert_eq!(number.capabilities.max, Some(255.0));
    assert!(number.capabilities.step.is_some());

    let select = parse_fixture("select__0x000d6ffffe9d44e4__indicator_mode");
    assert_eq!(select.kind, Kind::Select);
    assert!(!select.capabilities.options.is_empty());

    let alarm = parse_fixture("alarm_control_panel__jeedom__house_alarm");
    assert_eq!(alarm.kind, Kind::AlarmPanel);
    assert_eq!(
        alarm.capabilities.alarm_modes,
        BTreeSet::from([AlarmMode::Home, AlarmMode::Away, AlarmMode::Night])
    );
    // No `code` in the config: arming never asks for one.
    assert!(!alarm.capabilities.code_arm_required);
    assert!(!alarm.capabilities.code_disarm_required);
    assert_eq!(alarm.name, "Alarm");

    let vacuum = parse_fixture("vacuum__roborock_40313ca8b964__roborock_40313ca8b964");
    assert_eq!(
        vacuum.capabilities.vacuum_actions,
        strings(&["start", "pause", "stop", "return_to_base", "locate"])
    );
    assert_eq!(vacuum.capabilities.fan_speeds.len(), 6);

    let climate = parse_fixture("climate__daikin_192_0_2_150__climate");
    assert_eq!(climate.capabilities.hvac_modes.len(), 6);
    assert_eq!(climate.capabilities.fan_modes.len(), 7);
    assert_eq!(climate.capabilities.min_temp, Some(10.0));
    assert_eq!(climate.capabilities.max_temp, Some(32.0));
    assert_eq!(climate.capabilities.temp_step, Some(0.5));
}

#[test]
fn naming_and_path_area() {
    assert_eq!(
        naming::split(Some("rooms/bedroom1/thermometer"), Some("Temperature")),
        (
            "thermometer Temperature".to_owned(),
            Some("thermometer".to_owned()),
            Some("Temperature".to_owned()),
            Some("bedroom1".to_owned())
        )
    );
    assert_eq!(
        naming::split(Some("outdoor/pool"), Some("Water")),
        (
            "outdoor/pool Water".to_owned(),
            Some("outdoor/pool".to_owned()),
            Some("Water".to_owned()),
            None
        )
    );
    assert_eq!(
        naming::split(Some("rooms/bedroom1/thermometer"), None),
        (
            "thermometer".to_owned(),
            Some("thermometer".to_owned()),
            None,
            Some("bedroom1".to_owned())
        )
    );
    assert_eq!(
        naming::split(None, Some("Water")),
        ("Water".to_owned(), None, None, None)
    );
}

#[test]
fn diagnostic_and_disabled_are_kept_and_flagged() {
    let linkquality = parse_fixture("sensor__0x0000000000000000__linkquality");
    assert_eq!(linkquality.category, EntityCategory::Diagnostic);
    assert!(!linkquality.enabled_by_default);
    assert_eq!(linkquality.unit.as_deref(), Some("lqi"));
    assert_eq!(linkquality.icon.as_deref(), Some("mdi:signal"));

    let effect = parse_fixture("select__0x9035eafffec07012__effect");
    assert_eq!(effect.category, EntityCategory::Primary);
    assert!(!effect.enabled_by_default);

    let identify = parse_fixture("button__0x00158d0006ea91e3__identify");
    assert_eq!(identify.category, EntityCategory::Config);
    assert!(identify.enabled_by_default);
    assert_eq!(identify.device_class.as_deref(), Some("identify"));
}

#[test]
fn empty_payload_is_removed_and_ignored_component_is_ignored() {
    let light = discovery_topic("nidoo/light/0x1/light/config");
    assert!(matches!(
        parse(&light, b"", "nidoo/light/0x1/light/config"),
        Ok(Parsed::Removed)
    ));

    let device = discovery_topic("nidoo/device/0x1/config");
    match parse(&device, br#"{"cmps":{}}"#, "nidoo/device/0x1/config") {
        Ok(Parsed::Ignored { reason }) => {
            assert_eq!(reason, "device-based discovery: expanded by caller");
        }
        other => panic!("expected Ignored, got {other:?}"),
    }

    assert!(parse(&light, b"{not json", "nidoo/light/0x1/light/config").is_err());
    assert!(parse(&light, b"[1,2]", "nidoo/light/0x1/light/config").is_err());
    assert!(parse(&light, b"\xff\xfe", "nidoo/light/0x1/light/config").is_err());

    let sensor = discovery_topic("nidoo/sensor/0x1/temp/config");
    assert!(
        parse(
            &sensor,
            br#"{"name":"Temperature"}"#,
            "nidoo/sensor/0x1/temp/config"
        )
        .is_err(),
        "a sensor without state_topic is invalid"
    );

    // No unique_id: falls back to <component>.<node_id>.<object_id>.
    let payload = br#"{"state_topic":"z/t","command_topic":"z/t/set"}"#;
    match parse(&light, payload, "nidoo/light/0x1/light/config") {
        Ok(Parsed::Entity(entity)) => {
            assert_eq!(entity.unique_id, "light.0x1.light");
            assert_eq!(entity.name, "light");
            assert!(entity.device_id.is_none());
        }
        other => panic!("expected Entity, got {other:?}"),
    }
}

#[test]
fn device_based_discovery_expands_cmps() {
    let topic = discovery_topic("nidoo/device/plug_1/config");
    let payload = br#"{
        "~": "z/plug_1",
        "dev": {"ids": ["plug_1"], "name": "rooms/office/plug"},
        "o": {"name": "handwritten"},
        "avty": [{"t": "~/availability"}],
        "cmps": {
            "relay": {"p": "switch", "stat_t": "~", "cmd_t": "~/set", "uniq_id": "plug_1_relay"},
            "power": {"p": "sensor", "stat_t": "~", "val_tpl": "{{ value_json.power }}", "uniq_id": "plug_1_power"},
            "gone": {"p": "sensor"}
        }
    }"#;
    let expanded = expand_device_based(&topic, payload).expect("device-based config expands");
    assert_eq!(expanded.len(), 2);
    let by_id: BTreeMap<&str, &(DiscoveryTopic, Value)> = expanded
        .iter()
        .map(|item| (item.0.object_id.as_str(), item))
        .collect();

    let (relay_topic, relay) = by_id["relay"];
    assert_eq!(relay_topic.component, "switch");
    assert_eq!(relay_topic.node_id.as_deref(), Some("plug_1"));
    assert_eq!(relay["state_topic"], json!("z/plug_1"));
    assert_eq!(relay["command_topic"], json!("z/plug_1/set"));
    assert_eq!(relay["device"]["identifiers"], json!(["plug_1"]));
    assert_eq!(relay["origin"]["name"], json!("handwritten"));
    assert_eq!(
        relay["availability"][0]["topic"],
        json!("z/plug_1/availability")
    );
    assert!(relay.get("~").is_none());
    assert!(relay.get("platform").is_none() && relay.get("p").is_none());

    let (power_topic, power) = by_id["power"];
    assert_eq!(power_topic.component, "sensor");
    assert_eq!(power["state_topic"], json!("z/plug_1"));
    assert_eq!(power["value_template"], json!("{{ value_json.power }}"));

    // The expanded configs parse as ordinary entities.
    let bytes = serde_json::to_vec(power).expect("serialises");
    match parse(power_topic, &bytes, "nidoo/device/plug_1/config") {
        Ok(Parsed::Entity(entity)) => {
            assert_eq!(entity.unique_id, "plug_1_power");
            assert_eq!(entity.kind, Kind::Sensor);
            assert_eq!(entity.path_area.as_deref(), Some("office"));
        }
        other => panic!("expected Entity, got {other:?}"),
    }
}

#[test]
fn bindings_topics_rule() {
    let bindings = Bindings {
        config: json!({
            "state_topic": "z/l",
            "brightness_state_topic": "z/l/brightness",
            "command_topic": "z/l/set",
            "brightness_command_topic": "z/l/brightness/set",
            "set_position_topic": "z/l/position/set",
            "json_attributes_topic": "z/l/attributes",
            "availability": [{"topic": "z/bridge/state"}, {"topic": "z/l/availability"}]
        }),
    };
    assert_eq!(
        bindings.topics(),
        BTreeSet::from_iter(strings(&[
            "z/l",
            "z/l/brightness",
            "z/l/attributes",
            "z/bridge/state",
            "z/l/availability",
        ]))
    );
    assert_eq!(
        bindings.availability_topics(),
        BTreeSet::from_iter(strings(&["z/bridge/state", "z/l/availability"]))
    );
    assert!(bindings.is_availability_only("z/bridge/state"));
    assert!(!bindings.is_availability_only("z/l"));
    assert!(!bindings.is_availability_only("z/elsewhere"));
}

/// `availability_topic` is the single-topic form; a state topic that doubles as the
/// availability topic is not availability-only.
#[test]
fn bindings_availability_topic_rule() {
    let bindings = Bindings {
        config: json!({
            "state_topic": "z/s",
            "availability_topic": "z/s/availability"
        }),
    };
    assert_eq!(
        bindings.availability_topics(),
        BTreeSet::from_iter(strings(&["z/s/availability"]))
    );
    assert!(bindings.is_availability_only("z/s/availability"));

    let shared = Bindings {
        config: json!({"state_topic": "z/s", "availability_topic": "z/s"}),
    };
    assert_eq!(
        shared.availability_topics(),
        BTreeSet::from_iter(strings(&["z/s"]))
    );
    assert!(!shared.is_availability_only("z/s"));
}

#[test]
fn z2m_group_is_flagged_and_a_device_is_not() {
    let group = parse_fixture("light__1221051039810110150109113116116_1__light");
    assert!(group.capabilities.z2m_group);
    let cover_group = parse_fixture("cover__1221051039810110150109113116116_2__cover");
    assert!(cover_group.capabilities.z2m_group);
    let light = parse_fixture("light__0x00124b0011b6b80f__light_4");
    assert!(!light.capabilities.z2m_group);
    let empty = capabilities::from_config(Kind::Light, &json!({"group": []}));
    assert!(!empty.z2m_group);
}

#[test]
fn capabilities_from_config_reads_alarm_code() {
    let cfg = json!({
        "supported_features": ["arm_home", "arm_custom_bypass"],
        "code": "REMOTE_CODE_TEXT",
        "code_disarm_required": false
    });
    let caps = capabilities::from_config(Kind::AlarmPanel, &cfg);
    assert_eq!(
        caps.alarm_modes,
        BTreeSet::from([AlarmMode::Home, AlarmMode::CustomBypass])
    );
    assert!(caps.code_arm_required);
    assert!(!caps.code_disarm_required);
    assert_eq!(caps.code_format, nidoo_core::model::CodeFormat::Text);
}

/// One recorded fixture per component the type table adds (`device_automation` is covered
/// by the signal wave).
const GENERIC_FIXTURES: &[(&str, &str)] = &[
    ("lock", "lock__0x00158d0001a2b3c4__lock"),
    ("fan", "fan__0x00158d0002b3c4d5__fan"),
    ("siren", "siren__0x00124b0003c4d5e6__siren"),
    ("valve", "valve__0x00124b0004d5e6f7__valve"),
    ("humidifier", "humidifier__homeassistant__bedroom"),
    ("lawn_mower", "lawn_mower__homeassistant__garden"),
    ("text", "text__0x00124b001c2fa59c__action"),
    ("scene", "scene__homeassistant__movie_night"),
    ("device_tracker", "device_tracker__nidoo_phone"),
    ("camera", "camera__homeassistant__front_door"),
    ("image", "image__homeassistant__weather_map"),
    ("event", "event__0x00158d0005e6f708__action"),
    ("tag", "tag__homeassistant__key_fob"),
    ("notify", "notify__homeassistant__phone"),
    ("update", "update__0x20a716fffea4e8de__update"),
];

#[test]
fn each_new_component_becomes_a_generic_entity() {
    for (component, stem) in GENERIC_FIXTURES {
        let entity = parse_fixture(stem);
        assert_eq!(entity.kind, Kind::Generic, "{stem}");
        assert_eq!(entity.protocol_kind, Kind::Generic, "{stem}");
        assert_eq!(entity.component, *component, "{stem}");
        assert!(
            entity.capabilities.generic.is_some(),
            "{stem}: no generic caps"
        );
    }
}

#[test]
fn existing_kinds_carry_their_component() {
    let light = parse_fixture("light__0x00124b0011b6b80f__light_4");
    assert_eq!(light.component, "light");
    assert_eq!(light.kind, Kind::Light);
    assert!(light.capabilities.generic.is_none());
}

fn generic_entity(component: &str, config: Value) -> Entity {
    let topic_str = format!("nidoo/{component}/test/config");
    let topic = discovery_topic(&topic_str);
    match parse(&topic, config.to_string().as_bytes(), &topic_str) {
        Ok(Parsed::Entity(entity)) => *entity,
        other => panic!("{component} did not parse as an entity: {other:?}"),
    }
}

#[test]
fn fan_without_percentage_topic_offers_power_only() {
    let fan = generic_entity("fan", json!({"command_topic": "f/set", "state_topic": "f"}));
    assert_eq!(fan.capabilities.generic.unwrap().controls, ["power"]);
}

#[test]
fn lock_without_payload_open_has_no_open_control() {
    let lock = generic_entity(
        "lock",
        json!({"command_topic": "l/set", "state_topic": "l"}),
    );
    let controls = lock.capabilities.generic.unwrap().controls;
    assert!(controls.iter().any(|c| c == "locked"), "{controls:?}");
    assert!(!controls.iter().any(|c| c == "open"), "{controls:?}");
    let with_open = generic_entity(
        "lock",
        json!({"command_topic": "l/set", "state_topic": "l", "payload_open": "OPEN"}),
    );
    assert!(
        with_open
            .capabilities
            .generic
            .unwrap()
            .controls
            .iter()
            .any(|c| c == "open")
    );
}

#[test]
fn lock_without_state_topic_is_rejected() {
    let topic_str = "nidoo/lock/test/config";
    let topic = discovery_topic(topic_str);
    let result = parse(&topic, br#"{"command_topic": "l/set"}"#, topic_str);
    assert_eq!(
        result,
        Err(DiscoveryError::Invalid(
            "lock without state_topic".to_owned()
        ))
    );
}

#[test]
fn a_bare_topic_key_is_a_state_topic() {
    let camera = parse_fixture("camera__homeassistant__front_door");
    assert!(
        camera
            .bindings
            .topics()
            .contains("homeassistant/camera/front_door/image")
    );
}

/// The media player contract's discovery config.
fn media_player_config() -> Value {
    json!({
        "name": "Music",
        "unique_id": "music_node_player",
        "state_topic": "music/player/state",
        "command_topic": "music/player/set",
        "availability_topic": "music/status",
        "supported_features": ["play", "pause", "next", "previous", "play_media"],
        "device": {"identifiers": ["music_node"], "name": "Music server", "model": "M", "manufacturer": "X"}
    })
}

#[test]
fn media_player_discovers_from_three_and_two_level_topics() {
    let payload = media_player_config().to_string();
    for topic in [
        "nidoo/media_player/music_node/player/config",
        "nidoo/media_player/player/config",
    ] {
        let entity = common::entity_from_payload(topic, &payload);
        assert_eq!(entity.kind, Kind::MediaPlayer, "{topic}");
        assert_eq!(entity.protocol_kind, Kind::MediaPlayer, "{topic}");
        assert_eq!(entity.component, "media_player", "{topic}");
        assert!(entity.bindings.topics().contains("music/player/state"));
    }
}

#[test]
fn unknown_device_class_is_kept() {
    let entity = common::entity(
        "number",
        json!({
            "name": "Music volume", "unique_id": "vol", "state_topic": "s/vol",
            "command_topic": "s/vol/set", "min": 0, "max": 100, "device_class": "volume"
        }),
    );
    assert_eq!(entity.device_class.as_deref(), Some("volume"));
}
