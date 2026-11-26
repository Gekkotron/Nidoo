use super::*;
use nidoo_api::Span;
use serde_json::json;

fn payload(id: &str, order: u32) -> Vec<u8> {
    json!({"id": id, "name": id, "order": order, "widgets": [
        {"id": "w1", "kind": "light", "span": "1x1", "config": {"entity_id": "light.a"}}
    ]})
    .to_string()
    .into_bytes()
}

fn widget(kind: WidgetKind, span: Span, config: WidgetConfig) -> Widget {
    Widget {
        id: "w".into(),
        kind,
        span,
        config,
        pos: None,
    }
}

fn entity_config() -> WidgetConfig {
    WidgetConfig::Entity {
        entity_id: "light.a".into(),
        title: None,
    }
}

#[test]
fn apply_empty_payload_removes() {
    let mut store = DashboardStore::new();
    assert_eq!(store.apply("d1", &payload("d1", 1)), StoreApplied::Changed);
    assert_eq!(store.apply("d1", b""), StoreApplied::Removed);
    assert_eq!(store.apply("d1", b""), StoreApplied::NotFound);
    assert!(store.get("d1").is_none());
}

#[test]
fn apply_identical_is_unchanged() {
    let mut store = DashboardStore::new();
    let bytes = payload("d1", 1);
    assert_eq!(store.apply("d1", &bytes), StoreApplied::Changed);
    assert_eq!(store.apply("d1", &bytes), StoreApplied::Unchanged);
}

#[test]
fn invalid_payload_is_rejected_with_reason() {
    let mut store = DashboardStore::new();
    assert_eq!(store.apply("d1", b"{"), StoreApplied::Changed);
    assert!(!store.invalid_reason("d1").expect("invalid").is_empty());
    assert!(store.get("d1").is_none());
    assert!(store.list().is_empty());
    assert_eq!(store.apply("d1", b"{"), StoreApplied::Unchanged);
}

#[test]
fn widget_with_wrong_span_rejected() {
    let w = widget(WidgetKind::Light, Span::X2x2, entity_config());
    let err = validate_widget(&w).unwrap_err();
    assert!(err.contains("span 2x2 not allowed for kind light"), "{err}");
    assert!(validate_widget(&widget(WidgetKind::Light, Span::X2x1, entity_config())).is_ok());

    let mut store = DashboardStore::new();
    let bytes = json!({"id": "d1", "name": "d1", "order": 1, "widgets": [
        {"id": "w9", "kind": "light", "span": "2x2", "config": {"entity_id": "light.a"}}
    ]})
    .to_string();
    store.apply("d1", bytes.as_bytes());
    let reason = store.invalid_reason("d1").expect("invalid");
    assert!(
        reason.contains("w9") && reason.contains("light"),
        "{reason}"
    );
}

#[test]
fn widget_config_shape_mismatch_rejected() {
    let area = WidgetConfig::Area {
        area_id: "kitchen".into(),
        title: None,
    };
    let err = validate_widget(&widget(WidgetKind::Light, Span::X1x1, area.clone())).unwrap_err();
    assert!(
        err.contains("config shape does not match kind light"),
        "{err}"
    );
    assert!(validate_widget(&widget(WidgetKind::RoomGroup, Span::X1x1, area)).is_ok());
    assert!(validate_widget(&widget(WidgetKind::Camera, Span::X2x1, entity_config())).is_err());
}

fn cameras(ids: &[&str]) -> WidgetConfig {
    WidgetConfig::Cameras {
        cameras: ids.iter().map(|id| (*id).to_owned()).collect(),
        title: None,
    }
}

