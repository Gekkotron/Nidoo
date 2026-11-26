use super::*;
use nidoo_core::registry::RegistryConfig;
use serde_json::json;

const AT: u64 = 1_700_000_000;

fn registry() -> Registry {
    Registry::new(RegistryConfig {
        discovery_prefixes: vec!["nidoo".to_owned()],
        ui_prefix: "nidoo/ui".to_owned(),
        z2m_base: "zigbee2mqtt".to_owned(),
        reserved: vec!["ui".to_owned(), "automation".to_owned()],
    })
}

fn feed(reg: &mut Registry, topic: &str, payload: &str) {
    let inbound = reg.route(topic, payload.as_bytes().to_vec(), AT);
    reg.apply(inbound);
}

/// A satellite as `nidoo-sound` announces it, in `area`, with its switches as given.
fn satellite(
    reg: &mut Registry,
    sid: &str,
    area: &str,
    allowed: bool,
    connected: bool,
    mute: bool,
) {
    let device = format!("{SOUND_DEVICE_PREFIX}{sid}");
    for (component, object) in [
        ("number", "volume"),
        ("switch", "mute"),
        ("switch", "allowed"),
        ("binary_sensor", "connected"),
    ] {
        let mut config = json!({
            "name": object,
            "unique_id": format!("{device}_{object}"),
            "state_topic": format!("nidoo-sound/{sid}/{object}"),
            "payload_on": "ON",
            "payload_off": "OFF",
            "availability_topic": "nidoo-sound/status",
            "device": { "identifiers": [device], "name": sid },
        });
        if component != "binary_sensor" {
            config["command_topic"] = json!(format!("nidoo-sound/{sid}/{object}/set"));
        }
        feed(
            reg,
            &format!("nidoo/{component}/{device}/{object}/config"),
            &config.to_string(),
        );
        feed(
            reg,
            &format!("nidoo/ui/{device}_{object}"),
            &json!({ "area": area }).to_string(),
        );
    }
    let on = |b: bool| if b { "ON" } else { "OFF" };
    feed(reg, &format!("nidoo-sound/{sid}/volume"), "40");
    feed(reg, &format!("nidoo-sound/{sid}/mute"), on(mute));
    feed(reg, &format!("nidoo-sound/{sid}/allowed"), on(allowed));
    feed(reg, &format!("nidoo-sound/{sid}/connected"), on(connected));
}

