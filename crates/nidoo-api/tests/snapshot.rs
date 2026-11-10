//! Wire types against the recorded house.

use std::path::PathBuf;

use std::collections::BTreeMap;

use nidoo_api::{ConnectionStatus, Snapshot, WsEvent, snapshot};
use nidoo_core::model::EntityCategory;
use nidoo_core::registry::{Registry, RegistryConfig};

const AT: u64 = 1_700_000_000;

fn fixtures() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../nidoo-core/tests/fixtures/"
    ))
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

fn files(sub: &str) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = std::fs::read_dir(fixtures().join(sub))
        .expect("fixture dir")
        .map(|entry| {
            let path = entry.expect("dir entry").path();
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .expect("fixture stem")
                .to_owned();
            (stem, std::fs::read(&path).expect("fixture"))
        })
        .collect();
    out.sort();
    out
}

fn feed(registry: &mut Registry, topic: &str, payload: Vec<u8>) {
    let inbound = registry.route(topic, payload, AT);
    registry.apply(inbound);
}

fn load_house() -> Registry {
    let mut registry = Registry::new(RegistryConfig {
        discovery_prefixes: strings(&["nidoo"]),
        ui_prefix: "nidoo/ui".to_owned(),
        z2m_base: "zigbee2mqtt".to_owned(),
        reserved: strings(&["ui", "automation"]),
    });
    for (file, topic) in [
        ("bridge_info.json", "zigbee2mqtt/bridge/info"),
        ("bridge_devices.json", "zigbee2mqtt/bridge/devices"),
        ("bridge_state.json", "zigbee2mqtt/bridge/state"),
    ] {
        let payload = std::fs::read(fixtures().join(file)).expect("fixture");
        feed(&mut registry, topic, payload);
    }
    for (stem, payload) in files("discovery") {
        feed(
            &mut registry,
            &format!("nidoo/{}/config", stem.replace("__", "/")),
            payload,
        );
    }
    for (stem, payload) in files("ui") {
        feed(&mut registry, &format!("nidoo/ui/{stem}"), payload);
    }
    for (stem, payload) in files("states") {
        feed(
            &mut registry,
            &format!("zigbee2mqtt/{}", stem.replace("__", "/")),
            payload,
        );
    }
    registry
}

#[test]
fn snapshot_of_recorded_house_serializes() {
    let registry = load_house();
    let snap = snapshot(&registry, ConnectionStatus::Online, AT);
    assert_eq!(snap.generated_at, AT);
    assert!(
        snap.entities.len() >= 400,
        "entities: {}",
        snap.entities.len()
    );
    assert_eq!(snap.devices.len(), 66);
    let json = serde_json::to_string(&snap).expect("serialize");
    let back: Snapshot = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, snap);
    let with_battery = snap.devices.iter().filter(|d| d.battery.is_some()).count();
    assert!(with_battery > 0, "no device has a battery");
    let thermometer = snap
        .devices
        .iter()
        .find(|d| d.friendly_name == "rooms/bedroom2/thermometer")
        .expect("thermometer");
    assert!(thermometer.battery.is_some());
    assert!(thermometer.linkquality.is_some());
}

/// Each area counts its entities and, apart, the primary ones (what the UI shows by
/// default: primary category, enabled by default, not folded, not hidden, and a card
/// placement for the generic types).
#[test]
fn area_views_count_entities_and_primary_entities() {
    let registry = load_house();
    let snap = snapshot(&registry, ConnectionStatus::Online, AT);
    let mut entities: BTreeMap<&str, usize> = BTreeMap::new();
    let mut primary: BTreeMap<&str, usize> = BTreeMap::new();
    for entity in &snap.entities {
        *entities.entry(entity.area.as_str()).or_default() += 1;
        let card = nidoo_core::types::type_def(&entity.component)
            .is_none_or(|d| d.placement == nidoo_core::types::Placement::Card);
        if entity.is_primary() && card {
            *primary.entry(entity.area.as_str()).or_default() += 1;
        }
    }
    assert!(!snap.areas.is_empty());
    for area in &snap.areas {
        assert_eq!(
            area.entity_count,
            entities.get(area.id.as_str()).copied().unwrap_or(0),
            "{}",
            area.id
        );
        assert_eq!(
            area.primary_count,
            primary.get(area.id.as_str()).copied().unwrap_or(0),
            "{}",
            area.id
        );
    }
    let total_primary: usize = snap.areas.iter().map(|a| a.primary_count).sum();
    let total: usize = snap.areas.iter().map(|a| a.entity_count).sum();
    assert_eq!(total, snap.entities.len());
    assert!(
        total_primary > 0 && total_primary < total,
        "{total_primary}/{total}"
    );

    let diagnostic = snap
        .entities
        .iter()
        .find(|e| e.category == EntityCategory::Diagnostic)
        .expect("a diagnostic entity");
    assert!(!diagnostic.is_primary());
    let json = serde_json::to_value(&snap.areas[0]).expect("serialize");
    assert!(json["primary_count"].is_u64(), "{json}");
}

#[test]
fn ws_event_tag_shape() {
    let event = WsEvent::EntityRemoved { id: "x".to_owned() };
    assert_eq!(
        serde_json::to_string(&event).expect("serialize"),
        r#"{"type":"entity_removed","id":"x"}"#
    );
}

