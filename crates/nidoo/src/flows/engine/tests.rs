use super::*;
use chrono::TimeZone;
use nidoo_core::command::Publish;
use serde_json::json;

fn sun_flow(id: &str, enabled: bool) -> String {
    serde_json::json!({
        "id": id, "name": id, "enabled": enabled, "mode": "single",
        "nodes": [{ "id": "sun", "type": "trigger.sun", "params": { "which": "sunrise" } }],
        "edges": []
    })
    .to_string()
}

/// An engine whose clock reads 2026-01-01 00:00 UTC, on a hub at (`lat`, `lon`).
fn engine_at(lat: f64, lon: f64, zone: &str) -> Engine {
    let mut cfg = crate::config::Config::default();
    cfg.flows.timezone = Some(zone.to_owned());
    cfg.flows.latitude = Some(lat);
    cfg.flows.longitude = Some(lon);
    let (tx, _rx) = mpsc::channel::<Publish>(8);
    let hub = Hub::new(&cfg, tx);
    let fixed = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Engine {
        zone: hub.zone(),
        hub,
        flows: Vec::new(),
        known: BTreeMap::new(),
        watched: BTreeSet::new(),
        confirmed_unknown: BTreeSet::new(),
        active: BTreeMap::new(),
        keep: BTreeSet::new(),
        tasks: JoinSet::new(),
        now: Arc::new(move || fixed),
        schedules: BTreeMap::new(),
        silents: BTreeMap::new(),
        silent_epoch: 0,
        holds: BTreeMap::new(),
        sun_timers: BTreeMap::new(),
        wake_timers: BTreeMap::new(),
        off_peak_base: None,
        off_peak_timer: None,
        off_peak_settle: None,
        catch_up_settle: None,
    }
}

#[tokio::test]
async fn polar_night_flow_stays_disarmed_across_a_toggle() {
    let mut engine = engine_at(78.22, 15.65, "Europe/Oslo");
    for enabled in [true, false, true] {
        engine.hub.ingest(
            "nidoo/automation/polar",
            sun_flow("polar", enabled).as_bytes(),
        );
        engine.reload();
        assert!(engine.sun_timers.is_empty(), "enabled={enabled}");
    }
}

#[tokio::test]
async fn temperate_flow_arms_on_the_same_date_and_disarms_when_disabled() {
    let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
    engine
        .hub
        .ingest("nidoo/automation/day", sun_flow("day", true).as_bytes());
    engine.reload();
    assert_eq!(engine.sun_timers.keys().collect::<Vec<_>>(), ["day/sun"]);
    engine
        .hub
        .ingest("nidoo/automation/day", sun_flow("day", false).as_bytes());
    engine.reload();
    assert!(engine.sun_timers.is_empty());
}

/// A Paris engine, online, with a text sensor `tariff` (availability on its own topic) and
/// a clock the test moves. Returns the engine, the entity id and the clock.
fn tariff_engine(
    utc: (i32, u32, u32, u32, u32),
) -> (
    Engine,
    String,
    Arc<std::sync::Mutex<chrono::DateTime<chrono::Utc>>>,
) {
    let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
    let (y, mo, d, h, mi) = utc;
    let clock = Arc::new(std::sync::Mutex::new(
        chrono::Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap(),
    ));
    let read = Arc::clone(&clock);
    engine.now = Arc::new(move || *read.lock().unwrap());
    engine.hub.ingest(
        "nidoo/sensor/meter/tariff/config",
        br#"{"unique_id":"tariff","state_topic":"test/tariff","availability_topic":"test/tariff/avail"}"#,
    );
    engine.hub.set_connection(ConnectionStatus::Online);
    let id = engine.hub.snapshot().entities[0].id.clone();
    (engine, id, clock)
}

fn off_peak_flow(engine: &Engine, id: &str, edge: &str) {
    let flow = serde_json::json!({
        "id": id, "name": id, "enabled": true, "mode": "single",
        "nodes": [{ "id": "t", "type": "trigger.off_peak", "params": { "edge": edge } }],
        "edges": []
    });
    engine.hub.ingest(
        &format!("nidoo/automation/{id}"),
        flow.to_string().as_bytes(),
    );
}

/// Stores home settings with the device (when `device`) and the windows, and tells the
/// engine as the hub's broadcast would.
fn off_peak_settings(engine: &mut Engine, device: Option<&str>, windows: &[(&str, &str)]) {
    let windows: Vec<_> = windows
        .iter()
        .map(|(start, end)| serde_json::json!({ "start": start, "end": end }))
        .collect();
    let mut doc = serde_json::json!({ "off_peak": windows });
    if let Some(id) = device {
        doc["off_peak_device"] = serde_json::json!({ "entity": id, "values": ["OFF"] });
    }
    engine.hub.ingest_home(doc.to_string().as_bytes());
    engine.handle_event(WsEvent::HomeSettingsChanged {
        settings: engine.hub.home_settings(),
    });
}

/// Publishes `payload` on the tariff's `topic` and hands the engine the change event.
fn tariff_report(engine: &mut Engine, id: &str, topic: &str, payload: &str) {
    engine.hub.ingest(topic, payload.as_bytes());
    let state = engine.hub.read().state(id).cloned().expect("tariff state");
    engine.handle_event(WsEvent::StateChanged {
        id: id.to_owned(),
        state,
    });
}

fn run_count(engine: &Engine, flow: &str) -> usize {
    engine.hub.flow(flow).map_or(0, |f| f.runs.len())
}

const ALARM_STATE_TOPIC: &str = "test/alarm/panel";

/// An online Paris engine that has the alarm panel entity (as the hub publishes it) and a
/// text sensor `tariff` on `test/tariff`.
fn alarm_panel_engine() -> (Engine, String) {
    let engine = engine_at(48.8566, 2.3522, "Europe/Paris");
    engine.hub.ingest(
        &crate::alarm::entity::config_topic("nidoo"),
        crate::alarm::entity::build_config("test/alarm")
            .to_string()
            .as_bytes(),
    );
    engine.hub.ingest(
        "nidoo/sensor/meter/tariff/config",
        br#"{"unique_id":"tariff","state_topic":"test/tariff"}"#,
    );
    engine.hub.set_connection(ConnectionStatus::Online);
    let ids: Vec<String> = engine
        .hub
        .snapshot()
        .entities
        .iter()
        .map(|e| e.id.clone())
        .collect();
    assert!(ids.iter().any(|id| id == crate::alarm::ALARM_ENTITY_ID));
    let tariff = ids
        .into_iter()
        .find(|id| id != crate::alarm::ALARM_ENTITY_ID)
        .expect("tariff");
    (engine, tariff)
}

fn alarm_trigger_flow(engine: &Engine, id: &str, params: serde_json::Value) {
    let flow = serde_json::json!({
        "id": id, "name": id, "enabled": true, "mode": "single",
        "nodes": [{ "id": "t", "type": "trigger.alarm", "params": params }],
        "edges": []
    });
    engine.hub.ingest(
        &format!("nidoo/automation/{id}"),
        flow.to_string().as_bytes(),
    );
}

/// Publishes `payload` on `topic` and hands the engine the change event for `id`.
fn report_on(engine: &mut Engine, id: &str, topic: &str, payload: &str) {
    engine.hub.ingest(topic, payload.as_bytes());
    let state = engine.hub.read().state(id).cloned().expect("state");
    engine.handle_event(WsEvent::StateChanged {
        id: id.to_owned(),
        state,
    });
}

fn alarm_mode(engine: &mut Engine, mode: &str) {
    report_on(
        engine,
        crate::alarm::ALARM_ENTITY_ID,
        ALARM_STATE_TOPIC,
        mode,
    );
}

#[tokio::test]
async fn alarm_trigger_fires_on_to_mode() {
    let (mut engine, _) = alarm_panel_engine();
    alarm_trigger_flow(&engine, "a", serde_json::json!({ "to": "armed_away" }));
    engine.reload();
    assert!(
        engine
            .watched
            .contains(&(crate::alarm::ALARM_ENTITY_ID.to_owned(), None))
    );
    alarm_mode(&mut engine, "disarmed");
    assert_eq!(run_count(&engine, "a"), 0);
    alarm_mode(&mut engine, "armed_home");
    assert_eq!(run_count(&engine, "a"), 0);
    alarm_mode(&mut engine, "disarmed");
    alarm_mode(&mut engine, "armed_away");
    assert_eq!(run_count(&engine, "a"), 1);
    let trigger = engine.hub.flow("a").unwrap().runs[0]
        .trigger
        .clone()
        .unwrap();
    assert_eq!(
        trigger.entity.as_deref(),
        Some(crate::alarm::ALARM_ENTITY_ID)
    );
    assert_eq!(trigger.field, None);
    assert_eq!(trigger.value, serde_json::json!("armed_away"));
    assert_eq!(trigger.previous, serde_json::json!("disarmed"));
}

