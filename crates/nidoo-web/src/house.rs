//! The reactive house: one signal per slice of [`crate::house_state::HouseState`].

use std::collections::BTreeMap;

use leptos::prelude::*;
use nidoo_api::{
    AlertView, AmbianceView, ApplyReport, AreaView, BridgeView, ConnectionStatus, Dashboard,
    DashboardView, DeviceView, EntityView, FlowView, FlowsConfigView, GroupView, HomeSettings,
    IgnoredView, SolarView, WeatherView, WsEvent,
};

#[derive(Clone, Copy)]
pub struct House {
    pub connection: RwSignal<ConnectionStatus>,
    pub bridge: RwSignal<BridgeView>,
    pub areas: RwSignal<Vec<AreaView>>,
    pub devices: RwSignal<Vec<DeviceView>>,
    pub entities: RwSignal<BTreeMap<String, EntityView>>,
    pub ignored: RwSignal<Vec<IgnoredView>>,
    /// The push alerts, newest first.
    pub alerts: RwSignal<Vec<AlertView>>,
    /// The flows with their recent runs.
    pub flows: RwSignal<Vec<FlowView>>,
    /// The hub's `[flows]` settings; `None` until the first fetch answers.
    pub flows_config: RwSignal<Option<FlowsConfigView>>,
    /// The current weather; `None` while the hub has none or a fetch failed.
    pub weather: RwSignal<Option<WeatherView>>,
    /// Whether the first weather fetch has answered (with weather or without).
    pub weather_loaded: RwSignal<bool>,
    /// The solar forecast; `None` while the hub has none or a fetch failed.
    pub solar: RwSignal<Option<SolarView>>,
    /// Whether the first solar fetch has answered (with a forecast or without).
    pub solar_loaded: RwSignal<bool>,
    /// The home settings; the default document until the first fetch answers.
    pub home_settings: RwSignal<HomeSettings>,
    /// The ambiances as the hub lists them, unreadable ones included.
    pub ambiances: RwSignal<Vec<AmbianceView>>,
    /// Whether the first ambiance fetch (or push) has landed; the Home row waits for it.
    pub ambiances_loaded: RwSignal<bool>,
    /// The last apply report per ambiance id, for this session.
    pub ambiance_reports: RwSignal<BTreeMap<String, ApplyReport>>,
    /// The hand-made groups by id (without the `group_` prefix), for the member counts on Home.
    pub groups: RwSignal<BTreeMap<String, GroupView>>,
    /// The dashboards, fetched at start-up and on each reconnect: the hub does not push their
    /// structure.
    pub dashboards: RwSignal<Vec<DashboardView>>,
    /// Whether the first dashboards fetch has answered (with a list or an error); a dashboard
    /// opened by its address shows a loading state until then.
    pub dashboards_loaded: RwSignal<bool>,
    /// The last rejected command as `(entity id, reason)`.
    pub command_failed: RwSignal<Option<(String, String)>>,
    /// Unix seconds of the user's last visit to the Alerts tab. The nav badge counts alerts
    /// newer than this. Persisted to `localStorage` by [`crate::alerts::install_alerts_badge`].
    pub last_alerts_seen: RwSignal<u64>,
    /// Bumped on every `ClimateChanged` event; the Climate screen and the nav refetch on it.
    pub climate_rev: RwSignal<u64>,
    /// Bumped on every `AlarmChanged` event; the alarm settings page and the device sheet refetch.
    pub alarm_rev: RwSignal<u64>,
    /// Bumped on every `AssistTurnAdded` event and on each snapshot (a reconnect may have
    /// missed some); the Assistant page refetches its turns.
    pub assist_rev: RwSignal<u64>,
}

impl House {
    pub fn new() -> Self {
        Self {
            connection: RwSignal::new(ConnectionStatus::default()),
            bridge: RwSignal::new(BridgeView::default()),
            areas: RwSignal::new(Vec::new()),
            devices: RwSignal::new(Vec::new()),
            entities: RwSignal::new(BTreeMap::new()),
            ignored: RwSignal::new(Vec::new()),
            alerts: RwSignal::new(Vec::new()),
            flows: RwSignal::new(Vec::new()),
            flows_config: RwSignal::new(None),
            weather: RwSignal::new(None),
            weather_loaded: RwSignal::new(false),
            solar: RwSignal::new(None),
            solar_loaded: RwSignal::new(false),
            home_settings: RwSignal::new(HomeSettings::default()),
            ambiances: RwSignal::new(Vec::new()),
            ambiances_loaded: RwSignal::new(false),
            ambiance_reports: RwSignal::new(BTreeMap::new()),
            groups: RwSignal::new(BTreeMap::new()),
            dashboards: RwSignal::new(Vec::new()),
            dashboards_loaded: RwSignal::new(false),
            command_failed: RwSignal::new(None),
            last_alerts_seen: RwSignal::new(0),
            climate_rev: RwSignal::new(0),
            alarm_rev: RwSignal::new(0),
            assist_rev: RwSignal::new(0),
        }
    }

