use super::*;
use nidoo_climate::decide::DeviceCaps;
use nidoo_climate::presence::PresenceState;

fn device(target: Option<f64>, mode: &str) -> Device {
    Device {
        id: "climate.a".into(),
        caps: DeviceCaps {
            hvac_modes: vec!["heat".into(), "off".into()],
            fan_modes: vec![],
            min_temp: None,
            max_temp: None,
            temp_step: Some(0.5),
        },
        target,
        mode: Some(mode.into()),
        fan_mode: None,
    }
}

fn want(setpoint: Option<f64>, mode: &str) -> DeviceTarget {
    DeviceTarget {
        setpoint,
        mode: mode.into(),
        held: false,
    }
}

fn guard_hub() -> Arc<Hub> {
    let mut cfg = crate::config::Config::default();
    cfg.climate.enabled = true;
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    Hub::new(&cfg, tx)
}

#[test]
fn an_armed_guard_dropped_leaves_the_command_unconfirmed() {
    let hub = guard_hub();
    let intended = want(Some(19.0), "heat");
    hub.climate_runtime().inflight.insert("climate.a".into());
    drop(InflightGuard {
        hub: &hub,
        id: "climate.a",
        intended: Some(&intended),
        now: 7,
        armed: true,
    });
    let rt = hub.climate_runtime();
    assert!(rt.inflight.is_empty());
    assert_eq!(rt.unconfirmed.get("climate.a"), Some(&(intended, 7)));
}

#[test]
fn a_disarmed_guard_dropped_only_clears_the_mark() {
    let hub = guard_hub();
    let intended = want(Some(19.0), "heat");
    hub.climate_runtime().inflight.insert("climate.a".into());
    let mut guard = InflightGuard {
        hub: &hub,
        id: "climate.a",
        intended: Some(&intended),
        now: 7,
        armed: true,
    };
    guard.disarm();
    drop(guard);
    let rt = hub.climate_runtime();
    assert!(rt.inflight.is_empty());
    assert!(rt.unconfirmed.is_empty());
}

#[tokio::test]
async fn settings_change_resets_correction() {
    let mut cfg = crate::config::Config::default();
    cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
    cfg.climate.enabled = true;
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../nidoo-core/tests/fixtures/discovery/climate__daikin_192_0_2_150__climate.json");
    hub.ingest(
        "nidoo/climate/daikin_192_0_2_150/climate/config",
        &std::fs::read(fixture).unwrap(),
    );
    let id = hub.snapshot().entities[0].id.clone();
    let mut patch = serde_json::Map::new();
    patch.insert("area".to_owned(), serde_json::Value::from("living"));
    hub.set_override(&id, &patch).await.unwrap();
    let state = FeedbackState {
        target: 19.0,
        offset: -0.5,
        since: 1,
        last_step_at: Some(2),
        reached: false,
    };
    hub.climate_runtime()
        .feedback
        .insert("living".to_owned(), state);
    // Other settings keep the correction.
    let settings = nidoo_climate::doc::ZoneSettings {
        sensor: Some("sensor.x".to_owned()),
        manual_hold_max_min: 100,
        ..Default::default()
    };
    hub.set_climate_zone("living", settings.clone()).unwrap();
    assert!(hub.climate_runtime().feedback.contains_key("living"));
    // A change to the correction settings drops it.
    let mut settings = settings;
    settings.feedback.band = 0.8;
    hub.set_climate_zone("living", settings).unwrap();
    assert!(!hub.climate_runtime().feedback.contains_key("living"));
}

#[tokio::test]
async fn presence_settings_change_resets_state() {
    let mut cfg = crate::config::Config::default();
    cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
    cfg.climate.enabled = true;
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../nidoo-core/tests/fixtures/discovery/climate__daikin_192_0_2_150__climate.json");
    hub.ingest(
        "nidoo/climate/daikin_192_0_2_150/climate/config",
        &std::fs::read(fixture).unwrap(),
    );
    let id = hub.snapshot().entities[0].id.clone();
    let mut patch = serde_json::Map::new();
    patch.insert("area".to_owned(), serde_json::Value::from("living"));
    hub.set_override(&id, &patch).await.unwrap();
    let state = PresenceState {
        last_detected: 1,
        empty_since: Some(2),
        arrival: None,
    };
    hub.climate_runtime()
        .presence
        .insert("living".to_owned(), state);
    let mut settings = nidoo_climate::doc::ZoneSettings::default();
    settings.presence.enabled = true;
    hub.set_climate_zone("living", settings.clone()).unwrap();
    hub.climate_runtime()
        .presence
        .insert("living".to_owned(), state);
    // Other settings keep the state.
    settings.manual_hold_max_min = 100;
    hub.set_climate_zone("living", settings.clone()).unwrap();
    assert!(hub.climate_runtime().presence.contains_key("living"));
    // A change to the presence settings drops it.
    settings.presence.setback = 3.0;
    hub.set_climate_zone("living", settings).unwrap();
    assert!(!hub.climate_runtime().presence.contains_key("living"));
}