#[tokio::test]
async fn alarm_trigger_armed_matches_every_armed_mode() {
    let (mut engine, _) = alarm_panel_engine();
    alarm_trigger_flow(&engine, "a", serde_json::json!({ "to": "armed" }));
    engine.reload();
    alarm_mode(&mut engine, "disarmed");
    alarm_mode(&mut engine, "arming");
    assert_eq!(run_count(&engine, "a"), 0);
    alarm_mode(&mut engine, "armed_home");
    assert_eq!(run_count(&engine, "a"), 1);
    alarm_mode(&mut engine, "pending");
    assert_eq!(run_count(&engine, "a"), 1);
}

#[tokio::test]
async fn alarm_trigger_from_only_fires_on_leaving_that_mode() {
    let (mut engine, _) = alarm_panel_engine();
    alarm_trigger_flow(&engine, "a", serde_json::json!({ "from": "disarmed" }));
    engine.reload();
    alarm_mode(&mut engine, "armed_away");
    assert_eq!(run_count(&engine, "a"), 0);
    alarm_mode(&mut engine, "disarmed");
    assert_eq!(run_count(&engine, "a"), 0);
    alarm_mode(&mut engine, "arming");
    assert_eq!(run_count(&engine, "a"), 1);
    alarm_mode(&mut engine, "armed_away");
    assert_eq!(run_count(&engine, "a"), 1);
}

#[tokio::test]
async fn alarm_trigger_without_filters_fires_on_every_change() {
    let (mut engine, _) = alarm_panel_engine();
    alarm_trigger_flow(&engine, "a", serde_json::json!({}));
    engine.reload();
    alarm_mode(&mut engine, "disarmed");
    assert_eq!(run_count(&engine, "a"), 0);
    alarm_mode(&mut engine, "arming");
    alarm_mode(&mut engine, "armed_night");
    assert_eq!(run_count(&engine, "a"), 2);
}

#[tokio::test]
async fn alarm_trigger_first_sighting_and_replay_stay_silent() {
    let (mut engine, _) = alarm_panel_engine();
    alarm_trigger_flow(&engine, "a", serde_json::json!({ "to": "armed_away" }));
    engine.reload();
    alarm_mode(&mut engine, "armed_away");
    assert_eq!(run_count(&engine, "a"), 0);
    alarm_mode(&mut engine, "disarmed");
    engine.handle_event(WsEvent::ConnectionChanged(ConnectionStatus::Offline {
        reason: "test".to_owned(),
    }));
    engine.handle_event(WsEvent::ConnectionChanged(ConnectionStatus::Online));
    alarm_mode(&mut engine, "armed_away");
    assert_eq!(run_count(&engine, "a"), 0);
}

#[tokio::test]
async fn alarm_trigger_ignores_other_entities() {
    let (mut engine, tariff) = alarm_panel_engine();
    alarm_trigger_flow(&engine, "a", serde_json::json!({ "to": "armed_away" }));
    engine.reload();
    report_on(&mut engine, &tariff, "test/tariff", "disarmed");
    report_on(&mut engine, &tariff, "test/tariff", "armed_away");
    assert_eq!(run_count(&engine, "a"), 0);
}

#[tokio::test]
async fn device_change_fires_start_then_end() {
    let (mut engine, id, _) = tariff_engine((2026, 1, 1, 12, 0));
    off_peak_flow(&engine, "on", "start");
    off_peak_flow(&engine, "off", "end");
    engine.reload();
    off_peak_settings(&mut engine, Some(&id), &[]);
    tariff_report(&mut engine, &id, "test/tariff", "PEAK");
    assert_eq!(
        (run_count(&engine, "on"), run_count(&engine, "off")),
        (0, 0)
    );
    tariff_report(&mut engine, &id, "test/tariff", "OFF");
    assert_eq!(
        (run_count(&engine, "on"), run_count(&engine, "off")),
        (1, 0)
    );
    let trigger = engine.hub.flow("on").unwrap().runs[0]
        .trigger
        .clone()
        .unwrap();
    assert_eq!(trigger.entity.as_deref(), Some(id.as_str()));
    assert_eq!(trigger.value, serde_json::json!("start"));
    tariff_report(&mut engine, &id, "test/tariff", "PEAK");
    assert_eq!(
        (run_count(&engine, "on"), run_count(&engine, "off")),
        (1, 1)
    );
}

#[tokio::test]
async fn reconnect_replay_seeds_without_firing() {
    let (mut engine, id, _) = tariff_engine((2026, 1, 1, 12, 0));
    off_peak_flow(&engine, "on", "start");
    off_peak_flow(&engine, "off", "end");
    engine.reload();
    off_peak_settings(&mut engine, Some(&id), &[]);
    tariff_report(&mut engine, &id, "test/tariff", "PEAK");
    engine.handle_event(WsEvent::ConnectionChanged(ConnectionStatus::Offline {
        reason: "test".to_owned(),
    }));
    engine.handle_event(WsEvent::ConnectionChanged(ConnectionStatus::Online));
    tariff_report(&mut engine, &id, "test/tariff", "OFF");
    assert_eq!(run_count(&engine, "on"), 0);
    tariff_report(&mut engine, &id, "test/tariff", "PEAK");
    assert_eq!(run_count(&engine, "off"), 1);
}

#[tokio::test]
async fn settings_change_never_fires() {
    // 12:00 UTC is 13:00 in Paris.
    let (mut engine, _, _) = tariff_engine((2026, 1, 1, 12, 0));
    off_peak_flow(&engine, "on", "start");
    engine.reload();
    off_peak_settings(&mut engine, None, &[("22:00", "06:00")]);
    off_peak_settings(&mut engine, None, &[("12:00", "14:00")]);
    assert_eq!(run_count(&engine, "on"), 0);
}

#[tokio::test]
async fn flow_save_never_fires() {
    let (mut engine, _, _) = tariff_engine((2026, 1, 1, 12, 0));
    off_peak_settings(&mut engine, None, &[("12:00", "14:00")]);
    off_peak_flow(&engine, "on", "start");
    engine.reload();
    off_peak_flow(&engine, "off", "end");
    engine.reload();
    assert_eq!(
        (run_count(&engine, "on"), run_count(&engine, "off")),
        (0, 0)
    );
}

#[tokio::test]
async fn window_boundary_fires_without_device() {
    // 20:59 UTC is 21:59 in Paris.
    let (mut engine, _, clock) = tariff_engine((2026, 1, 1, 20, 59));
    off_peak_flow(&engine, "on", "start");
    engine.reload();
    off_peak_settings(&mut engine, None, &[("22:00", "06:00")]);
    engine.tick_off_peak();
    assert_eq!(run_count(&engine, "on"), 0);
    *clock.lock().unwrap() = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 21, 0, 0).unwrap();
    engine.tick_off_peak();
    assert_eq!(run_count(&engine, "on"), 1);
    let trigger = engine.hub.flow("on").unwrap().runs[0]
        .trigger
        .clone()
        .unwrap();
    assert_eq!(trigger.entity, None);
    assert_eq!(trigger.value, serde_json::json!("start"));
}

#[tokio::test]
async fn fallback_without_windows_ends_once() {
    let (mut engine, id, _) = tariff_engine((2026, 1, 1, 12, 0));
    off_peak_flow(&engine, "off", "end");
    engine.reload();
    off_peak_settings(&mut engine, Some(&id), &[]);
    tariff_report(&mut engine, &id, "test/tariff/avail", "online");
    tariff_report(&mut engine, &id, "test/tariff", "OFF");
    tariff_report(&mut engine, &id, "test/tariff/avail", "offline");
    assert_eq!(run_count(&engine, "off"), 1);
    tariff_report(&mut engine, &id, "test/tariff/avail", "offline");
    assert_eq!(run_count(&engine, "off"), 1);
}

#[tokio::test]
async fn boot_flows_before_settings_then_first_report_never_fires() {
    // At boot the flows can replay before the home settings, and both before the device.
    let (mut engine, id, _) = tariff_engine((2026, 1, 1, 1, 0));
    off_peak_flow(&engine, "on", "start");
    engine.reload();
    off_peak_settings(&mut engine, Some(&id), &[]);
    tariff_report(&mut engine, &id, "test/tariff", "OFF");
    assert_eq!(run_count(&engine, "on"), 0);
    tariff_report(&mut engine, &id, "test/tariff", "PEAK");
    tariff_report(&mut engine, &id, "test/tariff", "OFF");
    assert_eq!(run_count(&engine, "on"), 1);
}

#[tokio::test]
async fn a_lag_keeps_the_next_window_edge() {
    let (mut engine, _, clock) = tariff_engine((2026, 1, 1, 20, 59));
    off_peak_flow(&engine, "on", "start");
    engine.reload();
    off_peak_settings(&mut engine, None, &[("22:00", "06:00")]);
    engine.events_lagged();
    *clock.lock().unwrap() = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 21, 0, 0).unwrap();
    engine.tick_off_peak();
    assert_eq!(run_count(&engine, "on"), 1);
}