#[test]
fn camera_widget_rules() {
    let camera = |config| validate_widget(&widget(WidgetKind::Camera, Span::X2x2, config));
    assert!(camera(cameras(&["front_door"])).is_ok());
    assert!(camera(cameras(&["a", "b", "c", "d"])).is_ok());
    assert!(camera(cameras(&[])).is_err());
    assert!(camera(cameras(&["a", "b", "c", "d", "e"])).is_err());
    assert!(camera(cameras(&["a", "a"])).is_err());
    assert!(camera(cameras(&[""])).is_err());
    assert!(camera(entity_config()).is_err());
    assert!(
        validate_widget(&widget(
            WidgetKind::Lock,
            Span::X1x1,
            cameras(&["front_door"])
        ))
        .is_err()
    );
}

#[test]
fn flow_button_requires_flow_config() {
    let flow = WidgetConfig::Flow {
        flow_id: "morning".into(),
        title: None,
    };
    assert!(validate_widget(&widget(WidgetKind::FlowButton, Span::X1x1, flow)).is_ok());
    assert!(validate_widget(&widget(WidgetKind::FlowButton, Span::X1x1, entity_config())).is_err());
}

#[test]
fn list_sorted_by_order() {
    let mut store = DashboardStore::new();
    store.apply("c", &payload("c", 3));
    store.apply("a", &payload("a", 1));
    store.apply("b", &payload("b", 2));
    let ids: Vec<_> = store.list().into_iter().map(|d| d.id).collect();
    assert_eq!(ids, ["a", "b", "c"]);
}

#[test]
fn list_breaks_order_ties_by_id() {
    let mut store = DashboardStore::new();
    store.apply("b", &payload("b", 1));
    store.apply("c", &payload("c", 1));
    store.apply("a", &payload("a", 1));
    let ids: Vec<_> = store.list().into_iter().map(|d| d.id).collect();
    assert_eq!(ids, ["a", "b", "c"]);
}

#[test]
fn next_free_order_starts_at_one_and_fills_above_max() {
    let mut store = DashboardStore::new();
    assert_eq!(store.next_free_order(), 1);
    store.apply("a", &payload("a", 1));
    store.apply("b", &payload("b", 7));
    assert_eq!(store.next_free_order(), 8);
}

#[test]
fn house_id_rejected_on_apply() {
    let mut store = DashboardStore::new();
    assert_eq!(
        store.apply("house", &payload("house", 1)),
        StoreApplied::Changed
    );
    let reason = store.invalid_reason("house").expect("invalid");
    assert!(reason.contains("house"), "{reason}");
    assert!(store.get("house").is_none());
    assert!(store.list().is_empty());
}

#[test]
fn apply_decodes_dashboard_with_unknown_widget_kind() {
    let mut store = DashboardStore::new();
    let bytes = json!({"id": "d1", "name": "d1", "order": 1, "widgets": [
        {"id": "w1", "kind": "light", "span": "1x1", "config": {"entity_id": "light.a"}},
        {"id": "w2", "kind": "bogus_kind_xyz", "span": "1x1", "config": {"entity_id": "x"}}
    ]})
    .to_string();
    assert_eq!(store.apply("d1", bytes.as_bytes()), StoreApplied::Changed);
    let dashboard = store.get("d1").expect("valid entry");
    assert_eq!(dashboard.widgets.len(), 1);
    assert_eq!(dashboard.widgets[0].id, "w1");
    assert_eq!(store.list().len(), 1);
}

#[test]
fn apply_fills_missing_id_from_topic() {
    let mut store = DashboardStore::new();
    let bytes = br#"{"name":"X","order":1,"widgets":[]}"#;
    assert_eq!(store.apply("morning", bytes), StoreApplied::Changed);
    assert_eq!(store.get("morning").expect("valid").id, "morning");
}

#[test]
fn apply_rejects_payload_with_mismatched_id() {
    let mut store = DashboardStore::new();
    let bytes = br#"{"id":"other","name":"X","order":1,"widgets":[]}"#;
    store.apply("morning", bytes);
    let reason = store.invalid_reason("morning").expect("invalid");
    assert!(
        reason.contains("other") && reason.contains("morning"),
        "{reason}"
    );
    assert!(store.get("morning").is_none());
}
