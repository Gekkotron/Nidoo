use super::*;
use nidoo_core::discovery::{Parsed, TopicClass, classify, parse};
use nidoo_core::model::{EntityCategory, Kind};

const ID: &str = "9kDIII64_oap3z6x5vV92ffB6OvJ6WU5778PBR5u-Bs";

fn prefs() -> PlayerPrefs {
    PlayerPrefs {
        client_id: ID.into(),
        name: "Kitchen satellite".into(),
        volume: 45,
        speech_volume: 45,
        muted: false,
        allowed: true,
    }
}

#[test]
fn discovery_parses_in_the_hub() {
    let sid = short_id(ID);
    let mut found = Vec::new();
    for (topic, payload) in discovery_messages(&prefs(), "homeassistant") {
        let TopicClass::Discovery(t) = classify(&topic, &["homeassistant".into()], &[]) else {
            panic!("{topic} is not a discovery topic")
        };
        let Ok(Parsed::Entity(e)) = parse(&t, payload.as_bytes(), &topic) else {
            panic!("{topic} did not parse: {payload}")
        };
        assert_eq!(e.device_name.as_deref(), Some("Kitchen satellite"));
        found.push((e.kind, e.unique_id.clone(), e.category));
    }
    assert_eq!(
        found,
        [
            (
                Kind::Number,
                format!("nidoo_sound_{sid}_volume"),
                EntityCategory::Primary
            ),
            (
                Kind::Number,
                format!("nidoo_sound_{sid}_speech"),
                EntityCategory::Primary
            ),
            (
                Kind::Switch,
                format!("nidoo_sound_{sid}_mute"),
                EntityCategory::Primary
            ),
            (
                Kind::Switch,
                format!("nidoo_sound_{sid}_allowed"),
                // Primary, not Config: the web UI's search and room
                // pages leave Config entities out, and a new satellite
                // plays nothing until this is switched on.
                EntityCategory::Primary
            ),
            (
                Kind::BinarySensor,
                format!("nidoo_sound_{sid}_connected"),
                EntityCategory::Diagnostic
            ),
            (
                Kind::Sensor,
                format!("nidoo_sound_{sid}_address"),
                EntityCategory::Diagnostic
            ),
        ]
    );
}

#[test]
fn discovery_topics_and_availability() {
    let sid = short_id(ID);
    let msgs = discovery_messages(&prefs(), "homeassistant");
    assert_eq!(
        msgs[0].0,
        format!("homeassistant/number/nidoo_sound_{sid}/volume/config")
    );
    for (_, payload) in &msgs {
        let v: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(v["availability_topic"], STATUS_TOPIC);
        assert_eq!(
            v["device"]["identifiers"],
            serde_json::json!([format!("nidoo_sound_{sid}")])
        );
        assert_eq!(v["device"]["manufacturer"], "Nidoo");
    }
    let vol: serde_json::Value = serde_json::from_str(&msgs[0].1).unwrap();
    assert_eq!(
        (
            vol["min"].as_i64(),
            vol["max"].as_i64(),
            vol["mode"].as_str()
        ),
        (Some(0), Some(100), Some("slider"))
    );
    assert_eq!(
        vol["command_topic"],
        format!("nidoo-sound/{sid}/volume/set")
    );
    assert_eq!(vol["state_topic"], format!("nidoo-sound/{sid}/volume"));
}

#[test]
fn state_messages_carry_values() {
    let sid = short_id(ID);
    let mut p = prefs();
    p.muted = true;
    let msgs = state_messages(&p, false, None);
    assert!(msgs.contains(&(format!("nidoo-sound/{sid}/volume"), "45".into())));
    assert!(msgs.contains(&(format!("nidoo-sound/{sid}/mute"), "ON".into())));
    assert!(msgs.contains(&(format!("nidoo-sound/{sid}/allowed"), "ON".into())));
    assert!(msgs.contains(&(format!("nidoo-sound/{sid}/connected"), "OFF".into())));
}

#[test]
fn speech_command_parses() {
    let t = "nidoo-sound/abc/speech/set";
    assert_eq!(
        parse_command(t, b"72.6"),
        Some(Control::SetSpeech {
            short_id: "abc".into(),
            volume: 73
        })
    );
    assert_eq!(parse_command(t, b"nan"), None);
}

