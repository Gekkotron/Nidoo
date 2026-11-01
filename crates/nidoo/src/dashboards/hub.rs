//! The dashboard side of the hub: the read views and the writers behind the REST layer.

use nidoo_api::{Dashboard, DashboardView, HOUSE_ID};
use nidoo_core::command::Publish;
use nidoo_core::dashboard::{is_valid_dashboard_id, validate_layout};

use super::house::synthesize_house;
use super::store::validate_widget;
use crate::hub::{Hub, HubError};

impl Hub {
    /// House synthesized at index 0, then the authored dashboards sorted by `order`.
    pub fn dashboards(&self) -> Vec<DashboardView> {
        let mut out = vec![house_view()];
        out.extend(self.dashboard_store().list().into_iter().map(authored_view));
        out
    }

    pub fn dashboard(&self, id: &str) -> Option<DashboardView> {
        if id == HOUSE_ID {
            return Some(house_view());
        }
        self.dashboard_store().get(id).cloned().map(authored_view)
    }

    /// Create-or-replace: validates, normalizes a colliding `order`, publishes retained and
    /// applies locally so the next read sees it before the broker echo arrives.
    pub async fn set_dashboard(&self, mut body: Dashboard) -> Result<Dashboard, HubError> {
        if body.id == HOUSE_ID {
            return Err(reserved());
        }
        if !is_valid_dashboard_id(&body.id) {
            return Err(HubError::Rejected(format!(
                "invalid dashboard id `{}`",
                body.id
            )));
        }
        for widget in &body.widgets {
            validate_widget(widget).map_err(|problem| {
                HubError::Rejected(format!("widget `{}`: {problem}", widget.id))
            })?;
        }
        validate_layout(&body.widgets).map_err(HubError::Rejected)?;
        let _write = self.dashboards_write();
        {
            let store = self.dashboard_store();
            let taken = store
                .list()
                .iter()
                .any(|d| d.id != body.id && d.order == body.order);
            if body.order == 0 || taken {
                body.order = store.next_free_order();
            }
        }
        self.write_dashboard(&body)?;
        Ok(body)
    }

    /// Removes a dashboard by publishing an empty retained payload and applying it locally.
    pub async fn delete_dashboard(&self, id: &str) -> Result<(), HubError> {
        if id == HOUSE_ID {
            return Err(reserved());
        }
        let known = {
            let store = self.dashboard_store();
            store.get(id).is_some() || store.invalid_reason(id).is_some()
        };
        if !known {
            return Err(HubError::UnknownDashboard(id.to_owned()));
        }
        self.publish(Publish {
            topic: self.dashboard_topic(id),
            payload: String::new(),
            retain: true,
        })?;
        self.ingest_dashboard(id, b"");
        Ok(())
    }

    /// Reassigns contiguous 1-based orders by position. The list is validated against the
    /// store before anything is published, so a stale list changes nothing. `house` is ignored.
    pub async fn reorder_dashboards(&self, ids: &[String]) -> Result<(), HubError> {
        let requested: Vec<&str> = ids
            .iter()
            .map(String::as_str)
            .filter(|id| *id != HOUSE_ID)
            .collect();
        let _write = self.dashboards_write();
        let updated: Vec<Dashboard> = {
            let store = self.dashboard_store();
            let mut current: Vec<String> = store.list().into_iter().map(|d| d.id).collect();
            let mut sorted: Vec<&str> = requested.clone();
            sorted.sort_unstable();
            current.sort_unstable();
            if sorted.len() != current.len() || sorted.iter().zip(&current).any(|(a, b)| a != b) {
                return Err(HubError::Rejected(
                    "reorder list does not match the current dashboards".to_owned(),
                ));
            }
            let mut updated = Vec::with_capacity(requested.len());
            for (index, id) in requested.iter().enumerate() {
                let mut dashboard = store
                    .get(id)
                    .cloned()
                    .ok_or_else(|| HubError::UnknownDashboard((*id).to_owned()))?;
                dashboard.order = u32::try_from(index + 1).unwrap_or(u32::MAX);
                updated.push(dashboard);
            }
            updated
        };
        // Validation above is all-or-nothing, but a publish failing mid-loop leaves the broker
        // and the local store half-reordered; they still agree, since each write publishes
        // and then ingests.
        for dashboard in &updated {
            self.write_dashboard(dashboard)?;
        }
        Ok(())
    }