#[tokio::test]
async fn a_window_edge_during_an_outage_is_recorded_skipped() {
    let (mut engine, _, clock) = tariff_engine((2026, 1, 1, 20, 59));
    off_peak_flow(&engine, "on", "start");
    engine.reload();
    off_peak_settings(&mut engine, None, &[("22:00", "06:00")]);
    let offline = ConnectionStatus::Offline {
        reason: "test".to_owned(),
    };
    engine.hub.set_connection(offline.clone());
    engine.handle_event(WsEvent::ConnectionChanged(offline));
    *clock.lock().unwrap() = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 21, 0, 0).unwrap();
    engine.tick_off_peak();
    let runs = engine.hub.flow("on").unwrap().runs;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, RunStatus::Skipped);
}

#[tokio::test]
async fn is_off_peak_reads_the_resolver() {
    let (mut engine, id, _) = tariff_engine((2026, 1, 1, 12, 0));
    off_peak_settings(&mut engine, Some(&id), &[]);
    tariff_report(&mut engine, &id, "test/tariff", "OFF");
    let check = node(serde_json::json!({
        "id": "c", "type": "if", "params": { "op": "is_off_peak" }
    }));
    let (outcome, detail, ports) = run_node(
        &engine.hub,
        &engine.now,
        "f",
        "r",
        &check,
        None,
        &mut BTreeMap::new(),
    )
    .await;
    assert_eq!(outcome, StepOutcome::Ok);
    assert_eq!(ports, ["true"]);
    assert_eq!(detail, "is_off_peak → true");
}

/// Runs one `if` node with `average` on an engine hub that has no history.
async fn run_if_average(extra: serde_json::Value) -> (StepOutcome, String, Vec<String>) {
    let engine = engine_at(48.8566, 2.3522, "Europe/Paris");
    let mut params = serde_json::json!({
        "op": "lte", "value": 19, "average": "30m", "entity": "sensor_temperature"
    });
    params
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let node: Node = serde_json::from_value(serde_json::json!({
        "id": "check", "type": "if", "params": params
    }))
    .unwrap();
    run_node(
        &engine.hub,
        &engine.now,
        "f",
        "r",
        &node,
        None,
        &mut BTreeMap::new(),
    )
    .await
}

#[tokio::test]
async fn if_average_without_history_takes_the_missing_port() {
    let (outcome, detail, port) = run_if_average(serde_json::json!({ "missing": "false" })).await;
    assert_eq!(outcome, StepOutcome::Ok);
    assert_eq!(detail, "missing → false");
    assert_eq!(port, ["false"]);
    let (_, detail, port) = run_if_average(serde_json::json!({ "missing": "true" })).await;
    assert_eq!(detail, "missing → true");
    assert_eq!(port, ["true"]);
}

#[tokio::test]
async fn if_average_without_history_and_default_missing_compares_nothing() {
    let (outcome, _, port) = run_if_average(serde_json::json!({})).await;
    assert_eq!(outcome, StepOutcome::Ok);
    assert_eq!(port, ["false"]);
}

