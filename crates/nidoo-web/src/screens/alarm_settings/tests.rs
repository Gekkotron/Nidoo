use std::collections::BTreeMap;

use nidoo_core::model::EntityCategory;
use nidoo_core::model::{Capabilities, EntityState, UiOverride};

use super::*;

fn entity(id: &str, kind: Kind, class: Option<&str>) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: id.into(),
        device_name: None,
        label: None,
        kind,
        component: kind.component().to_string(),
        protocol_kind: kind,
        device_class: class.map(Into::into),
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

#[test]
fn sensor_candidates_classifies_by_device_class() {
    let entities = vec![
        entity("a_door", Kind::BinarySensor, Some("door")),
        entity("a_window", Kind::BinarySensor, Some("window")),
        entity("a_opening", Kind::BinarySensor, Some("opening")),
        entity("a_garage", Kind::BinarySensor, Some("garage_door")),
        entity("a_motion", Kind::BinarySensor, Some("motion")),
        entity("a_occupancy", Kind::BinarySensor, Some("occupancy")),
        entity("a_presence", Kind::BinarySensor, Some("presence")),
        entity("a_smoke", Kind::BinarySensor, Some("smoke")),
        entity("a_plain", Kind::BinarySensor, None),
        entity("a_switch", Kind::Switch, Some("door")),
    ];
    let got = sensor_candidates(&entities, &[]);
    let kinds: Vec<(&str, SensorKind)> = got.iter().map(|(i, k)| (i.as_str(), *k)).collect();
    assert_eq!(
        kinds,
        [
            ("a_door", SensorKind::Opening),
            ("a_window", SensorKind::Opening),
            ("a_opening", SensorKind::Opening),
            ("a_garage", SensorKind::Opening),
            ("a_motion", SensorKind::Motion),
            ("a_occupancy", SensorKind::Motion),
            ("a_presence", SensorKind::Motion),
        ]
    );
    let listed = vec![default_sensor("a_door", SensorKind::Opening)];
    let rest = sensor_candidates(&entities, &listed);
    assert!(rest.iter().all(|(id, _)| id != "a_door"));
    assert_eq!(rest.len(), 6);
}

fn settings_with_door() -> AlarmSettings {
    AlarmSettings {
        sensors: vec![SensorRule {
            entity: "front_door".into(),
            kind: SensorKind::Opening,
            modes: BTreeSet::from([ArmMode::Away, ArmMode::Home, ArmMode::Night]),
            entry: Entry::Delayed,
        }],
        ..AlarmSettings::default()
    }
}

#[test]
fn alarm_line_en_fr() {
    let s = settings_with_door();
    assert_eq!(
        alarm_line(&s, "front_door", Lang::En).as_deref(),
        Some("In alarm: delayed opening · away, home, night")
    );
    assert_eq!(
        alarm_line(&s, "front_door", Lang::Fr).as_deref(),
        Some("Dans l'alarme : ouverture temporisée · absent, présent, nuit")
    );
    assert_eq!(alarm_line(&s, "other", Lang::En), None);

    let mut s = settings_with_door();
    s.sensors[0].entry = Entry::Instant;
    assert_eq!(
        alarm_line(&s, "front_door", Lang::Fr).as_deref(),
        Some("Dans l'alarme : ouverture immédiate · absent, présent, nuit")
    );
    s.sensors.push(default_sensor("hall", SensorKind::Motion));
    assert_eq!(
        alarm_line(&s, "hall", Lang::En).as_deref(),
        Some("In alarm: motion · away")
    );
    s.sirens.push(Siren::Entity {
        entity: "plug".into(),
        chirp_ms: None,
    });
    s.remotes.push(Remote {
        entity: "fob".into(),
        signals: BTreeMap::new(),
    });
    s.presence.trackers.push("phone".into());
    assert_eq!(
        alarm_line(&s, "plug", Lang::En).as_deref(),
        Some("In alarm: siren")
    );
    assert_eq!(
        alarm_line(&s, "fob", Lang::Fr).as_deref(),
        Some("Dans l'alarme : télécommande")
    );
    assert_eq!(
        alarm_line(&s, "phone", Lang::En).as_deref(),
        Some("In alarm: presence tracker")
    );
}

#[test]
fn countdown_formats_minutes_seconds() {
    assert_eq!(countdown_text(127, 100), "0:27");
    assert_eq!(countdown_text(175, 100), "1:15");
    assert_eq!(countdown_text(100, 160), "0:00");
    assert_eq!(countdown_text(100, 100), "0:00");
    assert_eq!(countdown_text(100 + 605, 100), "10:05");
}

fn view(enabled: bool, state: &str) -> AlarmView {
    AlarmView {
        enabled,
        entity_id: "nidoo_alarm_house".into(),
        state: state.into(),
        mode: None,
        source: None,
        since: 0,
        deadline: None,
        cause: vec![],
        open_at_arm: vec![],
        settings: AlarmSettings::default(),
        settings_invalid: None,
        warnings: vec![],
    }
}

#[test]
fn settings_status_off_when_disabled() {
    assert_eq!(settings_status(&view(false, "disarmed"), Lang::En), "Off");
    assert_eq!(
        settings_status(&view(false, "armed_away"), Lang::Fr),
        "Désactivée"
    );
    assert_eq!(
        settings_status(&view(true, "disarmed"), Lang::En),
        "Disarmed"
    );
    assert_eq!(
        settings_status(&view(true, "armed_away"), Lang::En),
        "Armed · away"
    );
    assert_eq!(
        settings_status(&view(true, "armed_night"), Lang::Fr),
        "Armée · nuit"
    );
    assert_eq!(
        settings_status(&view(true, "triggered"), Lang::Fr),
        "Déclenchée"
    );
}

#[test]
fn merge_takes_edits_and_the_latest_elsewhere() {
    let base = AlarmSettings::default();
    let mut draft = base.clone();
    draft.exit_delay_s = 90;
    let mut latest = base.clone();
    latest.entry_delay_s = 45;
    latest.exit_delay_s = 20;
    let got = merge(&base, &draft, &latest);
    assert_eq!(got.exit_delay_s, 90, "the edit wins");
    assert_eq!(
        got.entry_delay_s, 45,
        "the untouched field follows the latest"
    );
    assert_eq!(merge(&base, &base, &latest), latest);
}

#[test]
fn next_signal_prefers_announced_names() {
    let mut taken = default_signals();
    let announced = vec!["disarm".to_owned(), "tamper".to_owned()];
    assert_eq!(next_signal(&announced, &taken), "tamper");
    taken.insert("tamper".into(), RemoteAction::Sos);
    assert_eq!(next_signal(&announced, &taken), "signal");
    taken.insert("signal".into(), RemoteAction::Sos);
    assert_eq!(next_signal(&[], &taken), "signal2");
}

#[test]
fn chirp_is_testable_only_when_the_siren_has_one() {
    let mqtt = |chirp: Option<&str>| Siren::Mqtt {
        mqtt: MqttSiren {
            topic: "t".into(),
            on: "1".into(),
            off: "0".into(),
            chirp: chirp.map(Into::into),
        },
    };
    assert!(siren_has_chirp(&mqtt(Some("b"))));
    assert!(!siren_has_chirp(&mqtt(None)));
    assert!(!siren_has_chirp(&mqtt(Some(""))));
    assert!(siren_has_chirp(&Siren::Entity {
        entity: "e".into(),
        chirp_ms: Some(200)
    }));
    assert!(!siren_has_chirp(&Siren::Entity {
        entity: "e".into(),
        chirp_ms: None
    }));
}