#[test]
fn a_command_is_needed_beyond_half_a_step_or_for_another_mode() {
    assert!(!needs_command(
        &device(Some(20.0), "heat"),
        &want(Some(20.25), "heat")
    ));
    assert!(needs_command(
        &device(Some(20.0), "heat"),
        &want(Some(20.5), "heat")
    ));
    assert!(needs_command(
        &device(Some(20.0), "off"),
        &want(Some(20.0), "heat")
    ));
    assert!(needs_command(
        &device(None, "heat"),
        &want(Some(20.0), "heat")
    ));
    assert!(!needs_command(&device(None, "off"), &want(None, "off")));
    assert!(!needs_command(
        &device(Some(9.0), "off"),
        &want(Some(10.0), "off")
    ));
}

#[tokio::test]
async fn restart_restores_the_retained_correction() {
    let mut cfg = crate::config::Config::default();
    cfg.climate.enabled = true;
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let hub = Hub::new(&cfg, tx);
    let decision = |zone: &str| {
        let mut d = no_safety(zone);
        d.source = Source::Schedule;
        d.setpoint = Some(Setpoint::Temp(19.0));
        d.adjust = Some(-1.0);
        d.at = 500;
        d
    };
    {
        let mut store = hub.climate_store();
        let living = store.zones.entry("living".to_owned()).or_default();
        living.settings.feedback.enabled = true;
        living.decision = Some(decision("living"));
        // Correction switched off since: nothing to restore.
        let office = store.zones.entry("office".to_owned()).or_default();
        office.decision = Some(decision("office"));
    }
    hub.ensure_seeded();
    let rt = hub.climate_runtime();
    assert_eq!(
        rt.feedback.get("living"),
        Some(&FeedbackState {
            target: 19.0,
            offset: -1.0,
            since: 500,
            last_step_at: Some(500),
            reached: false,
        })
    );
    assert!(!rt.feedback.contains_key("office"));
}

#[test]
fn decision_rows_store_target_and_adjust() {
    let mut decision = no_safety("z");
    decision.setpoint = Some(Setpoint::Temp(19.0));
    decision.adjust = Some(-0.5);
    decision
        .entities
        .insert("a".into(), want(Some(18.5), "heat"));
    decision.entities.insert(
        "b".into(),
        DeviceTarget {
            setpoint: Some(21.0),
            mode: "heat".into(),
            held: true,
        },
    );
    let rows = decision_rows(&decision, 0);
    let got: Vec<_> = rows
        .iter()
        .map(|r| (r.entity_id.as_str(), r.setpoint, r.adjust))
        .collect();
    assert_eq!(
        got,
        vec![("a", Some(19.0), Some(-0.5)), ("b", Some(21.0), None)]
    );

    decision.adjust = None;
    let rows = decision_rows(&decision, 0);
    assert_eq!(rows[0].setpoint, Some(18.5));
    assert_eq!(rows[0].adjust, None);
}

#[tokio::test]
async fn hand_hold_until_matches_hand_change() {
    use nidoo_climate::doc::{Block, Day, HhMm, ModeRules, Rules, Season};
    let mut cfg = crate::config::Config::default();
    cfg.climate.enabled = true;
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let hub = Hub::new(&cfg, tx);
    let tz = hub.zone();
    // A Monday, 10:00 house time.
    let now = tz
        .with_ymd_and_hms(2026, 11, 2, 10, 0, 0)
        .single()
        .unwrap()
        .timestamp() as u64;
    let all_days = vec![
        Day::Mon,
        Day::Tue,
        Day::Wed,
        Day::Thu,
        Day::Fri,
        Day::Sat,
        Day::Sun,
    ];
    let schedule = |from: u16, to: u16| Rules {
        heat: Some(ModeRules {
            base: Setpoint::Temp(17.0),
            blocks: vec![Block {
                days: all_days.clone(),
                from: HhMm(from),
                to: HhMm(to),
                setpoint: Setpoint::Temp(20.0),
                evidence: None,
                heated_by: None,
            }],
        }),
        cool: None,
    };
    {
        let mut store = hub.climate_store();
        store.house.season = Season::Heat;
        let living = store.zones.entry("living".to_owned()).or_default();
        living.settings.manual_hold_max_min = 240;
        // The next change is at 11:30, in 90 minutes.
        living.settings.rules = schedule(11 * 60 + 30, 12 * 60);
    }
    assert_eq!(hub.hand_hold_until("living", now), now + 90 * 60);
    hub.record_hand_change("living", "climate.a", Some(21.0), "heat", now);
    let held = hub.climate_store().zones["living"].overrides.clone();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].until, hub.hand_hold_until("living", now));

    // The next change in 6 h: held for manual_hold_max_min.
    hub.climate_store()
        .zones
        .get_mut("living")
        .unwrap()
        .settings
        .rules = schedule(16 * 60, 17 * 60);
    assert_eq!(hub.hand_hold_until("living", now), now + 240 * 60);
}
