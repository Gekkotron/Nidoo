//! Dashboard wire types. Unknown widget kinds are dropped on decode (nidoo-api-side
//! wrapper, nidoo-core untouched) so a newer hub's payload never breaks an older UI.
//!
//! Do NOT use `serde_json::from_*::<Dashboard>` for payloads that cross a version boundary
//! (retained MQTT, REST responses from a newer/older peer): it errors on unknown widget
//! kinds. Use `decode_dashboard` or `DashboardView` instead.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// For cross-version payloads decode with `decode_dashboard`, not plain serde: it errors on
/// unknown widget kinds.
pub use nidoo_core::dashboard::Dashboard;
pub use nidoo_core::dashboard::{
    HOUSE_ID, HOUSE_NAME, MAX_CAMERAS, Span, Widget, WidgetConfig, WidgetKind, allowed_spans,
};

/// Decodes a dashboard, silently dropping widgets whose `kind` this build does not know.
/// Known kinds with malformed bodies still fail: only unknown kinds are tolerated.
/// Do NOT use `serde_json::from_*::<Dashboard>` across a version boundary: it errors on
/// unknown widget kinds.
pub fn decode_dashboard(mut value: Value) -> Result<Dashboard, serde_json::Error> {
    if let Some(widgets) = value.get_mut("widgets").and_then(Value::as_array_mut) {
        widgets.retain(|w| {
            w.get("kind")
                .is_none_or(|k| serde_json::from_value::<WidgetKind>(k.clone()).is_ok())
        });
    }
    serde_json::from_value(value)
}

#[derive(Serialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct DashboardView {
    pub dashboard: Dashboard,
    pub synthesized: bool,
}

impl<'de> Deserialize<'de> for DashboardView {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            dashboard: Value,
            synthesized: bool,
        }
        let raw = Raw::deserialize(d)?;
        let dashboard = decode_dashboard(raw.dashboard).map_err(serde::de::Error::custom)?;
        Ok(Self {
            dashboard,
            synthesized: raw.synthesized,
        })
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct DashboardReorder {
    pub order: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn light_widget() -> Value {
        json!({"id":"w1","kind":"light","span":"1x1","config":{"entity_id":"light.a"}})
    }

    #[test]
    fn dashboard_view_round_trips() {
        let v = json!({
            "dashboard": {"id":"d1","name":"Main","order":0,"widgets":[light_widget()]},
            "synthesized": false
        });
        let view: DashboardView = serde_json::from_value(v.clone()).unwrap();
        assert!(!view.synthesized);
        assert_eq!(view.dashboard.widgets.len(), 1);
        assert_eq!(serde_json::to_value(&view).unwrap(), v);
    }

    #[test]
    fn reorder_body_round_trips() {
        let v = json!({"order":["a","b","c"]});
        let r: DashboardReorder = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(r.order, ["a", "b", "c"]);
        assert_eq!(serde_json::to_value(&r).unwrap(), v);
    }

    #[test]
    fn unknown_widget_kind_decodes_as_placeholder() {
        let v = json!({"id":"d1","name":"Main","order":0,"widgets":[
            light_widget(),
            {"id":"w2","kind":"bogus_kind_xyz","span":"1x1","config":{"entity_id":"x"}}
        ]});
        let d = decode_dashboard(v).unwrap();
        assert_eq!(d.widgets.len(), 1);
        assert_eq!(d.widgets[0].id, "w1");

        let again = decode_dashboard(serde_json::to_value(&d).unwrap()).unwrap();
        assert_eq!(again, d);

        let view_json = json!({"dashboard": {"id":"d1","name":"Main","order":0,"widgets":[
            {"id":"w2","kind":"bogus_kind_xyz","span":"1x1","config":{"entity_id":"x"}}
        ]}, "synthesized": false});
        let view: DashboardView = serde_json::from_value(view_json).unwrap();
        assert!(view.dashboard.widgets.is_empty());
    }
}