#[tokio::test]
async fn if_average_counts_every_reading_including_the_newest() {
    let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
    engine.hub.ingest(
        "nidoo/sensor/room/temperature/config",
        br#"{"unique_id":"room_temperature","state_topic":"test/room","value_template":"{{ value_json.temperature }}","unit_of_measurement":"\u00b0C"}"#,
    );
    let entity = engine.hub.snapshot().entities[0].id.clone();
    let flow = serde_json::json!({
        "id": "avg", "name": "avg", "enabled": true, "mode": "single",
        "nodes": [
            { "id": "t", "type": "trigger.state", "params": { "entity": entity } },
            { "id": "check", "type": "if", "params": { "op": "lte", "value": 19, "average": "30m" } }
        ],
        "edges": [{ "from": "t", "to": "check" }]
    });
    engine
        .hub
        .ingest("nidoo/automation/avg", flow.to_string().as_bytes());
    engine.reload();
    // A repeated reading counts again, and the newest one counts.
    for t in [20, 20, 17] {
        engine
            .hub
            .ingest("test/room", format!(r#"{{"temperature":{t}}}"#).as_bytes());
    }
    let node = engine.flows[0].nodes[1].clone();
    let trigger = TriggerInfo {
        entity: Some(entity.clone()),
        field: None,
        value: serde_json::json!(17),
        previous: serde_json::json!(20),
    };
    let (outcome, detail, port) = run_node(
        &engine.hub,
        &engine.now,
        "avg",
        "r",
        &node,
        Some(&trigger),
        &mut BTreeMap::new(),
    )
    .await;
    assert_eq!(outcome, StepOutcome::Ok);
    assert_eq!(detail, "average 30m = 19.0 → true");
    assert_eq!(port, ["true"]);
}

#[tokio::test]
async fn switch_average_counts_every_reading_including_the_newest() {
    let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
    let entity = add_sensor(&engine, None);
    let flow = serde_json::json!({
        "id": "swavg", "name": "swavg", "enabled": true, "mode": "single",
        "nodes": [
            { "id": "t", "type": "trigger.state", "params": { "entity": entity } },
            { "id": "s", "type": "switch", "params": { "average": "30m",
                "rules": [{ "op": "lte", "value": 18 }, { "op": "lte", "value": 19 }] } }
        ],
        "edges": [{ "from": "t", "to": "s" }]
    });
    engine
        .hub
        .ingest("nidoo/automation/swavg", flow.to_string().as_bytes());
    engine.reload();
    // Only a switch registers the entity: its readings must be buffered.
    for t in [20, 20, 17] {
        engine
            .hub
            .ingest("test/room", format!(r#"{{"temperature":{t}}}"#).as_bytes());
    }
    let node = engine.flows[0].nodes[1].clone();
    let trigger = TriggerInfo {
        entity: Some(entity.clone()),
        field: None,
        value: serde_json::json!(17),
        previous: serde_json::json!(20),
    };
    let (outcome, detail, ports) = run_node(
        &engine.hub,
        &engine.now,
        "swavg",
        "r",
        &node,
        Some(&trigger),
        &mut BTreeMap::new(),
    )
    .await;
    assert_eq!(outcome, StepOutcome::Ok);
    // The mean is 19, not the current 17 (which would take rule 1).
    assert_eq!(detail, "average 30m = 19.0 → 2");
    assert_eq!(ports, ["2"]);
}

#[test]
fn math_input_average_registers_the_window() {
    let flow = Flow::decode(
        serde_json::json!({
            "id": "m", "name": "m", "enabled": true, "mode": "single",
            "nodes": [
                { "id": "t", "type": "trigger.schedule", "params": { "cron": "* * * * *" } },
                { "id": "calc", "type": "math", "params": {
                    "inputs": [
                        { "name": "a", "entity": "sensor_a", "average": "15m" },
                        { "name": "b", "entity": "sensor_b" }
                    ],
                    "formula": "a + b", "name": "sum" } }
            ],
            "edges": [{ "from": "t", "to": "calc" }]
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let windows = averaged_windows(&[flow]);
    assert_eq!(windows.len(), 1, "{windows:?}");
    assert_eq!(windows["sensor_a"], 15 * 60 * 1_000_000);
}

/// Reports a numeric sensor and returns its entity id.
fn add_sensor(engine: &Engine, value: Option<f64>) -> String {
    engine.hub.ingest(
        "nidoo/sensor/room/temperature/config",
        br#"{"unique_id":"room_temperature","state_topic":"test/room","value_template":"{{ value_json.temperature }}","unit_of_measurement":"\u00b0C"}"#,
    );
    if let Some(value) = value {
        engine.hub.ingest(
            "test/room",
            format!(r#"{{"temperature":{value}}}"#).as_bytes(),
        );
    }
    engine.hub.snapshot().entities[0].id.clone()
}

fn node(value: serde_json::Value) -> Node {
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn switch_takes_first_matching_exit() {
    let engine = engine_at(48.8566, 2.3522, "Europe/Paris");
    let entity = add_sensor(&engine, Some(20.0));
    let node = node(serde_json::json!({
        "id": "s", "type": "switch", "params": { "entity": entity, "rules": [
            { "op": "lt", "value": 18 },
            { "op": "between", "value": 18, "to": 22 },
            { "op": "gt", "value": 22 } ] }
    }));
    let (outcome, detail, ports) = run_node(
        &engine.hub,
        &engine.now,
        "f",
        "r",
        &node,
        None,
        &mut BTreeMap::new(),
    )
    .await;
    assert_eq!(outcome, StepOutcome::Ok);
    assert_eq!(ports, ["2"]);
    assert!(detail.ends_with("→ 2"), "{detail}");
}

#[tokio::test]
async fn switch_unknown_value_takes_otherwise() {
    let engine = engine_at(48.8566, 2.3522, "Europe/Paris");
    let node = node(serde_json::json!({
        "id": "s", "type": "switch", "params": {
            "entity": "nobody", "otherwise": true,
            "rules": [{ "op": "gt", "value": 1 }] }
    }));
    let (outcome, detail, ports) = run_node(
        &engine.hub,
        &engine.now,
        "f",
        "r",
        &node,
        None,
        &mut BTreeMap::new(),
    )
    .await;
    assert_eq!(outcome, StepOutcome::Ok);
    assert_eq!(ports, ["otherwise"]);
    assert_eq!(detail, "unknown → otherwise");
}

#[tokio::test]
async fn switch_nothing_fired_reads_none() {
    let engine = engine_at(48.8566, 2.3522, "Europe/Paris");
    let entity = add_sensor(&engine, Some(20.0));
    let node = node(serde_json::json!({
        "id": "s", "type": "switch", "params": {
            "entity": entity, "rules": [{ "op": "gt", "value": 99 }] }
    }));
    let (_, detail, ports) = run_node(
        &engine.hub,
        &engine.now,
        "f",
        "r",
        &node,
        None,
        &mut BTreeMap::new(),
    )
    .await;
    assert!(ports.is_empty());
    assert_eq!(detail, "20 → none");
}

/// The ids of the steps a run recorded, in order.
fn recorded(hub: &Hub, flow: &str) -> Vec<String> {
    hub.flow(flow).unwrap().runs[0]
        .steps
        .iter()
        .map(|s| s.node.clone())
        .collect()
}

#[tokio::test]
async fn switch_all_runs_exits_in_rule_order() {
    let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
    let entity = add_sensor(&engine, Some(20.0));
    let flow = serde_json::json!({
        "id": "sw", "name": "sw", "enabled": true, "mode": "single",
        "nodes": [
            { "id": "t", "type": "trigger.state", "params": { "entity": entity } },
            { "id": "s", "type": "switch", "params": { "entity": entity, "all": true,
                "rules": [{ "op": "gte", "value": 10 }, { "op": "lte", "value": 30 }] } },
            { "id": "a", "type": "notify", "params": { "title": "x" } },
            { "id": "a2", "type": "notify", "params": { "title": "x" } },
            { "id": "b", "type": "notify", "params": { "title": "x" } }
        ],
        "edges": [
            { "from": "t", "to": "s" },
            { "from": "s", "to": "b", "port": "2" },
            { "from": "s", "to": "a", "port": "1" },
            { "from": "a", "to": "a2" }
        ]
    });
    engine
        .hub
        .ingest("nidoo/automation/sw", flow.to_string().as_bytes());
    engine.reload();
    let flow = engine.flows[0].clone();
    let run = engine.hub.begin_run("sw", None).unwrap();
    execute(
        engine.hub.clone(),
        engine.now.clone(),
        flow,
        run.id,
        "t".to_owned(),
        None,
        None,
    )
    .await;
    // Exit 1 runs before exit 2 whatever the edge order, depth first.
    assert_eq!(recorded(&engine.hub, "sw"), ["t", "s", "a", "a2", "b"]);
}

/// Runs a `time.between` node with the clock at `utc`.
async fn run_time(utc: (i32, u32, u32, u32, u32), from: &str, to: &str) -> Vec<String> {
    let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
    let at = chrono::Utc
        .with_ymd_and_hms(utc.0, utc.1, utc.2, utc.3, utc.4, 0)
        .unwrap();
    engine.now = Arc::new(move || at);
    let node = node(serde_json::json!({
        "id": "tm", "type": "time.between", "params": { "from": from, "to": to }
    }));
    run_node(
        &engine.hub,
        &engine.now,
        "f",
        "r",
        &node,
        None,
        &mut BTreeMap::new(),
    )
    .await
    .2
}

#[tokio::test]
async fn time_between_uses_house_zone() {
    // 21:40 UTC is 22:40 in Paris (CET is UTC+1 only in winter; this is CET after 25 Oct).
    let now = (2026, 10, 27, 21, 40);
    assert_eq!(run_time(now, "07:00", "22:30").await, ["out"]);
    assert_eq!(run_time(now, "22:00", "06:00").await, ["in"]);
}

#[tokio::test]
async fn time_between_wall_clock_on_dst_day() {
    // 06:30 UTC on 2026-10-25 is 07:30 in Paris, after the clocks went back.
    assert_eq!(
        run_time((2026, 10, 25, 6, 30), "07:00", "08:00").await,
        ["in"]
    );
}

#[tokio::test]
async fn step_view_records_ports_for_branching_nodes_only() {
    let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
    let entity = add_sensor(&engine, Some(20.0));
    let flow = serde_json::json!({
        "id": "pv", "name": "pv", "enabled": true, "mode": "single",
        "nodes": [
            { "id": "t", "type": "trigger.state", "params": { "entity": entity } },
            { "id": "tm", "type": "time.between", "params": { "from": "00:00", "to": "23:59" } },
            { "id": "c", "type": "notify", "params": { "title": "x" } }
        ],
        "edges": [{ "from": "t", "to": "tm" }, { "from": "tm", "to": "c", "port": "in" }]
    });
    engine
        .hub
        .ingest("nidoo/automation/pv", flow.to_string().as_bytes());
    engine.reload();
    let flow = engine.flows[0].clone();
    let run = engine.hub.begin_run("pv", None).unwrap();
    execute(
        engine.hub.clone(),
        engine.now.clone(),
        flow,
        run.id,
        "t".to_owned(),
        None,
        None,
    )
    .await;
    let steps = engine.hub.flow("pv").unwrap().runs[0].steps.clone();
    assert_eq!(steps[0].ports, None);
    assert_eq!(steps[1].ports, Some(vec!["in".to_owned()]));
    assert_eq!(steps[2].node, "c");
    assert_eq!(steps[2].ports, None);
}

#[tokio::test]
async fn step_view_records_no_ports_for_a_failed_branching_step() {
    let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
    let entity = add_sensor(&engine, Some(20.0));
    let flow = serde_json::json!({
        "id": "pf", "name": "pf", "enabled": true, "mode": "single",
        "nodes": [
            { "id": "t", "type": "trigger.state", "params": { "entity": entity } },
            { "id": "tm", "type": "switch", "params": { "rules": [{ "op": "gt", "value": 1 }] } }
        ],
        "edges": [{ "from": "t", "to": "tm" }]
    });
    engine
        .hub
        .ingest("nidoo/automation/pf", flow.to_string().as_bytes());
    engine.reload();
    let flow = engine.flows[0].clone();
    let run = engine.hub.begin_run("pf", None).unwrap();
    execute(
        engine.hub.clone(),
        engine.now.clone(),
        flow,
        run.id,
        "t".to_owned(),
        None,
        None,
    )
    .await;
    let steps = engine.hub.flow("pf").unwrap().runs[0].steps.clone();
    assert_eq!(steps[1].node, "tm");
    assert_eq!(steps[1].outcome, StepOutcome::Failed);
    assert_eq!(steps[1].ports, None);
}

/// An online Paris engine with a vacuum `robo` reporting `{state, battery_level}` on
/// `test/robo`; returns the engine and the vacuum's id.
fn vacuum_engine() -> (Engine, String) {
    let engine = engine_at(48.8566, 2.3522, "Europe/Paris");
    engine.hub.ingest(
        "nidoo/vacuum/robo/config",
        br#"{"unique_id":"robo","state_topic":"test/robo","fan_speed_list":["Silent","Turbo"]}"#,
    );
    engine.hub.set_connection(ConnectionStatus::Online);
    let id = engine.hub.snapshot().entities[0].id.clone();
    (engine, id)
}

/// Publishes the vacuum's state and hands the engine the change event.
fn vacuum_report(engine: &mut Engine, id: &str, state: &str, battery: u8) {
    let payload = format!(r#"{{"state":"{state}","battery_level":{battery}}}"#);
    engine.hub.ingest("test/robo", payload.as_bytes());
    let state = engine.hub.read().state(id).cloned().expect("vacuum state");
    engine.handle_event(WsEvent::StateChanged {
        id: id.to_owned(),
        state,
    });
}

/// A flow `id` triggered by `params` of a state trigger, with nothing after it.
fn state_flow(engine: &Engine, id: &str, params: serde_json::Value) {
    let flow = serde_json::json!({
        "id": id, "name": id, "enabled": true, "mode": "single",
        "nodes": [{ "id": "t", "type": "trigger.state", "params": params }],
        "edges": []
    });
    engine.hub.ingest(
        &format!("nidoo/automation/{id}"),
        flow.to_string().as_bytes(),
    );
}

#[tokio::test]
async fn field_trigger_fires_only_on_its_field() {
    let (mut engine, id) = vacuum_engine();
    state_flow(&engine, "plain", serde_json::json!({ "entity": id }));
    state_flow(
        &engine,
        "battery",
        serde_json::json!({ "entity": id, "field": "battery" }),
    );
    engine.reload();
    vacuum_report(&mut engine, &id, "cleaning", 62);
    vacuum_report(&mut engine, &id, "cleaning", 55);
    assert_eq!(
        (run_count(&engine, "plain"), run_count(&engine, "battery")),
        (0, 1)
    );
    let trigger = engine.hub.flow("battery").unwrap().runs[0]
        .trigger
        .clone()
        .unwrap();
    assert_eq!(trigger.entity.as_deref(), Some(id.as_str()));
    assert_eq!(trigger.field.as_deref(), Some("battery"));
    assert_eq!(trigger.value, serde_json::json!(55.0));
    assert_eq!(trigger.previous, serde_json::json!(62.0));
    vacuum_report(&mut engine, &id, "docked", 55);
    assert_eq!(
        (run_count(&engine, "plain"), run_count(&engine, "battery")),
        (1, 1)
    );
}

#[tokio::test]
async fn field_trigger_seeds_silently_after_reconnect() {
    let (mut engine, id) = vacuum_engine();
    state_flow(
        &engine,
        "battery",
        serde_json::json!({ "entity": id, "field": "battery" }),
    );
    engine.reload();
    vacuum_report(&mut engine, &id, "cleaning", 62);
    engine.handle_event(WsEvent::ConnectionChanged(ConnectionStatus::Offline {
        reason: "test".to_owned(),
    }));
    engine.handle_event(WsEvent::ConnectionChanged(ConnectionStatus::Online));
    vacuum_report(&mut engine, &id, "cleaning", 55);
    assert_eq!(run_count(&engine, "battery"), 0);
    vacuum_report(&mut engine, &id, "cleaning", 50);
    assert_eq!(run_count(&engine, "battery"), 1);
}

#[tokio::test]
async fn if_without_entity_reads_the_trigger_field() {
    let (mut engine, id) = vacuum_engine();
    vacuum_report(&mut engine, &id, "cleaning", 55);
    let node = node(serde_json::json!({
        "id": "check", "type": "if", "params": { "op": "lte", "value": 55 }
    }));
    let trigger = TriggerInfo {
        entity: Some(id.clone()),
        field: Some("battery".to_owned()),
        value: serde_json::json!(55),
        previous: serde_json::json!(62),
    };
    let (outcome, detail, port) = run_node(
        &engine.hub,
        &engine.now,
        "f",
        "r",
        &node,
        Some(&trigger),
        &mut BTreeMap::new(),
    )
    .await;
    assert_eq!(outcome, StepOutcome::Ok, "{detail}");
    assert_eq!(detail, "55 → true");
    assert_eq!(port, ["true"]);
}

#[tokio::test]
async fn if_reads_entity_field() {
    let (mut engine, id) = vacuum_engine();
    vacuum_report(&mut engine, &id, "cleaning", 50);
    let node = node(serde_json::json!({
        "id": "check", "type": "if",
        "params": { "entity": id, "field": "battery", "op": "lte", "value": 55 }
    }));
    let (outcome, detail, port) = run_node(
        &engine.hub,
        &engine.now,
        "f",
        "r",
        &node,
        None,
        &mut BTreeMap::new(),
    )
    .await;
    assert_eq!(outcome, StepOutcome::Ok, "{detail}");
    assert_eq!(detail, "50 → true");
    assert_eq!(port, ["true"]);
}

#[tokio::test]
async fn switch_reads_entity_field() {
    let (mut engine, id) = vacuum_engine();
    vacuum_report(&mut engine, &id, "cleaning", 50);
    let node = node(serde_json::json!({
        "id": "s", "type": "switch", "params": { "entity": id, "field": "battery",
            "rules": [{ "op": "lte", "value": 20 }, { "op": "lte", "value": 60 }] }
    }));
    let (outcome, detail, ports) = run_node(
        &engine.hub,
        &engine.now,
        "f",
        "r",
        &node,
        None,
        &mut BTreeMap::new(),
    )
    .await;
    assert_eq!(outcome, StepOutcome::Ok, "{detail}");
    assert_eq!(ports, ["2"]);
}

#[tokio::test]
async fn if_average_reads_a_recorded_field() {
    let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
    engine.hub.ingest(
        "nidoo/climate/clim/config",
        br#"{"unique_id":"clim","mode_state_topic":"test/clim","mode_state_template":"{{ value_json.mode }}","current_temperature_topic":"test/clim","current_temperature_template":"{{ value_json.current }}","modes":["off","heat"]}"#,
    );
    let entity = engine.hub.snapshot().entities[0].id.clone();
    let flow = serde_json::json!({
        "id": "avg", "name": "avg", "enabled": true, "mode": "single",
        "nodes": [
            { "id": "t", "type": "trigger.state", "params": { "entity": entity } },
            { "id": "check", "type": "if", "params": { "entity": entity, "field": "current",
                "op": "lte", "value": 19, "average": "30m" } }
        ],
        "edges": [{ "from": "t", "to": "check" }]
    });
    engine
        .hub
        .ingest("nidoo/automation/avg", flow.to_string().as_bytes());
    engine.reload();
    for t in [20, 20, 17] {
        engine.hub.ingest(
            "test/clim",
            format!(r#"{{"mode":"heat","current":{t}}}"#).as_bytes(),
        );
    }
    let node = engine.flows[0].nodes[1].clone();
    let (outcome, detail, port) = run_node(
        &engine.hub,
        &engine.now,
        "avg",
        "r",
        &node,
        None,
        &mut BTreeMap::new(),
    )
    .await;
    assert_eq!(outcome, StepOutcome::Ok, "{detail}");
    assert_eq!(detail, "average 30m = 19.0 → true");
    assert_eq!(port, ["true"]);
}

#[tokio::test]
async fn generic_headline_trigger_fires() {
    let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
    engine.hub.ingest(
        "nidoo/lock/front/config",
        br#"{"unique_id":"front","state_topic":"test/front"}"#,
    );
    engine.hub.set_connection(ConnectionStatus::Online);
    let id = engine.hub.snapshot().entities[0].id.clone();
    state_flow(
        &engine,
        "lock",
        serde_json::json!({ "entity": id, "to": "unlocked" }),
    );
    engine.reload();
    for payload in ["LOCKED", "UNLOCKED"] {
        engine.hub.ingest("test/front", payload.as_bytes());
        let state = engine.hub.read().state(&id).cloned().expect("lock state");
        engine.handle_event(WsEvent::StateChanged {
            id: id.clone(),
            state,
        });
    }
    assert_eq!(run_count(&engine, "lock"), 1);
    assert_eq!(
        engine.hub.entity_value(&id, None),
        Some(Scalar::Text("unlocked".into()))
    );
}

/// An online Paris engine with a binary sensor `det` whose alarm is the `alarm` key of the
/// JSON on `test/det`: `{"alarm":"yes"}` is on, `{}` leaves the alarm unknown.
fn alarm_engine() -> (Engine, String) {
    let engine = engine_at(48.8566, 2.3522, "Europe/Paris");
    engine.hub.ingest(
        "nidoo/binary_sensor/det/alarm/config",
        br#"{"unique_id":"alarm","state_topic":"test/det","value_template":"{{ value_json.alarm }}","payload_on":"yes","payload_off":"no"}"#,
    );
    engine.hub.set_connection(ConnectionStatus::Online);
    let id = engine.hub.snapshot().entities[0].id.clone();
    (engine, id)
}

/// Publishes `payload` on the detector's topic and hands the engine what the hub sends: the
/// change event when the value moved, then every message signal the hub broadcast.
fn alarm_report(engine: &mut Engine, id: &str, payload: &str) {
    let mut touched = engine.hub.subscribe_state_touched();
    let changes = engine.hub.ingest("test/det", payload.as_bytes());
    if changes
        .iter()
        .any(|c| matches!(c, nidoo_core::registry::Change::StateChanged(_)))
    {
        let state = engine.hub.read().state(id).cloned().expect("state");
        engine.handle_event(WsEvent::StateChanged {
            id: id.to_owned(),
            state,
        });
    }
    while let Ok(signal) = touched.try_recv() {
        engine.state_touched(&signal);
    }
}

/// Lets every spawned run end, as the engine loop would.
async fn finish_runs(engine: &mut Engine) {
    while let Some(joined) = engine.tasks.join_next_with_id().await {
        engine.run_finished(joined);
    }
}

#[tokio::test]
async fn unknown_then_value_fires_after_a_confirming_report() {
    let (mut engine, id) = alarm_engine();
    state_flow(
        &engine,
        "a",
        serde_json::json!({ "entity": id, "to": true }),
    );
    engine.reload();
    alarm_report(&mut engine, &id, "{}");
    alarm_report(&mut engine, &id, r#"{"alarm":"yes"}"#);
    assert_eq!(run_count(&engine, "a"), 1);
    let trigger = engine.hub.flow("a").unwrap().runs[0]
        .trigger
        .clone()
        .unwrap();
    assert_eq!(trigger.value, serde_json::json!(true));
    assert_eq!(trigger.previous, serde_json::Value::Null);
}

/// An online engine with the binary sensor a Zigbee2MQTT smoke detector announces: the
/// `smoke` key of the JSON on `test/det`, `true` on and `false` off.
fn smoke_engine() -> (Engine, String) {
    let engine = engine_at(48.8566, 2.3522, "Europe/Paris");
    engine.hub.ingest(
        "nidoo/binary_sensor/det/smoke/config",
        br#"{"unique_id":"smoke","state_topic":"test/det","value_template":"{{ value_json[\"smoke\"] }}","payload_on":true,"payload_off":false}"#,
    );
    engine.hub.set_connection(ConnectionStatus::Online);
    let id = engine.hub.snapshot().entities[0].id.clone();
    (engine, id)
}

#[tokio::test]
async fn null_reports_then_true_fires_once() {
    let (mut engine, id) = smoke_engine();
    state_flow(
        &engine,
        "a",
        serde_json::json!({ "entity": id, "to": true }),
    );
    engine.reload();
    alarm_report(&mut engine, &id, r#"{"battery":13,"smoke":null}"#);
    alarm_report(&mut engine, &id, r#"{"battery":13,"smoke":null}"#);
    alarm_report(&mut engine, &id, r#"{"battery":13,"smoke":true}"#);
    assert_eq!(run_count(&engine, "a"), 1);
}

/// At boot the restore waits for the broker's replay: a message that read nothing lands
/// first and confirms the unknown, so a recorded value put back after it would look like a
/// first value. The registry refuses to restore over an entity that spoke.
#[tokio::test]
async fn restore_after_a_null_report_does_not_fire() {
    for (recorded, to) in [(true, true), (false, false)] {
        let (mut engine, id) = smoke_engine();
        state_flow(&engine, "a", serde_json::json!({ "entity": id, "to": to }));
        engine.reload();
        alarm_report(&mut engine, &id, r#"{"battery":13,"smoke":null}"#);
        let restored =
            engine
                .hub
                .restore_recorded_states(vec![(id.clone(), 0, StateValue::OnOff(recorded))]);
        assert_eq!(restored, 0, "recorded {recorded}");
        let state = engine.hub.read().state(&id).cloned().expect("state");
        assert_eq!(state.value, StateValue::Unknown);
        assert_eq!(run_count(&engine, "a"), 0, "recorded {recorded}");
        alarm_report(&mut engine, &id, r#"{"battery":13,"smoke":true}"#);
        assert_eq!(
            run_count(&engine, "a"),
            usize::from(to),
            "recorded {recorded}"
        );
    }
}

#[tokio::test]
async fn missing_key_reports_then_true_fires_once() {
    let (mut engine, id) = smoke_engine();
    state_flow(
        &engine,
        "a",
        serde_json::json!({ "entity": id, "to": true }),
    );
    engine.reload();
    alarm_report(&mut engine, &id, r#"{"battery":13}"#);
    alarm_report(&mut engine, &id, r#"{"battery":13,"smoke":true}"#);
    assert_eq!(run_count(&engine, "a"), 1);
}

#[tokio::test]
async fn a_first_true_without_a_prior_report_stays_silent() {
    let (mut engine, id) = smoke_engine();
    state_flow(
        &engine,
        "a",
        serde_json::json!({ "entity": id, "to": true }),
    );
    engine.reload();
    alarm_report(&mut engine, &id, r#"{"battery":13,"smoke":true}"#);
    assert_eq!(run_count(&engine, "a"), 0);
}

#[tokio::test]
async fn first_report_with_the_value_is_silent() {
    let (mut engine, id) = alarm_engine();
    state_flow(
        &engine,
        "a",
        serde_json::json!({ "entity": id, "to": true }),
    );
    engine.reload();
    alarm_report(&mut engine, &id, r#"{"alarm":"yes"}"#);
    assert_eq!(run_count(&engine, "a"), 0);
}

#[tokio::test]
async fn restore_of_the_to_value_does_not_fire() {
    let (mut engine, id) = alarm_engine();
    state_flow(
        &engine,
        "a",
        serde_json::json!({ "entity": id, "to": true }),
    );
    engine.reload();
    engine
        .hub
        .restore_recorded_states(vec![(id.clone(), 0, StateValue::OnOff(true))]);
    let state = engine.hub.read().state(&id).cloned().expect("state");
    engine.handle_event(WsEvent::StateChanged {
        id: id.clone(),
        state,
    });
    assert_eq!(run_count(&engine, "a"), 0);
}

#[tokio::test]
async fn from_or_no_to_stays_silent() {
    for params in [
        serde_json::json!({ "to": true, "from": false }),
        serde_json::json!({}),
    ] {
        let (mut engine, id) = alarm_engine();
        let mut params = params;
        params["entity"] = serde_json::json!(id);
        state_flow(&engine, "a", params.clone());
        engine.reload();
        alarm_report(&mut engine, &id, "{}");
        alarm_report(&mut engine, &id, r#"{"alarm":"yes"}"#);
        assert_eq!(run_count(&engine, "a"), 0, "{params}");
    }
}

#[tokio::test]
async fn vanish_and_return_fires_again() {
    let (mut engine, id) = alarm_engine();
    state_flow(
        &engine,
        "a",
        serde_json::json!({ "entity": id, "to": true }),
    );
    engine.reload();
    alarm_report(&mut engine, &id, "{}");
    alarm_report(&mut engine, &id, r#"{"alarm":"yes"}"#);
    assert_eq!(run_count(&engine, "a"), 1);
    finish_runs(&mut engine).await;
    alarm_report(&mut engine, &id, r#"{"alarm":null}"#);
    alarm_report(&mut engine, &id, r#"{"alarm":"yes"}"#);
    assert_eq!(run_count(&engine, "a"), 2);
}

#[tokio::test]
async fn link_drop_clears_the_mark() {
    let (mut engine, id) = alarm_engine();
    state_flow(
        &engine,
        "a",
        serde_json::json!({ "entity": id, "to": true }),
    );
    engine.reload();
    alarm_report(&mut engine, &id, "{}");
    engine.handle_event(WsEvent::ConnectionChanged(ConnectionStatus::Offline {
        reason: "test".to_owned(),
    }));
    engine.handle_event(WsEvent::ConnectionChanged(ConnectionStatus::Online));
    alarm_report(&mut engine, &id, r#"{"alarm":"yes"}"#);
    assert_eq!(run_count(&engine, "a"), 0);
}

type TestClock = Arc<std::sync::Mutex<chrono::DateTime<chrono::Utc>>>;

/// An online detector engine whose clock the test moves, starting at 2026-01-01 00:00 UTC.
fn clocked_engine() -> (Engine, String, TestClock) {
    let (mut engine, id) = alarm_engine();
    let clock = Arc::new(std::sync::Mutex::new(
        chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    ));
    let read = Arc::clone(&clock);
    engine.now = Arc::new(move || *read.lock().unwrap());
    (engine, id, clock)
}

/// Moves the clock to `minutes` after 2026-01-01 00:00 UTC.
fn at(clock: &TestClock, minutes: i64) {
    *clock.lock().unwrap() = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
        + chrono::Duration::minutes(minutes);
}

/// Stores flow `id` with a silent trigger on `entity` for `for_`, then a delay of `delay`.
fn silent_flow(engine: &Engine, id: &str, name: &str, for_: &str, delay: &str, entity: &str) {
    let flow = serde_json::json!({
        "id": id, "name": name, "enabled": true, "mode": "single",
        "nodes": [
            { "id": "t", "type": "trigger.silent", "params": { "entity": entity, "for": for_ } },
            { "id": "d", "type": "delay", "params": { "for": delay } }
        ],
        "edges": [{ "from": "t", "to": "d" }]
    });
    engine.hub.ingest(
        &format!("nidoo/automation/{id}"),
        flow.to_string().as_bytes(),
    );
}

/// An entity report, as the engine's `StateTouched` branch gets it.
fn report(engine: &mut Engine, id: &str) {
    let epoch = engine.hub.link_epoch();
    engine.state_touched(&StateTouched {
        id: id.to_owned(),
        epoch,
        read: true,
    });
}

#[tokio::test(start_paused = true)]
async fn silent_fires_once_after_for() {
    let (mut engine, id, clock) = clocked_engine();
    silent_flow(&engine, "s", "s", "50m", "1s", &id);
    engine.reload();
    at(&clock, 49);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 0);
    at(&clock, 50);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 1);
    finish_runs(&mut engine).await;
    at(&clock, 180);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 1);
    let run = &engine.hub.flow("s").unwrap().runs[0];
    let trigger = run.trigger.clone().unwrap();
    assert_eq!(trigger.entity.as_deref(), Some(id.as_str()));
    assert_eq!(trigger.value, serde_json::json!("50m"));
    assert_eq!(run.steps[0].detail, format!("{id} silent for 50m"));
}

#[tokio::test(start_paused = true)]
async fn messages_that_read_nothing_restart_the_clock() {
    let (mut engine, id, clock) = clocked_engine();
    silent_flow(&engine, "s", "s", "50m", "1s", &id);
    engine.reload();
    for minutes in [40, 80, 120] {
        at(&clock, minutes);
        alarm_report(&mut engine, &id, r#"{"battery":13}"#);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 0, "at {minutes}m");
    }
    at(&clock, 170);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 1);
}

#[tokio::test(start_paused = true)]
async fn report_restarts_the_clock() {
    let (mut engine, id, clock) = clocked_engine();
    silent_flow(&engine, "s", "s", "50m", "1s", &id);
    engine.reload();
    at(&clock, 40);
    report(&mut engine, &id);
    at(&clock, 50);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 0);
    at(&clock, 90);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 1);
}

#[tokio::test(start_paused = true)]
async fn fires_again_after_a_report_and_new_silence() {
    let (mut engine, id, clock) = clocked_engine();
    silent_flow(&engine, "s", "s", "50m", "1s", &id);
    engine.reload();
    at(&clock, 50);
    engine.tick_silents();
    finish_runs(&mut engine).await;
    at(&clock, 120);
    report(&mut engine, &id);
    at(&clock, 170);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 2);
}

#[tokio::test(start_paused = true)]
async fn silent_counts_from_arming_not_last_update() {
    let (mut engine, id, clock) = clocked_engine();
    alarm_report(&mut engine, &id, r#"{"alarm":"no"}"#);
    at(&clock, 600);
    silent_flow(&engine, "s", "s", "50m", "1s", &id);
    engine.reload();
    at(&clock, 601);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 0);
}

#[tokio::test(start_paused = true)]
async fn silent_starts_at_enable() {
    let (mut engine, id, clock) = clocked_engine();
    silent_flow(&engine, "s", "s", "50m", "1s", &id);
    let mut flow = engine.hub.flow_definitions().remove(0);
    flow.enabled = false;
    engine
        .hub
        .ingest("nidoo/automation/s", flow.encode().as_bytes());
    engine.reload();
    at(&clock, 300);
    flow.enabled = true;
    engine
        .hub
        .ingest("nidoo/automation/s", flow.encode().as_bytes());
    engine.reload();
    at(&clock, 300 + 49);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 0);
    at(&clock, 300 + 50);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 1);
}

#[tokio::test(start_paused = true)]
async fn silent_frozen_while_link_down_and_restarts_on_link_up() {
    let (mut engine, id, clock) = clocked_engine();
    silent_flow(&engine, "s", "s", "50m", "1s", &id);
    engine.reload();
    at(&clock, 10);
    let down = ConnectionStatus::Offline {
        reason: "test".to_owned(),
    };
    engine.hub.set_connection(down.clone());
    engine.handle_event(WsEvent::ConnectionChanged(down));
    assert!(engine.next_deadline().is_none());
    at(&clock, 120);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 0);
    engine.hub.set_connection(ConnectionStatus::Online);
    engine.handle_event(WsEvent::ConnectionChanged(ConnectionStatus::Online));
    assert!(engine.next_deadline().is_some());
    at(&clock, 120 + 49);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 0);
    at(&clock, 120 + 50);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 1);
}

#[tokio::test(start_paused = true)]
async fn silent_clock_survives_unrelated_edit() {
    let (mut engine, id, clock) = clocked_engine();
    silent_flow(&engine, "s", "s", "50m", "1s", &id);
    engine.reload();
    at(&clock, 30);
    silent_flow(&engine, "s", "renamed", "50m", "1s", &id);
    engine.reload();
    at(&clock, 50);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 1);

    let (mut engine, id, clock) = clocked_engine();
    silent_flow(&engine, "s", "s", "50m", "1s", &id);
    engine.reload();
    at(&clock, 30);
    silent_flow(&engine, "s", "s", "1h", "1s", &id);
    engine.reload();
    at(&clock, 50);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 0);
    at(&clock, 90);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 1);
}

#[tokio::test(start_paused = true)]
async fn seen_lag_restarts_every_clock() {
    let (mut engine, id, clock) = clocked_engine();
    silent_flow(&engine, "s", "s", "50m", "1s", &id);
    engine.reload();
    at(&clock, 30);
    engine.seen_lagged();
    at(&clock, 50);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 0);
    at(&clock, 80);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 1);
}

#[tokio::test(start_paused = true)]
async fn skipped_fire_retries_after_the_run_ends() {
    let (mut engine, id, clock) = clocked_engine();
    // The first run sleeps for an hour of real time and stays active.
    silent_flow(&engine, "s", "s", "50m", "1h", &id);
    engine.reload();
    at(&clock, 50);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 1);
    at(&clock, 60);
    report(&mut engine, &id);
    at(&clock, 110);
    // Due, but the single-mode run is still active: no skipped run, no busy loop.
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 1);
    assert!(engine.next_deadline().is_none());
    finish_runs(&mut engine).await;
    assert!(engine.next_deadline().is_some());
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 2);
    assert_ne!(
        engine.hub.flow("s").unwrap().runs[0].status,
        RunStatus::Skipped
    );
    finish_runs(&mut engine).await;
    at(&clock, 400);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 2);
}

#[tokio::test(start_paused = true)]
async fn a_link_that_came_back_unseen_restarts_the_clocks() {
    let (mut engine, id, clock) = clocked_engine();
    silent_flow(&engine, "s", "s", "50m", "1s", &id);
    engine.reload();
    at(&clock, 10);
    // The hub's link bounced but the engine has not handled the Online event yet.
    engine.hub.set_connection(ConnectionStatus::Offline {
        reason: "test".to_owned(),
    });
    engine.hub.set_connection(ConnectionStatus::Online);
    at(&clock, 60);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 0);
    at(&clock, 109);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 0);
    at(&clock, 110);
    engine.tick_silents();
    assert_eq!(run_count(&engine, "s"), 1);
}

/// Stores flow `id`: a manual trigger, a limit of `max` per `per`, then a delay of 1s.
fn limit_flow(engine: &Engine, id: &str, name: &str, max: u32, per: &str) {
    let flow = serde_json::json!({
        "id": id, "name": name, "enabled": true, "mode": "single",
        "nodes": [
            { "id": "m", "type": "trigger.manual", "params": {} },
            { "id": "l", "type": "limit", "params": { "max": max, "per": per } },
            { "id": "d", "type": "delay", "params": { "for": "1s" } }
        ],
        "edges": [{ "from": "m", "to": "l" }, { "from": "l", "to": "d" }]
    });
    engine.hub.ingest(
        &format!("nidoo/automation/{id}"),
        flow.to_string().as_bytes(),
    );
}

/// Runs flow `id` by hand to its end; returns the step nodes and details and the status.
async fn run_now(engine: &mut Engine, id: &str) -> (Vec<(String, String)>, RunStatus) {
    let run = engine.hub.run_flow(id).unwrap();
    engine.run_manual(id, run.id, None);
    finish_runs(engine).await;
    let run = engine.hub.flow(id).unwrap().runs[0].clone();
    let steps = run
        .steps
        .iter()
        .map(|s| (s.node.clone(), s.detail.clone()))
        .collect();
    (steps, run.status)
}

#[tokio::test(start_paused = true)]
async fn limit_passes_max_times_then_stops() {
    let (mut engine, _, _) = clocked_engine();
    limit_flow(&engine, "l", "l", 2, "1h");
    engine.reload();
    for passes in 1..=2 {
        let (steps, status) = run_now(&mut engine, "l").await;
        assert_eq!(status, RunStatus::Done);
        assert_eq!(
            steps[1],
            (
                "l".to_owned(),
                format!("passed ({passes} of 2 in the last 1h)")
            )
        );
        assert_eq!(steps[2].0, "d");
    }
    let (steps, status) = run_now(&mut engine, "l").await;
    assert_eq!(status, RunStatus::Done);
    assert_eq!(steps.len(), 2);
    assert_eq!(
        steps[1],
        ("l".to_owned(), "limited: 2 in the last 1h".to_owned())
    );
}

#[tokio::test(start_paused = true)]
async fn limit_window_rolls() {
    let (mut engine, _, clock) = clocked_engine();
    limit_flow(&engine, "l", "l", 2, "1h");
    engine.reload();
    run_now(&mut engine, "l").await;
    at(&clock, 30);
    run_now(&mut engine, "l").await;
    at(&clock, 45);
    let (steps, _) = run_now(&mut engine, "l").await;
    assert!(steps[1].1.starts_with("limited:"));
    at(&clock, 61);
    let (steps, _) = run_now(&mut engine, "l").await;
    assert!(steps[1].1.starts_with("passed"), "{steps:?}");
}

#[tokio::test(start_paused = true)]
async fn limit_count_survives_flow_save() {
    let (mut engine, _, _) = clocked_engine();
    limit_flow(&engine, "l", "l", 2, "1h");
    engine.reload();
    run_now(&mut engine, "l").await;
    run_now(&mut engine, "l").await;
    limit_flow(&engine, "l", "renamed", 2, "1h");
    engine.reload();
    let (steps, _) = run_now(&mut engine, "l").await;
    assert!(steps[1].1.starts_with("limited:"), "{steps:?}");
}

#[tokio::test(start_paused = true)]
async fn limit_count_dropped_when_node_removed_or_changed() {
    let (mut engine, _, _) = clocked_engine();
    limit_flow(&engine, "l", "l", 2, "1h");
    engine.reload();
    run_now(&mut engine, "l").await;
    run_now(&mut engine, "l").await;
    limit_flow(&engine, "l", "l", 3, "1h");
    engine.reload();
    let (steps, _) = run_now(&mut engine, "l").await;
    assert_eq!(steps[1].1, "passed (1 of 3 in the last 1h)");
}

fn backup_flow(partial_wired: bool) -> Flow {
    let mut edges = vec![
        json!({ "from": "bk", "to": "yes", "port": "ok" }),
        json!({ "from": "bk", "to": "no", "port": "failed" }),
    ];
    if partial_wired {
        edges.push(json!({ "from": "bk", "to": "part", "port": "partial" }));
    }
    let tail = |id: &str| json!({ "id": id, "type": "flow.enable", "params": { "enabled": true } });
    serde_json::from_value(json!({
        "id": "f", "name": "f", "enabled": true, "mode": "single",
        "nodes": [
            { "id": "bk", "type": "backup.run", "params": {} },
            tail("yes"), tail("no"), tail("part"),
        ],
        "edges": edges,
    }))
    .unwrap()
}

fn strings(ports: &[&str]) -> Vec<String> {
    ports.iter().map(|p| (*p).to_owned()).collect()
}

#[test]
fn backup_partial_follows_partial_when_wired() {
    let flow = backup_flow(true);
    assert_eq!(
        backup_ports(&flow, "bk", strings(&["partial"])),
        ["partial"]
    );
}

#[test]
fn backup_partial_falls_back_to_ok_when_unwired() {
    let flow = backup_flow(false);
    assert_eq!(backup_ports(&flow, "bk", strings(&["partial"])), ["ok"]);
}

#[test]
fn backup_ok_and_failed_pass_through() {
    for wired in [true, false] {
        let flow = backup_flow(wired);
        assert_eq!(backup_ports(&flow, "bk", strings(&["ok"])), ["ok"]);
        assert_eq!(backup_ports(&flow, "bk", strings(&["failed"])), ["failed"]);
    }
}

#[test]
fn backup_detail_names_the_missing_zigbee_data() {
    assert_eq!(
        backup_detail("1.2 MB", "nas", "b.nidoo", None),
        "Backed up 1.2 MB to nas as b.nidoo"
    );
    assert_eq!(
        backup_detail("1.2 MB", "nas", "b.nidoo", Some("Zigbee2MQTT is offline")),
        "Backed up 1.2 MB to nas as b.nidoo, without Zigbee2MQTT data: Zigbee2MQTT is offline"
    );
}

/// A vacuum engine whose clock the test moves; its `battery` field is the number watched.
fn battery_engine() -> (Engine, String, TestClock) {
    let (mut engine, id) = vacuum_engine();
    let clock = Arc::new(std::sync::Mutex::new(
        chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    ));
    let read = Arc::clone(&clock);
    engine.now = Arc::new(move || *read.lock().unwrap());
    (engine, id, clock)
}

#[tokio::test]
async fn above_fires_once_per_crossing() {
    let (mut engine, id, _) = battery_engine();
    state_flow(
        &engine,
        "a",
        serde_json::json!({ "entity": id, "field": "battery", "above": 25 }),
    );
    engine.reload();
    vacuum_report(&mut engine, &id, "cleaning", 24);
    vacuum_report(&mut engine, &id, "cleaning", 26);
    assert_eq!(run_count(&engine, "a"), 1);
    finish_runs(&mut engine).await;
    vacuum_report(&mut engine, &id, "cleaning", 27);
    assert_eq!(run_count(&engine, "a"), 1);
    vacuum_report(&mut engine, &id, "cleaning", 24);
    vacuum_report(&mut engine, &id, "cleaning", 26);
    assert_eq!(run_count(&engine, "a"), 2);
}

#[tokio::test]
async fn above_and_below_fire_on_entering_the_range() {
    let (mut engine, id, _) = battery_engine();
    state_flow(
        &engine,
        "a",
        serde_json::json!({ "entity": id, "field": "battery", "above": 18, "below": 22 }),
    );
    engine.reload();
    vacuum_report(&mut engine, &id, "cleaning", 15);
    vacuum_report(&mut engine, &id, "cleaning", 20);
    assert_eq!(run_count(&engine, "a"), 1);
    finish_runs(&mut engine).await;
    vacuum_report(&mut engine, &id, "cleaning", 21);
    assert_eq!(run_count(&engine, "a"), 1);
    vacuum_report(&mut engine, &id, "cleaning", 23);
    vacuum_report(&mut engine, &id, "cleaning", 20);
    assert_eq!(run_count(&engine, "a"), 2);
}

fn held_off_flow(engine: &Engine, id: &str) {
    state_flow(
        engine,
        "a",
        serde_json::json!({ "entity": id, "to": false, "for": "5m" }),
    );
}

#[tokio::test]
async fn to_with_for_fires_after_the_wait() {
    let (mut engine, id, clock) = clocked_engine();
    held_off_flow(&engine, &id);
    engine.reload();
    alarm_report(&mut engine, &id, r#"{"alarm":"yes"}"#);
    alarm_report(&mut engine, &id, r#"{"alarm":"no"}"#);
    assert_eq!(run_count(&engine, "a"), 0);
    assert!(engine.next_deadline().is_some());
    at(&clock, 4);
    engine.tick_holds();
    assert_eq!(run_count(&engine, "a"), 0);
    at(&clock, 5);
    engine.tick_holds();
    assert_eq!(run_count(&engine, "a"), 1);
    assert!(engine.holds.is_empty());
    let trigger = engine.hub.flow("a").unwrap().runs[0]
        .trigger
        .clone()
        .unwrap();
    assert_eq!(trigger.value, serde_json::json!(false));
}

#[tokio::test]
async fn a_report_that_breaks_the_condition_cancels_the_wait() {
    let (mut engine, id, clock) = clocked_engine();
    held_off_flow(&engine, &id);
    engine.reload();
    alarm_report(&mut engine, &id, r#"{"alarm":"yes"}"#);
    alarm_report(&mut engine, &id, r#"{"alarm":"no"}"#);
    at(&clock, 2);
    alarm_report(&mut engine, &id, r#"{"alarm":"yes"}"#);
    assert!(engine.holds.is_empty());
    at(&clock, 6);
    engine.tick_holds();
    assert_eq!(run_count(&engine, "a"), 0);
}

#[tokio::test]
async fn for_without_a_filter_restarts_on_a_new_change() {
    let (mut engine, id, clock) = battery_engine();
    state_flow(
        &engine,
        "a",
        serde_json::json!({ "entity": id, "field": "battery", "for": "5m" }),
    );
    engine.reload();
    vacuum_report(&mut engine, &id, "cleaning", 10);
    vacuum_report(&mut engine, &id, "cleaning", 11);
    at(&clock, 3);
    vacuum_report(&mut engine, &id, "cleaning", 12);
    at(&clock, 5);
    engine.tick_holds();
    assert_eq!(run_count(&engine, "a"), 0);
    at(&clock, 8);
    engine.tick_holds();
    assert_eq!(run_count(&engine, "a"), 1);
    let trigger = engine.hub.flow("a").unwrap().runs[0]
        .trigger
        .clone()
        .unwrap();
    assert_eq!(trigger.value, serde_json::json!(12.0));
}

#[tokio::test]
async fn the_deadline_rechecks_the_registry() {
    let (mut engine, id, clock) = clocked_engine();
    held_off_flow(&engine, &id);
    engine.reload();
    alarm_report(&mut engine, &id, r#"{"alarm":"yes"}"#);
    alarm_report(&mut engine, &id, r#"{"alarm":"no"}"#);
    // The registry moves without the engine hearing about it.
    engine.hub.ingest("test/det", br#"{"alarm":"yes"}"#);
    at(&clock, 5);
    engine.tick_holds();
    assert_eq!(run_count(&engine, "a"), 0);
    assert!(engine.holds.is_empty());
}

#[tokio::test]
async fn reload_keeps_an_unchanged_hold_and_drops_an_edited_one() {
    let (mut engine, id, _) = clocked_engine();
    held_off_flow(&engine, &id);
    engine.reload();
    alarm_report(&mut engine, &id, r#"{"alarm":"yes"}"#);
    alarm_report(&mut engine, &id, r#"{"alarm":"no"}"#);
    assert_eq!(engine.holds.len(), 1);
    engine.reload();
    assert_eq!(engine.holds.len(), 1);
    state_flow(
        &engine,
        "a",
        serde_json::json!({ "entity": id, "to": false, "for": "10m" }),
    );
    engine.reload();
    assert!(engine.holds.is_empty());
}