#[test]
fn volume_and_mute_carry_their_markers() {
    let sid = short_id(ID);
    let configs: Vec<serde_json::Value> = discovery_messages(&prefs(), "homeassistant")
        .iter()
        .map(|(_, payload)| serde_json::from_str(payload).unwrap())
        .collect();
    let find = |uid: String| configs.iter().find(|c| c["unique_id"] == uid).unwrap();
    assert_eq!(
        find(format!("nidoo_sound_{sid}_volume"))["device_class"],
        "volume"
    );
    assert_eq!(
        find(format!("nidoo_sound_{sid}_mute"))["device_class"],
        "mute"
    );
    assert!(
        find(format!("nidoo_sound_{sid}_speech"))
            .get("device_class")
            .is_none()
    );
}

#[test]
fn speech_entity_is_published() {
    let sid = short_id(ID);
    let mut p = prefs();
    p.speech_volume = 62;
    let configs: Vec<serde_json::Value> = discovery_messages(&p, "homeassistant")
        .iter()
        .map(|(_, payload)| serde_json::from_str(payload).unwrap())
        .collect();
    let find = |uid: String| configs.iter().find(|c| c["unique_id"] == uid).unwrap();
    let speech = find(format!("nidoo_sound_{sid}_speech"));
    assert_eq!(speech["name"], "Speech volume");
    assert_eq!(
        speech["command_topic"],
        format!("nidoo-sound/{sid}/speech/set")
    );
    assert_eq!(
        find(format!("nidoo_sound_{sid}_volume"))["name"],
        "Music volume"
    );
    assert!(
        state_messages(&p, false, None)
            .contains(&(format!("nidoo-sound/{sid}/speech"), "62".into()))
    );
    assert!(
        removal_messages(&p, "homeassistant")
            .contains(&(format!("nidoo-sound/{sid}/speech"), String::new()))
    );
}

#[test]
fn volume_command_parses_and_clamps() {
    let t = "nidoo-sound/0123456789ab/volume/set";
    let sid = "0123456789ab".to_owned();
    assert_eq!(
        parse_command(t, b"45"),
        Some(Control::SetVolume {
            short_id: sid.clone(),
            volume: 45
        })
    );
    assert_eq!(
        parse_command(t, b"120"),
        Some(Control::SetVolume {
            short_id: sid.clone(),
            volume: 100
        })
    );
    assert_eq!(
        parse_command(t, b"45.6"),
        Some(Control::SetVolume {
            short_id: sid.clone(),
            volume: 46
        })
    );
    assert_eq!(
        parse_command(t, b"-3"),
        Some(Control::SetVolume {
            short_id: sid,
            volume: 0
        })
    );
    assert_eq!(parse_command(t, b"abc"), None);
}

#[test]
fn mute_and_allowed_commands_parse() {
    let sid = "0123456789ab".to_owned();
    assert_eq!(
        parse_command("nidoo-sound/0123456789ab/mute/set", b"ON"),
        Some(Control::SetMute {
            short_id: sid.clone(),
            muted: true
        })
    );
    assert_eq!(
        parse_command("nidoo-sound/0123456789ab/allowed/set", b"OFF"),
        Some(Control::SetAllowed {
            short_id: sid,
            allowed: false
        })
    );
    assert_eq!(
        parse_command("nidoo-sound/0123456789ab/mute/set", b"maybe"),
        None
    );
    assert_eq!(
        parse_command("nidoo-sound/0123456789ab/connected/set", b"ON"),
        None
    );
    assert_eq!(
        parse_command("elsewhere/0123456789ab/mute/set", b"ON"),
        None
    );
}

#[test]
fn incoming_messages_are_routed() {
    use crate::speech::{SPEAK_TOPIC, SpeakRequest};
    let ha = "homeassistant/status";
    let speak = br#"{"id":"c","url":"/api/v1/assist/speech/c.wav","players":["abc"]}"#;
    assert_eq!(
        route(SPEAK_TOPIC, speak, ha),
        Incoming::Speak(SpeakRequest {
            id: "c".into(),
            url: "/api/v1/assist/speech/c.wav".into(),
            players: vec!["abc".into()],
        })
    );
    assert_eq!(route(SPEAK_TOPIC, b"{}", ha), Incoming::Ignored);
    assert_eq!(route(ha, b"online", ha), Incoming::HaOnline);
    assert_eq!(route(ha, b"offline", ha), Incoming::Ignored);
    assert!(matches!(
        route("nidoo-sound/abc/mute/set", b"ON", ha),
        Incoming::Control(Control::SetMute { muted: true, .. })
    ));
    assert_eq!(route("elsewhere", b"x", ha), Incoming::Ignored);
}

