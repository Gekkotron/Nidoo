//! The app root: house state, contexts and the routes, framed by the sidebar shell.

use leptos::prelude::*;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::hooks::use_navigate;
use leptos_router::path;
use nidoo_core::model::{Kind, StateValue};

use crate::ambiances::about::AmbiancesAboutScreen;
use crate::ambiances::editor::AmbianceEditorScreen;
use crate::ambiances::page::AmbiancesScreen;
use crate::ambiances::report::AmbianceReportScreen;
use crate::commands::Pending;
use crate::components::confirm::Confirming;
use crate::components::now::install_now;
use crate::components::toast::{ToastQueue, Toasts};
use crate::house::House;
use crate::i18n::Lang;
use crate::i18n::errors::{dashboards_load_failed, named_failure};
use crate::i18n::install_lang;
use crate::nav::{TrackBackStack, install_nav};
use crate::screens::alert_detail::AlertDetailScreen;
use crate::screens::alerts::AlertsScreen;
use crate::screens::assistant::AssistantScreen;
use crate::screens::climate::ClimateScreen;
use crate::screens::dashboard::DashboardScreen;
use crate::screens::devices::DevicesScreen;
use crate::screens::energy::{EnergyScreen, install_history_refresh};
use crate::screens::entity_sheet::EntitySheetScreen;
use crate::screens::flow_canvas::{FlowCanvasScreen, NewFlowScreen};
use crate::screens::flows::FlowsScreen;
use crate::screens::group_sheet_v2::GroupSheetV2Screen;
use crate::screens::group_wizard::GroupWizardScreen;
use crate::screens::groups::GroupsScreen;
use crate::screens::history_detail::HistoryDetailScreen;
use crate::screens::room::RoomScreen;
use crate::screens::search::SearchScreen;
use crate::screens::security::SecurityScreen;
use crate::screens::settings::{SettingsPageScreen, SettingsScreen};
use crate::screens::solar::SolarScreen;
use crate::screens::weather::WeatherScreen;
use crate::setup::wizard::SetupScreen;
use crate::setup::{
    Applying, ApplyingOverlay, SetupApp, SetupState, redirects_to_setup, reload_to,
};
use crate::shell::Shell;
use crate::theme::install_theme;

/// Where landing goes: always the house, which the hub always lists. Authored dashboards are one
/// click away in the sidebar; making one never moves the start page.
pub(crate) fn landing_path() -> String {
    format!("/dashboards/{}", nidoo_api::HOUSE_ID)
}

/// Landing and unknown paths go to [`landing_path`]. The house renders without the dashboards
/// list, so there is nothing to wait for. `replace` keeps Back from bouncing off the redirect.
#[component]
fn HouseRedirect() -> impl IntoView {
    let navigate = use_navigate();
    let setup = use_context::<SetupState>();
    Effect::new(move |_| {
        // An unfinished setup takes the landing page; the wizard resumes where it stopped.
        let target = match setup.and_then(|s| s.0.get()) {
            Some(status) if redirects_to_setup(&status, "/") => "/setup".to_owned(),
            _ => landing_path(),
        };
        navigate(
            &target,
            leptos_router::NavigateOptions {
                replace: true,
                ..Default::default()
            },
        );
    });
}

/// True while any alarm panel is other than disarmed: armed, arming, pending or triggered. The
/// Security tab glyph takes the warning colour then, as the app's tab icon does.
pub(crate) fn any_alarm_armed<'a>(
    mut entities: impl Iterator<Item = &'a nidoo_api::EntityView>,
) -> bool {
    entities.any(|e| {
        e.kind == Kind::AlarmPanel
            && matches!(&e.state.value, StateValue::Alarm(s) if !matches!(s.as_str(), "disarmed" | "disarming" | "unknown" | ""))
    })
}

/// What mounts first: asks the hub for its mode once, then shows the setup app or the house.
/// A failed call (an older hub with no setup API) shows the house. The "Applying…" overlay sits
/// above either.
#[component]
pub fn Root() -> impl IntoView {
    let lang = install_lang();
    provide_context(lang);
    provide_context(Applying::new(reload_to));
    let state = SetupState(RwSignal::new(None));
    provide_context(state);
    let loaded = RwSignal::new(false);
    leptos::task::spawn_local(async move {
        if let Ok(status) = crate::api::setup::get_setup().await {
            state.0.set(Some(status));
        }
        loaded.set(true);
    });
    view! {
        {move || {
            loaded
                .get()
                .then(|| {
                    if state.0.with_untracked(|s| s.as_ref().is_some_and(|s| s.mode == "setup")) {
                        view! { <SetupApp /> }.into_any()
                    } else {
                        view! { <App /> }.into_any()
                    }
                })
        }}
        <ApplyingOverlay />
    }
}

