//! `climate.hold` and `climate.eco` steps on the flow engine, and the warnings a save returns.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use chrono_tz::Europe::Paris;
use nidoo::config::Config;
use nidoo::hub::Hub;
use nidoo_api::{ConnectionStatus, RunStatus, RunView, StepOutcome, WsEvent};
use nidoo_climate::doc::{OverrideOrigin, Setpoint, ZoneSettings, ZoneState};
use nidoo_core::command::Publish;
use nidoo_core::flow::Flow;
use tokio::sync::mpsc;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../nidoo-core/tests/fixtures/discovery/"
        ))
        .join(name),
    )
    .expect("fixture")
}

/// A hub with the climate engine on, running its flow engine, and zones `living` and `bedroom`
/// (one recorded Daikin each) set active.
async fn test_hub() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let mut cfg = Config::default();
    cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
    cfg.climate.enabled = true;
    cfg.flows.timezone = Some("Europe/Paris".to_owned());
    let (tx, rx) = mpsc::channel(256);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    for (n, area) in [(0u8, "living"), (1, "bedroom")] {
        let name = format!("climate__daikin_192_0_2_15{n}__climate.json");
        let topic = format!("nidoo/climate/daikin_192_0_2_15{n}/climate/config");
        hub.ingest(&topic, &fixture(&name));
        let prefix = format!("daikin_192_0_2_15{n}");
        let id = hub
            .snapshot()
            .entities
            .into_iter()
            .find(|e| e.id.starts_with(&prefix))
            .expect("entity")
            .id;
        let mut patch = serde_json::Map::new();
        patch.insert("area".to_owned(), serde_json::Value::from(area));
        hub.set_override(&id, &patch).await.expect("area");
        hub.set_climate_zone(
            area,
            ZoneSettings {
                state: ZoneState::Active,
                ..ZoneSettings::default()
            },
        )
        .expect("zone");
    }
    (hub, rx)
}

fn now() -> u64 {
    Utc::now().timestamp() as u64
}

/// A manual flow running `steps` (node type, params) in sequence from a manual trigger.
fn flow(id: &str, steps: &[(&str, serde_json::Value)]) -> String {
    let mut nodes = vec![serde_json::json!({ "id": "t", "type": "trigger.manual", "params": {} })];
    let mut edges = Vec::new();
    let mut prev = "t".to_owned();
    for (i, (kind, params)) in steps.iter().enumerate() {
        let node = format!("s{i}");
        nodes.push(serde_json::json!({ "id": node, "type": kind, "params": params }));
        edges.push(serde_json::json!({ "from": prev, "to": node }));
        prev = node;
    }
    serde_json::json!({
        "id": id, "name": id, "enabled": true, "mode": "single",
        "nodes": nodes, "edges": edges
    })
    .to_string()
}

async fn run(hub: &Arc<Hub>, id: &str, json: &str) -> RunView {
    hub.ingest(&format!("nidoo/automation/{id}"), json.as_bytes());
    let mut events = hub.subscribe();
    // Let the engine load the flow before the run is requested.
    tokio::time::sleep(Duration::from_millis(50)).await;
    hub.run_flow(id).expect("run");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let WsEvent::FlowRunChanged { run, .. } = events.recv().await.expect("open")
                && run.flow_id == id
                && run.status != RunStatus::Running
            {
                return run;
            }
        }
    })
    .await
    .expect("the run ends")
}

fn zone(hub: &Hub, id: &str) -> nidoo_climate::view::ZoneView {
    hub.climate_view()
        .unwrap()
        .zones
        .into_iter()
        .find(|z| z.zone == id)
        .expect("zone")
}

#[tokio::test]
async fn hold_sets_override_until() {
    let (hub, _rx) = test_hub().await;
    let _engine = hub.start_engine();
    let before = now();
    let json = flow(
        "h",
        &[(
            "climate.hold",
            serde_json::json!({ "zones": ["living"], "setpoint": 21, "minutes": 180 }),
        )],
    );
    let run = run(&hub, "h", &json).await;
    let step = run.steps.last().expect("step");
    assert_eq!(step.outcome, StepOutcome::Ok, "{step:?}");
    assert!(
        step.detail.starts_with("living 21 °C until"),
        "{}",
        step.detail
    );
    let overrides = zone(&hub, "living").overrides;
    assert_eq!(overrides.len(), 1, "{overrides:?}");
    let ov = &overrides[0];
    assert_eq!(ov.setpoint, Some(Setpoint::Temp(21.0)));
    assert_eq!(ov.origin, OverrideOrigin::User);
    assert!(
        (before + 10800..=now() + 10800).contains(&ov.until),
        "{}",
        ov.until
    );
}