    /// Fetches the groups into [`House::groups`]; a failure leaves the last value (the `×N`
    /// badge simply stays hidden).
    pub fn refresh_groups(&self) {
        let groups = self.groups;
        leptos::task::spawn_local(async move {
            if let Ok(list) = crate::api::list_groups().await {
                groups.set(list.into_iter().map(|g| (g.id.clone(), g)).collect());
            }
        });
    }

    /// Fetches the `[flows]` settings into [`House::flows_config`]; a failure leaves the last
    /// value (the badge simply stays hidden).
    pub fn refresh_flows_config(&self) {
        let flows_config = self.flows_config;
        leptos::task::spawn_local(async move {
            if let Ok(cfg) = crate::api::get_flows_config().await {
                flows_config.set(Some(cfg));
            }
        });
    }

    /// Fetches the solar forecast into [`House::solar`]; none, or a failure, clears it so the
    /// header hides the card.
    pub fn refresh_solar(&self) {
        let solar = self.solar;
        let loaded = self.solar_loaded;
        leptos::task::spawn_local(async move {
            solar.set(crate::api::home::get_solar().await.ok().flatten());
            loaded.set(true);
        });
    }

    /// Fetches the weather into [`House::weather`]; no weather, or a failure, clears it so the
    /// header hides the card.
    pub fn refresh_weather(&self) {
        let weather = self.weather;
        let loaded = self.weather_loaded;
        leptos::task::spawn_local(async move {
            weather.set(crate::api::home::get_weather().await.ok().flatten());
            loaded.set(true);
        });
    }

    /// Fetches the ambiances into [`House::ambiances`]; a failure leaves the last value.
    pub fn refresh_ambiances(&self) {
        let ambiances = self.ambiances;
        let loaded = self.ambiances_loaded;
        leptos::task::spawn_local(async move {
            if let Ok(list) = crate::api::ambiances::list_ambiances().await {
                ambiances.set(list);
                loaded.set(true);
            }
        });
    }

    /// Fetches the home settings into [`House::home_settings`]; a failure leaves the last value.
    pub fn refresh_home_settings(&self) {
        let home_settings = self.home_settings;
        leptos::task::spawn_local(async move {
            if let Ok(settings) = crate::api::home::get_home_settings().await {
                home_settings.set(settings);
            }
        });
    }

    /// Fetches the dashboards into [`House::dashboards`]; a failure leaves the last list and is
    /// handed to `on_error`. Either way the first answer sets [`House::dashboards_loaded`].
    pub fn refresh_dashboards(&self, on_error: impl FnOnce(String) + 'static) {
        let dashboards = self.dashboards;
        let loaded = self.dashboards_loaded;
        leptos::task::spawn_local(async move {
            match crate::api::fetch_dashboards().await {
                Ok(list) => dashboards.set(list),
                Err(err) => on_error(err.error),
            }
            // Set once, so a reconnect does not wake what waits on the first answer.
            if !loaded.get_untracked() {
                loaded.set(true);
            }
        });
    }

    /// The entities folded into `parent_id` through `part_of`, read without subscribing.
    pub fn part_of_children(&self, parent_id: &str) -> Vec<EntityView> {
        self.entities.with_untracked(|m| {
            m.values()
                .filter(|e| e.part_of.as_deref() == Some(parent_id))
                .cloned()
                .collect()
        })
    }

    /// The dashboard with matching `id`, read without subscribing.
    pub fn dashboard(&self, id: &str) -> Option<DashboardView> {
        self.dashboards
            .with_untracked(|v| v.iter().find(|d| d.dashboard.id == id).cloned())
    }

    /// Writes a dashboard the hub just saved into the list, replacing the copy with the same
    /// id or appending it. The hub does not push dashboard structure, so without this the
    /// screen would rebuild from the copy fetched at start-up.
    pub fn store_dashboard(&self, saved: Dashboard) {
        self.dashboards.update(
            |v| match v.iter_mut().find(|d| d.dashboard.id == saved.id) {
                Some(view) => view.dashboard = saved,
                None => v.push(DashboardView {
                    dashboard: saved,
                    synthesized: false,
                }),
            },
        );
    }

