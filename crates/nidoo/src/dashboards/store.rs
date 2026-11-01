//! The in-memory copy of the retained dashboard definitions.

use std::collections::BTreeMap;

use nidoo_api::{
    Dashboard, HOUSE_ID, MAX_CAMERAS, Widget, WidgetConfig, WidgetKind, allowed_spans,
    decode_dashboard,
};

/// What `apply` did, so the caller knows whether to republish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreApplied {
    /// The stored definition (or its invalid state) changed.
    Changed,
    /// The payload is byte-identical to the last one for this id.
    Unchanged,
    /// An empty payload removed a known dashboard.
    Removed,
    /// An empty payload for an id the store never held.
    NotFound,
}

/// A retained payload, decoded: either a usable dashboard or the reason it is not.
enum Entry {
    Valid(Dashboard),
    Invalid { reason: String },
}

/// The decoded copy of the retained dashboards.
///
/// Invalid payloads are kept (with the reason) so they are not silently lost. The House
/// dashboard is synthesized elsewhere and never stored here.
#[derive(Default)]
pub struct DashboardStore {
    entries: BTreeMap<String, Entry>,
    last_payload: BTreeMap<String, Vec<u8>>,
}

impl DashboardStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies the retained payload of the topic `<prefix>/<id>`. An empty payload removes the
    /// dashboard. Otherwise the payload is decoded and each widget validated; a failure is
    /// stored as an invalid entry. Payloads identical to the last one for this id are `Unchanged`.
    pub fn apply(&mut self, id: &str, payload: &[u8]) -> StoreApplied {
        if payload.is_empty() {
            self.last_payload.remove(id);
            return match self.entries.remove(id) {
                Some(_) => StoreApplied::Removed,
                None => StoreApplied::NotFound,
            };
        }
        if self
            .last_payload
            .get(id)
            .is_some_and(|last| last == payload)
        {
            return StoreApplied::Unchanged;
        }
        self.last_payload.insert(id.to_owned(), payload.to_vec());
        self.entries
            .insert(id.to_owned(), Self::decode(id, payload));
        StoreApplied::Changed
    }

    fn decode(id: &str, payload: &[u8]) -> Entry {
        if id == HOUSE_ID {
            return Entry::Invalid {
                reason: format!("`{HOUSE_ID}` is a reserved dashboard id"),
            };
        }
        // `decode_dashboard` drops unknown widget kinds; plain serde would reject the payload.
        let dashboard = serde_json::from_slice::<serde_json::Value>(payload)
            .map_err(|err| err.to_string())
            .and_then(|mut value| {
                // A dashboard published without an id means "the topic's id".
                if let Some(object) = value.as_object_mut()
                    && object.get("id").is_none_or(|v| v.is_null())
                {
                    object.insert("id".to_owned(), serde_json::Value::String(id.to_owned()));
                }
                decode_dashboard(value).map_err(|err| err.to_string())
            });
        let dashboard = match dashboard {
            Ok(dashboard) => dashboard,
            Err(reason) => return Entry::Invalid { reason },
        };
        if dashboard.id != id {
            return Entry::Invalid {
                reason: format!(
                    "id mismatch: payload says `{}`, topic says `{id}`",
                    dashboard.id
                ),
            };
        }
        for widget in &dashboard.widgets {
            if let Err(problem) = validate_widget(widget) {
                return Entry::Invalid {
                    reason: format!("widget `{}`: {problem}", widget.id),
                };
            }
        }
        Entry::Valid(dashboard)
    }

    /// The valid dashboards, sorted by `order` ascending (ties by id).
    pub fn list(&self) -> Vec<Dashboard> {
        let mut dashboards: Vec<Dashboard> = self
            .entries
            .values()
            .filter_map(|entry| match entry {
                Entry::Valid(dashboard) => Some(dashboard.clone()),
                Entry::Invalid { .. } => None,
            })
            .collect();
        dashboards.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
        dashboards
    }

    /// A valid dashboard by id.
    pub fn get(&self, id: &str) -> Option<&Dashboard> {
        match self.entries.get(id)? {
            Entry::Valid(dashboard) => Some(dashboard),
            Entry::Invalid { .. } => None,
        }
    }

    /// Why a stored payload is not usable; `None` for a valid or unknown id.
    pub fn invalid_reason(&self, id: &str) -> Option<&str> {
        match self.entries.get(id)? {
            Entry::Invalid { reason } => Some(reason),
            Entry::Valid(_) => None,
        }
    }

    /// One above the highest `order` among valid dashboards; 1 when there are none.
    pub fn next_free_order(&self) -> u32 {
        self.entries
            .values()
            .filter_map(|entry| match entry {
                Entry::Valid(dashboard) => Some(dashboard.order),
                Entry::Invalid { .. } => None,
            })
            .max()
            .map_or(1, |max| max.saturating_add(1))
    }
}

fn kind_name(kind: WidgetKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{kind:?}"))
}

/// Checks a widget's span against its kind's allowed spans and its config shape against its kind.
pub fn validate_widget(w: &Widget) -> Result<(), String> {
    let kind = kind_name(w.kind);
    if !allowed_spans(w.kind).contains(&w.span) {
        return Err(format!(
            "span {} not allowed for kind {kind}",
            w.span.as_str()
        ));
    }
    let shape_ok = match w.kind {
        WidgetKind::Light
        | WidgetKind::Cover
        | WidgetKind::Climate
        | WidgetKind::Lock
        | WidgetKind::Sensor
        | WidgetKind::WaterHeater => matches!(w.config, WidgetConfig::Entity { .. }),
        WidgetKind::Camera => match &w.config {
            WidgetConfig::Cameras { cameras, .. } => return validate_cameras(cameras),
            _ => false,
        },
        WidgetKind::FlowButton => matches!(w.config, WidgetConfig::Flow { .. }),
        WidgetKind::RoomGroup => matches!(w.config, WidgetConfig::Area { .. }),
        WidgetKind::EnergyStat => matches!(w.config, WidgetConfig::Energy { .. }),
        WidgetKind::HistoryChart => matches!(w.config, WidgetConfig::History { .. }),
    };
    if !shape_ok {
        return Err(format!("config shape does not match kind {kind}"));
    }
    Ok(())
}

/// A Camera widget holds 1 to `MAX_CAMERAS` distinct, non-empty Frigate camera ids. They are not
/// checked against Frigate, which may be down when the dashboard is saved.
fn validate_cameras(cameras: &[String]) -> Result<(), String> {
    if cameras.is_empty() || cameras.len() > MAX_CAMERAS {
        return Err(format!(
            "camera widget holds {} cameras, expected 1 to {MAX_CAMERAS}",
            cameras.len()
        ));
    }
    for (i, id) in cameras.iter().enumerate() {
        if id.is_empty() {
            return Err("camera widget holds an empty camera id".to_owned());
        }
        if cameras[..i].contains(id) {
            return Err(format!("camera widget lists camera {id} twice"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
        let err =
            validate_widget(&widget(WidgetKind::Light, Span::X1x1, area.clone())).unwrap_err();
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
        assert!(
            validate_widget(&widget(WidgetKind::FlowButton, Span::X1x1, entity_config())).is_err()
        );
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
}
