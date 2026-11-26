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
        | WidgetKind::WaterHeater
        | WidgetKind::MediaPlayer => matches!(w.config, WidgetConfig::Entity { .. }),
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
mod tests;