#[component]
pub fn App() -> impl IntoView {
    let house = House::new();
    provide_context(house);
    // `Root` already installed the language; a test mounting `App` alone installs its own.
    let lang = use_context::<RwSignal<Lang>>().unwrap_or_else(install_lang);
    provide_context(lang);
    let toasts = Toasts::new();
    provide_context(toasts);
    provide_context(Pending::new());
    provide_context(Confirming::new());
    provide_context(install_now());
    let cameras = crate::cameras::CameraList::default();
    provide_context(cameras);
    crate::cameras::load_cameras(cameras);
    provide_context(crate::cameras::install_page_visible());
    // Dashboards are not pushed over the socket: the shell fetches them at start-up, and the
    // socket again on each reconnect.
    house.refresh_dashboards(move |err| {
        toasts.push(dashboards_load_failed(lang.get_untracked(), &err))
    });
    install_nav();
    crate::alerts::install_alerts_badge(house);
    install_history_refresh(house);
    crate::solar::measured::install_solar_measured(house);
    // A command the hub gave up on: tell the user, then clear it so the next one fires again.
    Effect::new(move |_| {
        if let Some((id, reason)) = house.command_failed.get() {
            let name = house
                .entities
                .with_untracked(|m| m.get(&id).map(|e| e.name.clone()));
            toasts.push(match name {
                Some(name) => named_failure(lang.get_untracked(), &name, &reason),
                None => reason,
            });
            house.command_failed.set(None);
        }
    });
    // The Climate link shows only while the hub's engine answers (a 503 means it is disabled).
    let climate_on = RwSignal::new(false);
    Effect::new(move |_| {
        house.climate_rev.track();
        leptos::task::spawn_local(async move {
            // Only the engine being disabled hides the link; any other failure keeps it as it was.
            match crate::api::climate::get_climate().await {
                Ok(_) => {
                    climate_on.try_set(true);
                }
                Err(e) if crate::api::climate::is_climate_disabled(&e) => {
                    climate_on.try_set(false);
                }
                Err(_) => {}
            }
        });
    });
    let theme = install_theme();
    provide_context(theme);
    let alarm_armed = Memo::new(move |_| house.entities.with(|m| any_alarm_armed(m.values())));
    let solar_active = Memo::new(move |_| {
        house
            .entities
            .with(|m| crate::energy::any_solar_active(m.values()))
    });
    // Count of alerts received since the user's last visit to /alerts. The badge in the sidebar
    // uses this: hidden at zero, capped at "99+".
    let alerts_badge = Memo::new(move |_| {
        let count = house
            .alerts
            .with(|list| crate::alerts::unseen_count(list, house.last_alerts_seen.get()));
        crate::alerts::badge_text(count)
    });
    #[cfg(target_arch = "wasm32")]
    crate::ws::connect("/ws", house);
    #[cfg(target_arch = "wasm32")]
    {
        house.refresh_flows_config();
        house.refresh_weather();
        house.refresh_solar();
        house.refresh_ambiances();
        house.refresh_home_settings();
    }
    view! {
        <Router>
            <TrackBackStack />
            <crate::alerts::TrackAlertsSeen />
            <Shell climate_on alarm_armed solar_active alerts_badge>
                <Routes fallback=HouseRedirect>
                    <Route path=path!("/") view=HouseRedirect />
                    <Route path=path!("/dashboards/:id") view=DashboardScreen />
                    <Route path=path!("/rooms/:id") view=RoomScreen />
                    <Route path=path!("/entity/:id") view=EntitySheetScreen />
                    <Route path=path!("/history/:id") view=HistoryDetailScreen />
                    <Route path=path!("/group/:id") view=GroupSheetV2Screen />
                    <Route path=path!("/energy") view=EnergyScreen />
                    <Route path=path!("/security") view=SecurityScreen />
                    <Route path=path!("/climate") view=ClimateScreen />
                    <Route path=path!("/weather") view=WeatherScreen />
                    <Route path=path!("/solar") view=SolarScreen />
                    <Route path=path!("/search") view=SearchScreen />
                    <Route path=path!("/assistant") view=AssistantScreen />
                    <Route path=path!("/alerts") view=AlertsScreen />
                    <Route path=path!("/alerts/:id") view=AlertDetailScreen />
                    <Route path=path!("/ambiances") view=AmbiancesScreen />
                    <Route path=path!("/ambiances/about") view=AmbiancesAboutScreen />
                    <Route path=path!("/ambiances/new") view=AmbianceEditorScreen />
                    <Route path=path!("/ambiances/:id") view=AmbianceEditorScreen />
                    <Route path=path!("/ambiances/:id/report") view=AmbianceReportScreen />
                    <Route path=path!("/flows") view=FlowsScreen />
                    <Route path=path!("/flows/new") view=NewFlowScreen />
                    <Route path=path!("/flows/:id") view=FlowCanvasScreen />
                    <Route path=path!("/groups") view=GroupsScreen />
                    <Route path=path!("/groups/new") view=GroupWizardScreen />
                    <Route path=path!("/groups/:id/edit") view=GroupWizardScreen />
                    <Route path=path!("/devices") view=DevicesScreen />
                    <Route path=path!("/settings") view=SettingsScreen />
                    <Route path=path!("/settings/:page") view=SettingsPageScreen />
                    <Route path=path!("/setup") view=SetupScreen />
                </Routes>
            </Shell>
            <ToastQueue />
        </Router>
    }
}

#[cfg(test)]
mod tests {
    use super::any_alarm_armed;
    use nidoo_api::EntityView;
    use nidoo_core::model::{
        Capabilities, EntityCategory, EntityState, Kind, StateValue, UiOverride,
    };

    fn panel(state: &str) -> EntityView {
        EntityView {
            id: "alarm".into(),
            device_id: None,
            name: "Alarm".into(),
            device_name: None,
            label: None,
            kind: Kind::AlarmPanel,
            component: "alarm_control_panel".to_string(),
            protocol_kind: Kind::AlarmPanel,
            device_class: None,
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: "hall".into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride::default(),
            state: EntityState {
                value: StateValue::Alarm(state.into()),
                ..Default::default()
            },
        }
    }

    #[test]
    fn armed_glyph_follows_the_panel_state() {
        assert!(!any_alarm_armed([panel("disarmed")].iter()));
        assert!(!any_alarm_armed([panel("disarming")].iter()));
        assert!(any_alarm_armed([panel("armed_away")].iter()));
        assert!(any_alarm_armed([panel("arming")].iter()));
        assert!(any_alarm_armed([panel("pending")].iter()));
        assert!(any_alarm_armed([panel("triggered")].iter()));
        assert!(!any_alarm_armed(std::iter::empty()));
    }
}