/// Two declared rooms; `a` and `muted` in the living room, `b` in the kitchen, one not
/// allowed and one disconnected.
fn house() -> Registry {
    let mut reg = registry();
    feed(
        &mut reg,
        "nidoo/ui/_areas",
        r#"{"areas":[{"id":"sejour","name":"Séjour","order":0},{"id":"kitchen","name":"Kitchen","order":1}]}"#,
    );
    feed(&mut reg, "nidoo-sound/status", "online");
    satellite(&mut reg, "a", "sejour", true, true, false);
    satellite(&mut reg, "muted", "sejour", true, true, true);
    satellite(&mut reg, "b", "kitchen", true, true, false);
    satellite(&mut reg, "refused", "kitchen", false, true, false);
    satellite(&mut reg, "gone", "sejour", true, false, false);
    reg
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn speaking_satellites_all_rooms_picks_every_able_one() {
    let got = speaking_satellites(&house(), &SpeakTarget::All).unwrap();
    assert_eq!(got, ids(&["a", "b", "muted"]));
}

#[test]
fn speaking_satellites_room_picks_only_its_own() {
    let got = speaking_satellites(&house(), &SpeakTarget::Room("kitchen".into())).unwrap();
    assert_eq!(got, ids(&["b"]));
}

#[test]
fn speaking_satellites_room_by_name_or_id() {
    let reg = house();
    for raw in ["Séjour", "séjour", "sejour"] {
        let got = speaking_satellites(&reg, &SpeakTarget::Room(raw.into())).unwrap();
        assert_eq!(got, ids(&["a", "muted"]), "{raw}");
    }
}

#[test]
fn speaking_satellites_one_satellite() {
    let reg = house();
    let got = speaking_satellites(&reg, &SpeakTarget::Satellite("b".into())).unwrap();
    assert_eq!(got, ids(&["b"]));
    let refused = speaking_satellites(&reg, &SpeakTarget::Satellite("refused".into()));
    assert!(
        matches!(refused, Err(SpeakError::SatelliteCannotSpeak)),
        "{refused:?}"
    );
    assert_eq!(
        refused.unwrap_err().to_string(),
        "this satellite cannot speak now"
    );
}

#[test]
fn speaking_satellites_unknown_room() {
    let got = speaking_satellites(&house(), &SpeakTarget::Room("attic".into()));
    assert!(
        matches!(got, Err(SpeakError::UnknownRoom(ref r)) if r == "attic"),
        "{got:?}"
    );
}

#[test]
fn speaking_satellites_none_while_the_sound_service_is_down() {
    // The service's last will marks every satellite entity offline; their retained
    // Allowed and Connected still read ON.
    let mut reg = house();
    feed(&mut reg, "nidoo-sound/status", "offline");
    let got = speaking_satellites(&reg, &SpeakTarget::All);
    assert!(matches!(got, Err(SpeakError::NoSatellite)), "{got:?}");
}

#[test]
fn speaking_satellites_nothing_left_is_an_error() {
    let mut reg = house();
    feed(&mut reg, "nidoo-sound/b/connected", "OFF");
    let got = speaking_satellites(&reg, &SpeakTarget::Room("Kitchen".into()));
    assert!(matches!(got, Err(SpeakError::NoSatelliteInRoom)), "{got:?}");
    let empty = speaking_satellites(&registry(), &SpeakTarget::All);
    assert!(matches!(empty, Err(SpeakError::NoSatellite)), "{empty:?}");
    assert_eq!(
        empty.unwrap_err().to_string(),
        "no satellite can speak in the house"
    );
}

fn with_address(reg: &mut Registry, sid: &str, address: &str) {
    let device = format!("{SOUND_DEVICE_PREFIX}{sid}");
    let config = json!({
        "name": "address",
        "unique_id": format!("{device}_address"),
        "state_topic": format!("nidoo-sound/{sid}/address"),
        "availability_topic": "nidoo-sound/status",
        "entity_category": "diagnostic",
        "device": { "identifiers": [device], "name": sid },
    });
    feed(
        reg,
        &format!("nidoo/sensor/{device}/address/config"),
        &config.to_string(),
    );
    feed(reg, &format!("nidoo-sound/{sid}/address"), address);
}

#[test]
fn room_satellites_take_every_allowed_one_of_the_room() {
    let mut reg = house();
    // Muted and disconnected ones are held too: the hold is harmless and they may
    // come back mid-exchange.
    assert_eq!(room_satellites(&reg, "a"), ids(&["a", "gone", "muted"]));
    assert_eq!(
        room_satellites(&reg, "b"),
        ids(&["b"]),
        "refused is not allowed"
    );
    satellite(&mut reg, "lonely", UNASSIGNED_AREA, true, true, false);
    assert_eq!(room_satellites(&reg, "lonely"), ids(&["lonely"]));
    assert_eq!(room_of(&reg, "lonely"), None);
    assert_eq!(room_of(&reg, "a").as_deref(), Some("sejour"));
}

#[test]
fn voice_satellites_need_an_address() {
    let mut reg = house();
    with_address(&mut reg, "a", "192.0.2.10");
    with_address(&mut reg, "b", "");
    with_address(&mut reg, "refused", "192.0.2.11");
    assert_eq!(
        voice_satellites(&reg),
        vec![("a".to_owned(), "192.0.2.10".parse().unwrap())]
    );
}

#[test]
fn parse_spoken_reads_the_sound_report() {
    assert_eq!(
        parse_spoken(br#"{"id":"c","played":true}"#),
        Some(Spoken {
            id: "c".into(),
            played: true
        })
    );
    assert_eq!(parse_spoken(b"nope"), None);
}