    fn dashboards_write(&self) -> std::sync::MutexGuard<'_, ()> {
        self.dashboards_write
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Publishes the definition retained, then applies it locally (the echo is a no-op).
    fn write_dashboard(&self, dashboard: &Dashboard) -> Result<(), HubError> {
        let payload = serde_json::to_string(dashboard)
            .map_err(|err| HubError::Rejected(format!("serialize: {err}")))?;
        self.publish(Publish {
            topic: self.dashboard_topic(&dashboard.id),
            payload: payload.clone(),
            retain: true,
        })?;
        self.ingest_dashboard(&dashboard.id, payload.as_bytes());
        Ok(())
    }
}

fn reserved() -> HubError {
    HubError::Rejected(format!("`{HOUSE_ID}` is reserved"))
}

/// House ignores its arguments (the client renders it from the live entities), so none are
/// collected here.
fn house_view() -> DashboardView {
    DashboardView {
        dashboard: synthesize_house(&[], &[]),
        synthesized: true,
    }
}

fn authored_view(dashboard: Dashboard) -> DashboardView {
    DashboardView {
        dashboard,
        synthesized: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use tokio::sync::mpsc;

    fn hub() -> (std::sync::Arc<Hub>, mpsc::Receiver<Publish>) {
        let (tx, rx) = mpsc::channel(16);
        let hub = Hub::new(&Config::default(), tx);
        hub.set_connection(nidoo_api::ConnectionStatus::Online);
        (hub, rx)
    }

    fn dash(id: &str, order: u32) -> Dashboard {
        Dashboard {
            id: id.to_owned(),
            name: id.to_owned(),
            order,
            widgets: Vec::new(),
            show_header: false,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn set_dashboard_rejects_house_id() {
        let (hub, _rx) = hub();
        let err = hub.set_dashboard(dash("house", 1)).await.unwrap_err();
        assert!(matches!(err, HubError::Rejected(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn set_dashboard_rejects_overlapping_widgets() {
        let (hub, _rx) = hub();
        let widget = |id: &str, col| nidoo_api::Widget {
            id: id.into(),
            kind: nidoo_api::WidgetKind::Light,
            span: nidoo_api::Span::X2x1,
            config: nidoo_api::WidgetConfig::Entity {
                entity_id: "light.a".into(),
                title: None,
            },
            pos: Some(nidoo_core::dashboard::Cell { row: 0, col }),
        };
        let mut d = dash("a", 1);
        d.widgets = vec![widget("w1", 0), widget("w2", 1)];
        let err = hub.set_dashboard(d.clone()).await.unwrap_err();
        assert!(
            matches!(err, HubError::Rejected(ref m) if m.contains("overlap")),
            "{err:?}"
        );
        d.widgets[1].pos = Some(nidoo_core::dashboard::Cell { row: 0, col: 3 });
        assert!(hub.set_dashboard(d).await.is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn delete_dashboard_rejects_house() {
        let (hub, _rx) = hub();
        let err = hub.delete_dashboard("house").await.unwrap_err();
        assert!(matches!(err, HubError::Rejected(_)));
    }

    #[test]
    fn dashboards_returns_house_at_index_zero() {
        let (hub, _rx) = hub();
        let all = hub.dashboards();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].dashboard.id, "house");
        assert!(all[0].synthesized);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn set_dashboard_normalizes_colliding_order() {
        let (hub, mut rx) = hub();
        hub.set_dashboard(dash("a", 1)).await.unwrap();
        hub.set_dashboard(dash("b", 2)).await.unwrap();
        let c = hub.set_dashboard(dash("c", 1)).await.unwrap();
        assert_eq!(c.order, 3);
        assert!(rx.try_recv().is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reorder_rejects_drifted_list_and_publishes_nothing() {
        let (hub, mut rx) = hub();
        for (id, order) in [("a", 1), ("b", 2), ("c", 3)] {
            hub.set_dashboard(dash(id, order)).await.unwrap();
        }
        while rx.try_recv().is_ok() {}
        let ids = ["a", "b", "d"].map(String::from);
        assert!(hub.reorder_dashboards(&ids).await.is_err());
        assert!(rx.try_recv().is_err());
        let ids = ["c", "a", "b"].map(String::from);
        hub.reorder_dashboards(&ids).await.unwrap();
        let order: Vec<String> = hub
            .dashboards()
            .into_iter()
            .skip(1)
            .map(|v| v.dashboard.id)
            .collect();
        assert_eq!(order, ["c", "a", "b"]);
    }
}