#[test]
fn the_hubs_retained_documents_fit_in_one_packet() {
    // `<ui_prefix>/+` brings `_types` back on every connect; a packet over the limit
    // drops the link, so the satellites flap offline and keep their own names.
    let options = mqtt_options(&MqttSettings::default());
    let types = nidoo_core::types::table_json().len();
    assert!(
        options.max_packet_size() >= 16 * types,
        "limit {} for a {types}-byte type table",
        options.max_packet_size()
    );
}

#[test]
fn rename_comes_from_the_volume_entity_override() {
    let ui = "nidoo/ui";
    let topic = "nidoo/ui/nidoo_sound_0123456789ab_volume";
    assert_eq!(
        parse_rename(topic, br#"{"name":" Living room ","area":"living"}"#, ui),
        Some(("0123456789ab".into(), Some("Living room".into())))
    );
    // Cleared override, override without a name, blank name: back to the satellite's own.
    for payload in [&b""[..], br#"{"area":"living"}"#, br#"{"name":"  "}"#] {
        assert_eq!(
            parse_rename(topic, payload, ui),
            Some(("0123456789ab".into(), None)),
            "{}",
            String::from_utf8_lossy(payload)
        );
    }
    for other in [
        "nidoo/ui/nidoo_sound_0123456789ab_mute",
        "nidoo/ui/light_kitchen",
        "nidoo/ui/_areas",
        "nidoo/ui/nidoo_sound__volume",
        "other/ui/nidoo_sound_0123456789ab_volume",
    ] {
        assert_eq!(parse_rename(other, br#"{"name":"X"}"#, ui), None, "{other}");
    }
}

#[test]
fn hold_topic_is_routed() {
    let ha = "homeassistant/status";
    assert_eq!(
        route(
            HOLD_TOPIC,
            br#"{"id":"x","players":["abc"],"for_ms":30000}"#,
            ha
        ),
        Incoming::Hold(HoldRequest {
            id: "x".into(),
            players: vec!["abc".into()],
            for_ms: 30_000,
        })
    );
    assert_eq!(route(HOLD_TOPIC, b"nope", ha), Incoming::Ignored);
}

#[test]
fn removal_clears_every_retained_topic_discovery_first() {
    let p = prefs();
    let msgs = removal_messages(&p, "homeassistant");
    let discovery: Vec<String> = discovery_messages(&p, "homeassistant")
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    let state: Vec<String> = state_messages(&p, true, None)
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    let topics: Vec<String> = msgs.iter().map(|(t, _)| t.clone()).collect();
    assert_eq!(topics, [discovery, state].concat());
    assert!(msgs.iter().all(|(_, payload)| payload.is_empty()));
}

#[test]
fn an_empty_discovery_payload_removes_the_entity_in_the_hub() {
    for (topic, payload) in removal_messages(&prefs(), "homeassistant") {
        let TopicClass::Discovery(t) = classify(&topic, &["homeassistant".into()], &[]) else {
            continue;
        };
        assert!(
            matches!(parse(&t, payload.as_bytes(), &topic), Ok(Parsed::Removed)),
            "{topic}"
        );
    }
}

#[test]
fn remove_command_parses() {
    assert_eq!(
        parse_command("nidoo-sound/0123456789ab/remove/set", b"PRESS"),
        Some(Control::Remove {
            short_id: "0123456789ab".into()
        })
    );
    assert_eq!(
        parse_command("nidoo-sound/0123456789ab/remove/set", b""),
        Some(Control::Remove {
            short_id: "0123456789ab".into()
        })
    );
}

#[test]
fn address_state_is_dotted_or_empty() {
    let sid = short_id(ID);
    let topic = format!("nidoo-sound/{sid}/address");
    let with = state_messages(&prefs(), true, Some("192.0.2.10".parse().unwrap()));
    assert!(with.contains(&(topic.clone(), "192.0.2.10".into())));
    let without = state_messages(&prefs(), false, None);
    assert!(without.contains(&(topic, String::new())));
}