#[tokio::test]
async fn eco_sets_room_eco_until_time() {
    let (hub, _rx) = test_hub().await;
    let _engine = hub.start_engine();
    let json = flow(
        "e",
        &[(
            "climate.eco",
            serde_json::json!({ "zones": ["bedroom"], "until": "18:00" }),
        )],
    );
    let run = run(&hub, "e", &json).await;
    let step = run.steps.last().expect("step");
    assert_eq!(step.outcome, StepOutcome::Ok, "{step:?}");
    let expected =
        nidoo_core::flow::climate::until_after(&Utc::now().with_timezone(&Paris), "18:00")
            .expect("time")
            .timestamp() as u64;
    assert_eq!(zone(&hub, "bedroom").eco.expect("eco").until, expected);
}

#[tokio::test]
async fn hold_unknown_zone_fails_with_reason() {
    let (hub, _rx) = test_hub().await;
    let _engine = hub.start_engine();
    let json = serde_json::json!({
        "id": "u", "name": "u", "enabled": true, "mode": "single",
        "nodes": [
            { "id": "t", "type": "trigger.manual", "params": {} },
            { "id": "h", "type": "climate.hold",
              "params": { "zones": ["living", "gone"], "setpoint": 20, "minutes": 60 } },
            { "id": "sib", "type": "climate.eco",
              "params": { "zones": ["bedroom"], "minutes": 60 } }
        ],
        "edges": [{ "from": "t", "to": "h" }, { "from": "t", "to": "sib" }]
    })
    .to_string();
    let run = run(&hub, "u", &json).await;
    let hold = run.steps.iter().find(|s| s.node == "h").expect("hold step");
    assert_eq!(hold.outcome, StepOutcome::Failed);
    assert!(hold.detail.contains("gone:"), "{}", hold.detail);
    assert_eq!(
        zone(&hub, "living").overrides.len(),
        1,
        "living stays applied"
    );
    let sib = run.steps.iter().find(|s| s.node == "sib").expect("sibling");
    assert_eq!(sib.outcome, StepOutcome::Ok, "{sib:?}");
    assert!(zone(&hub, "bedroom").eco.is_some());
}

#[tokio::test]
async fn hold_ends_eco() {
    let (hub, _rx) = test_hub().await;
    let _engine = hub.start_engine();
    let json = flow(
        "x",
        &[
            (
                "climate.eco",
                serde_json::json!({ "zones": ["living"], "minutes": 120 }),
            ),
            (
                "climate.hold",
                serde_json::json!({ "zones": ["living"], "setpoint": 22, "minutes": 60 }),
            ),
        ],
    );
    let run = run(&hub, "x", &json).await;
    assert!(
        run.steps.iter().all(|s| s.outcome == StepOutcome::Ok),
        "{:?}",
        run.steps
    );
    assert!(zone(&hub, "living").eco.is_none(), "the boost ends eco");
}

fn warned_flow() -> Flow {
    Flow::decode(
        serde_json::json!({
            "id": "w", "name": "w", "enabled": true, "mode": "single",
            "nodes": [
                { "id": "wake", "type": "trigger.wake_up",
                  "params": { "before_minutes": 30, "alarms": ["wake-up-9"] } },
                { "id": "hold", "type": "climate.hold",
                  "params": { "zones": ["attic"], "setpoint": 21, "minutes": 60 } }
            ],
            "edges": [{ "from": "wake", "to": "hold" }]
        })
        .to_string()
        .as_bytes(),
    )
    .expect("flow")
}

#[tokio::test]
async fn save_warns_unknown_zone_and_alarm() {
    let (hub, _rx) = test_hub().await;
    let outcome = hub.set_flow(warned_flow()).await.expect("saved");
    let warning = outcome.warning.expect("a warning");
    assert!(warning.contains("attic"), "{warning}");
    assert!(warning.contains("wake-up-9"), "{warning}");

    let mut hot = warned_flow();
    hot.nodes[1].kind = serde_json::from_value(serde_json::json!({
        "type": "climate.hold",
        "params": { "zones": ["living"], "setpoint": 99, "minutes": 60 }
    }))
    .expect("kind");
    assert!(hub.set_flow(hot).await.is_err(), "99 °C is refused");
}