#[test]
fn snapshot_has_a_json_schema() {
    let schema = schemars::schema_for!(Snapshot);
    assert!(schema.as_value().is_object());
    let _ = schemars::schema_for!(WsEvent);
}

#[test]
fn secret_text_is_masked_in_the_snapshot() {
    use nidoo_core::model::{GenericValue, StateValue};
    let mut registry = load_house();
    feed(
        &mut registry,
        "nidoo/text/u1/main/config",
        br#"{"name":"Code","unique_id":"u1","mode":"password","state_topic":"t/s","command_topic":"t/set"}"#
            .to_vec(),
    );
    feed(&mut registry, "t/s", b"hunter2".to_vec());
    let snap = snapshot(&registry, ConnectionStatus::Online, AT);
    let view = snap
        .entities
        .iter()
        .find(|e| e.component == "text" && e.name == "Code")
        .expect("text entity");
    let StateValue::Generic(gs) = &view.state.value else {
        panic!("generic state: {:?}", view.state.value);
    };
    assert_eq!(
        gs.fields.get("value"),
        Some(&GenericValue::Text("••••".to_owned()))
    );
}

const FIRMWARE_ID: &str = "fw1";

/// A Z2M `update` entity (the recorded discovery shape) on the Bedroom 2 light's device, fed
/// the device payload carrying both versions.
fn bedroom2_update(
    registry: &mut Registry,
    installed: &str,
    latest: &str,
) -> nidoo_api::DeviceView {
    let mut config: serde_json::Value = serde_json::from_str(include_str!(
        "../../nidoo-core/tests/fixtures/discovery/update__0x20a716fffea4e8de__update.json"
    ))
    .expect("fixture");
    config["unique_id"] = FIRMWARE_ID.into();
    let topic = config["state_topic"]
        .as_str()
        .expect("state topic")
        .to_owned();
    feed(
        registry,
        "nidoo/update/fw1/update/config",
        config.to_string().into_bytes(),
    );
    let payload = format!(
        r#"{{"update":{{"installed_version":"{installed}","latest_version":"{latest}","state":"idle"}}}}"#
    );
    feed(registry, &topic, payload.into_bytes());
    let snap = snapshot(registry, ConnectionStatus::Online, AT);
    snap.devices
        .into_iter()
        .find(|d| d.entity_ids.iter().any(|id| id == FIRMWARE_ID))
        .expect("device with an update entity")
}

#[test]
fn device_with_newer_firmware_is_marked() {
    let mut registry = load_house();
    let device = bedroom2_update(&mut registry, "1.0", "1.1");
    let update = device.update.expect("update view");
    assert_eq!(update.installed.as_deref(), Some("1.0"));
    assert_eq!(update.latest.as_deref(), Some("1.1"));
    assert!(update.available);

    let device = bedroom2_update(&mut registry, "1.1", "1.1");
    assert!(!device.update.expect("update view").available);
}

/// Signal, target and update entities never show as Home cards, so they are not counted as
/// primary entities of their area.
#[test]
fn area_primary_count_skips_non_card_generic_placements() {
    let mut registry = load_house();
    let before = snapshot(&registry, ConnectionStatus::Online, AT);
    feed(
        &mut registry,
        "nidoo/event/ev1/press/config",
        br#"{"name":"Press","unique_id":"ev1","state_topic":"t/ev","event_types":["press"]}"#
            .to_vec(),
    );
    let after = snapshot(&registry, ConnectionStatus::Online, AT);
    let event = after
        .entities
        .iter()
        .find(|e| e.component == "event")
        .expect("event entity");
    assert!(event.is_primary());
    let sum = |s: &nidoo_api::Snapshot| s.areas.iter().map(|a| a.primary_count).sum::<usize>();
    assert_eq!(sum(&after), sum(&before));
    let entities = |s: &nidoo_api::Snapshot| s.areas.iter().map(|a| a.entity_count).sum::<usize>();
    assert_eq!(entities(&after), entities(&before) + 1);
}

#[test]
fn flow_label_wire_shapes() {
    let edit: nidoo_api::LabelEdit =
        serde_json::from_str(r#"{"op":"create_label","name":"Été"}"#).expect("edit");
    assert_eq!(
        edit,
        nidoo_api::LabelEdit::CreateLabel {
            name: "Été".to_owned(),
            choice: None
        }
    );
    let edit: nidoo_api::LabelEdit =
        serde_json::from_str(r#"{"op":"recolor_label","id":"ete","color":"teal"}"#)
            .expect("recolor");
    assert!(matches!(edit, nidoo_api::LabelEdit::RecolorLabel { .. }));
    let event = WsEvent::FlowLabelsChanged {
        labels: nidoo_api::FlowLabelsView::default(),
    };
    assert_eq!(
        serde_json::to_string(&event).expect("serialize"),
        r#"{"type":"flow_labels_changed","labels":{"catalog":{"labels":[],"choices":[]},"counts":{}}}"#
    );
    let _ = schemars::schema_for!(nidoo_api::LabelEdit);
    let _ = schemars::schema_for!(nidoo_api::FlowLabelsView);
}