    /// Fans `event` out to the signals the way `HouseState::apply` patches its fields.
    pub fn apply(&self, event: WsEvent) {
        match event {
            WsEvent::Snapshot(s) => {
                self.connection.set(s.connection);
                self.bridge.set(s.bridge);
                self.areas.set(s.areas);
                self.devices.set(s.devices);
                self.entities
                    .set(s.entities.into_iter().map(|e| (e.id.clone(), e)).collect());
                self.ignored.set(s.ignored);
                self.alerts.set(s.alerts);
                self.flows.set(s.flows);
                // A snapshot follows each (re)connect: turns said meanwhile are fetched again.
                self.assist_rev.update(|r| *r += 1);
            }
            WsEvent::EntityUpserted(view) => {
                self.entities.update(|m| {
                    m.insert(view.id.clone(), view);
                });
            }
            WsEvent::EntityRemoved { id } => {
                self.entities.update(|m| {
                    m.remove(&id);
                });
            }
            WsEvent::StateChanged { id, state } => {
                self.entities.update(|m| {
                    if let Some(entity) = m.get_mut(&id) {
                        entity.state = state;
                    }
                });
            }
            WsEvent::AreasChanged { areas } => self.areas.set(areas),
            WsEvent::DevicesChanged { devices } => self.devices.set(devices),
            WsEvent::BridgeChanged(bridge) => self.bridge.set(bridge),
            WsEvent::ConnectionChanged(connection) => self.connection.set(connection),
            WsEvent::AlertsChanged { alerts } => self.alerts.set(alerts),
            WsEvent::FlowsChanged { flows } => self.flows.set(flows),
            WsEvent::FlowRunChanged { flow_id, run } => {
                self.flows
                    .update(|flows| crate::house_state::upsert_run(flows, &flow_id, run));
            }
            WsEvent::CommandFailed { id, reason } => self.command_failed.set(Some((id, reason))),
            // A press is an event for the flow engine; the UI has nothing to show for it.
            WsEvent::ButtonPressed { .. } | WsEvent::SignalReceived { .. } => {}
            // The Climate screen reads its own view: tell it to refetch.
            WsEvent::ClimateChanged { .. } => self.climate_rev.update(|r| *r += 1),
            // The alarm page reads its own view: tell it to refetch.
            WsEvent::AlarmChanged => self.alarm_rev.update(|r| *r += 1),
            // The Assistant page reads its own list: tell it to refetch.
            WsEvent::AssistTurnAdded { .. } => self.assist_rev.update(|r| *r += 1),
            WsEvent::WeatherChanged => self.refresh_weather(),
            WsEvent::HomeSettingsChanged { settings } => self.home_settings.set(settings),
            WsEvent::AmbiancesChanged { ambiances } => {
                self.ambiances.set(ambiances);
                self.ambiances_loaded.set(true);
            }
            WsEvent::AmbianceApplied { report } => self
                .ambiance_reports
                .update(|m| _ = m.insert(report.ambiance.clone(), report)),
            WsEvent::SolarChanged => self.refresh_solar(),
        }
    }
}

impl Default for House {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dashboard(id: &str, widgets: usize) -> Dashboard {
        let widget = |i: usize| nidoo_api::Widget {
            id: format!("w{i}"),
            kind: nidoo_api::WidgetKind::Light,
            span: nidoo_api::Span::X1x1,
            config: nidoo_api::WidgetConfig::Entity {
                entity_id: "light.a".into(),
                title: None,
            },
            pos: None,
        };
        Dashboard {
            id: id.into(),
            name: id.into(),
            order: 1,
            widgets: (0..widgets).map(widget).collect(),
            show_header: false,
        }
    }

    fn view(id: &str) -> AmbianceView {
        AmbianceView {
            id: id.into(),
            ambiance: None,
            invalid: Some("unreadable".into()),
        }
    }

    #[test]
    fn ambiances_changed_event_sets_list() {
        let house = House::new();
        assert!(!house.ambiances_loaded.get_untracked());
        house.apply(WsEvent::AmbiancesChanged {
            ambiances: vec![view("a"), view("b")],
        });
        assert_eq!(house.ambiances.get_untracked().len(), 2);
        assert!(house.ambiances_loaded.get_untracked());
    }

    #[test]
    fn ambiance_applied_event_stores_report() {
        let house = House::new();
        let report = |at| ApplyReport {
            ambiance: "a".into(),
            started_at: at,
            actions: Vec::new(),
        };
        house.apply(WsEvent::AmbianceApplied { report: report(1) });
        house.apply(WsEvent::AmbianceApplied { report: report(2) });
        let reports = house.ambiance_reports.get_untracked();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports["a"].started_at, 2);
    }

    #[test]
    fn home_settings_event_updates_signal() {
        let house = House::new();
        house.apply(WsEvent::HomeSettingsChanged {
            settings: HomeSettings {
                eco_setpoint: Some(16.0),
                ..HomeSettings::default()
            },
        });
        assert_eq!(house.home_settings.get_untracked().eco_setpoint, Some(16.0));
    }

    #[test]
    fn store_dashboard_replaces_the_cached_copy() {
        let house = House::new();
        house.dashboards.set(vec![DashboardView {
            dashboard: dashboard("d", 0),
            synthesized: false,
        }]);
        house.store_dashboard(dashboard("d", 2));
        assert_eq!(house.dashboard("d").unwrap().dashboard.widgets.len(), 2);
        assert_eq!(house.dashboards.get_untracked().len(), 1);
    }

    #[test]
    fn store_dashboard_appends_an_unknown_one() {
        let house = House::new();
        house.store_dashboard(dashboard("d", 1));
        let stored = house.dashboard("d").unwrap();
        assert!(!stored.synthesized);
        assert_eq!(stored.dashboard.widgets.len(), 1);
    }
}
