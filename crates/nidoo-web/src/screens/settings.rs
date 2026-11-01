//! Settings: a short list of sub-pages (the home header settings, appearance, the Zigbee2MQTT bridge, the broker, how to
//! connect an AI assistant over MCP, about), each opened at `/settings/<slug>`.

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;
use nidoo_api::{ConnectionStatus, DeviceView, SetupStatus, UpdateView};
use nidoo_core::model::Availability;

use crate::energy_view::{EnergyView, energy_view, set_energy_view};
use crate::house::House;
use crate::i18n::devices::availability_text;
use crate::i18n::settings::{
    CLAUDE_CODE_SNIPPET, CLAUDE_DESKTOP_SNIPPET, CODEX_SNIPPET, SettingsText as T, settings_text,
    updates_waiting,
};
use crate::i18n::setup::{SetupText, setup_text};
use crate::i18n::{Key, Lang, text, use_lang};
use crate::nav::BackLink;
use crate::screens::alarm_settings::{AlarmSettingsPage, settings_status};
use crate::screens::energy_preview::EnergyPreview;
use crate::screens::entity_sheet::availability_class;
use crate::screens::home_settings_panel::HomeSettingsPanel;
use crate::screens::network_settings::{NetworkSettingsPage, watched_label};
use crate::setup::connection::{DatabaseForm, MqttForm};
use crate::setup::services::basic::WeatherForm;
use crate::setup::services::keys::{FrigateForm, PushForm, SolcastForm};
use crate::setup::welcome::LocationForm;
use crate::theme::Theme;

const DASH: &str = "—";

#[component]
fn Panel(title: T, children: Children) -> impl IntoView {
    let lang = use_lang();
    view! {
        <section class="sheet__panel">
            <h2 class="sheet__heading">{move || settings_text(lang(), title)}</h2>
            {children()}
        </section>
    }
}

#[component]
fn Row(label: T, #[prop(into)] value: Signal<String>) -> impl IntoView {
    let lang = use_lang();
    view! {
        <div class="sheet__row">
            <dt>{move || settings_text(lang(), label)}</dt>
            <dd>{value}</dd>
        </div>
    }
}

fn yes_no(lang: Lang, value: bool) -> String {
    settings_text(lang, if value { T::Yes } else { T::No }).to_owned()
}

/// This page's origin, where the hub lives.
fn origin() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.location().origin().ok())
            .unwrap_or_else(|| DASH.to_owned())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::i18n::settings::NO_ORIGIN.to_owned()
    }
}

fn claude_code_snippet(origin: &str) -> String {
    CLAUDE_CODE_SNIPPET.replace("{origin}", origin)
}

fn codex_snippet(origin: &str) -> String {
    CODEX_SNIPPET.replace("{origin}", origin)
}

fn claude_desktop_snippet(origin: &str) -> String {
    // Written by hand so the keys keep the order of docs/users/assistants-mcp.md (`command` before `args`).
    CLAUDE_DESKTOP_SNIPPET.replace(
        "{origin}",
        &serde_json::Value::String(origin.to_owned()).to_string(),
    )
}

/// Writes `text` to the clipboard; false when the clipboard API is unavailable or refuses.
#[cfg(target_arch = "wasm32")]
async fn write_clipboard(text: &str) -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let navigator = window.navigator();
    let available = js_sys::Reflect::get(&navigator, &"clipboard".into())
        .map(|c| !c.is_undefined() && !c.is_null())
        .unwrap_or(false);
    if !available {
        return false;
    }
    let promise = navigator.clipboard().write_text(text);
    wasm_bindgen_futures::JsFuture::from(promise).await.is_ok()
}

#[component]
fn CopyButton(text: String) -> impl IntoView {
    let lang = use_lang();
    let copied = RwSignal::new(false);
    let on_click = move |_| {
        #[cfg(target_arch = "wasm32")]
        {
            let text = text.clone();
            spawn_local(async move {
                if write_clipboard(&text).await {
                    copied.set(true);
                    gloo_timers::future::TimeoutFuture::new(1500).await;
                    copied.set(false);
                }
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = &text;
    };
    view! {
        <button type="button" class="chip sheet__copy" on:click=on_click>
            {move || settings_text(lang(), if copied.get() { T::Copied } else { T::Copy })}
        </button>
    }
}

#[component]
fn Snippet(title: T, text: String) -> impl IntoView {
    let lang = use_lang();
    view! {
        <h3 class="sheet__subheading">{move || settings_text(lang(), title)}</h3>
        <pre class="sheet__json">{text.clone()}</pre>
        <CopyButton text=text />
    }
}

#[component]
fn ThemeSwitch() -> impl IntoView {
    let theme = expect_context::<RwSignal<Theme>>();
    let lang = use_lang();
    let segment = move |value: Theme| {
        view! {
            <button
                type="button"
                class="chip segmented__option"
                class:chip--selected=move || theme.get() == value
                aria-pressed=move || (theme.get() == value).to_string()
                on:click=move |_| theme.set(value)
            >
                {value.label()}
            </button>
        }
    };
    view! {
        <div class="segmented" role="group" aria-label=move || settings_text(lang(), T::Theme)>
            {segment(Theme::Auto)} {segment(Theme::Night)} {segment(Theme::Ivoire)}
        </div>
    }
}

#[component]
fn EnergyViewSwitch() -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let segment = move |value: EnergyView, text: T| {
        view! {
            <button
                type="button"
                class="chip segmented__option"
                class:chip--selected=move || energy_view(house) == value
                aria-pressed=move || (energy_view(house) == value).to_string()
                on:click=move |_| set_energy_view(house, value)
            >
                {move || settings_text(lang(), text)}
            </button>
        }
    };
    view! {
        <div class="segmented" role="group" aria-label=move || settings_text(lang(), T::EnergyView)>
            {segment(EnergyView::Classic, T::EnergyViewClassic)}
            {segment(EnergyView::Flux, T::EnergyViewFlux)}
            {segment(EnergyView::Orbite, T::EnergyViewOrbite)}
        </div>
    }
}

/// A Settings sub-page, at `/settings/<slug>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsPage {
    Home,
    Location,
    Weather,
    SolarForecast,
    Cameras,
    Push,
    Alarm,
    NetworkPresence,
    Appearance,
    Zigbee2mqtt,
    Broker,
    Database,
    Mcp,
    About,
}

impl SettingsPage {
    /// In the order the Settings list shows them.
    pub const ALL: [SettingsPage; 14] = [
        SettingsPage::Home,
        SettingsPage::Location,
        SettingsPage::Weather,
        SettingsPage::SolarForecast,
        SettingsPage::Cameras,
        SettingsPage::Push,
        SettingsPage::Alarm,
        SettingsPage::NetworkPresence,
        SettingsPage::Appearance,
        SettingsPage::Zigbee2mqtt,
        SettingsPage::Broker,
        SettingsPage::Database,
        SettingsPage::Mcp,
        SettingsPage::About,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            SettingsPage::Home => "home",
            SettingsPage::Location => "location",
            SettingsPage::Weather => "weather",
            SettingsPage::SolarForecast => "solar-forecast",
            SettingsPage::Cameras => "cameras",
            SettingsPage::Push => "push-notifications",
            SettingsPage::Alarm => "alarm",
            SettingsPage::NetworkPresence => "network-presence",
            SettingsPage::Appearance => "appearance",
            SettingsPage::Zigbee2mqtt => "zigbee2mqtt",
            SettingsPage::Broker => "broker",
            SettingsPage::Database => "database",
            SettingsPage::Mcp => "mcp",
            SettingsPage::About => "about",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.slug() == slug)
    }

    /// The page's name key, shared by the breadcrumb and the screen.
    pub fn key(self) -> Key {
        match self {
            SettingsPage::Home => Key::HomeSection,
            SettingsPage::Location => Key::Location,
            SettingsPage::Weather => Key::Weather,
            SettingsPage::SolarForecast => Key::SolarForecast,
            SettingsPage::Cameras => Key::Cameras,
            SettingsPage::Push => Key::PushNotifications,
            SettingsPage::Alarm => Key::Alarm,
            SettingsPage::NetworkPresence => Key::NetworkPresence,
            SettingsPage::Appearance => Key::Appearance,
            SettingsPage::Zigbee2mqtt => Key::Zigbee2mqtt,
            SettingsPage::Broker => Key::Broker,
            SettingsPage::Database => Key::Database,
            SettingsPage::Mcp => Key::AiAssistants,
            SettingsPage::About => Key::About,
        }
    }

    fn title(self, lang: Lang) -> &'static str {
        text(lang, self.key())
    }

    fn hint(self, lang: Lang) -> &'static str {
        settings_text(
            lang,
            match self {
                SettingsPage::Home => T::HintHome,
                SettingsPage::Location => T::HintLocation,
                SettingsPage::Weather => T::HintWeather,
                SettingsPage::SolarForecast => T::HintSolarForecast,
                SettingsPage::Cameras => T::HintCameras,
                SettingsPage::Push => T::HintPush,
                SettingsPage::Alarm => T::HintAlarm,
                SettingsPage::NetworkPresence => T::HintNetworkPresence,
                SettingsPage::Appearance => T::HintAppearance,
                SettingsPage::Zigbee2mqtt => T::HintZigbee,
                SettingsPage::Broker => T::HintBroker,
                SettingsPage::Database => T::HintDatabase,
                SettingsPage::Mcp => T::HintMcp,
                SettingsPage::About => T::HintAbout,
            },
        )
    }

    fn href(self) -> String {
        format!("/settings/{}", self.slug())
    }
}

const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

fn connection_word(lang: Lang, status: &ConnectionStatus) -> String {
    settings_text(
        lang,
        match status {
            ConnectionStatus::Connecting => T::Connecting,
            ConnectionStatus::Online => T::Online,
            ConnectionStatus::Offline { .. } => T::Offline,
        },
    )
    .to_owned()
}

/// The devices with a firmware update waiting, with that update.
fn pending_updates(devices: &[DeviceView]) -> Vec<(DeviceView, UpdateView)> {
    devices
        .iter()
        .filter_map(|d| {
            let u = d.update.clone().filter(|u| u.available)?;
            Some((d.clone(), u))
        })
        .collect()
}

/// The Zigbee2MQTT row's status: the waiting updates when there are any, else the bridge state.
fn zigbee_status(lang: Lang, updates: usize, state: Availability) -> String {
    match updates {
        0 => availability_text(lang, state).to_owned(),
        n => updates_waiting(lang, n),
    }
}

/// The Settings list: one row per sub-page, with its status on the right.
#[component]
pub fn SettingsScreen() -> impl IntoView {
    let house = expect_context::<House>();
    let theme = expect_context::<RwSignal<Theme>>();
    let lang = use_lang();
    let alarm = RwSignal::new(None::<nidoo_alarm::view::AlarmView>);
    Effect::new(move |_| {
        house.alarm_rev.track();
        spawn_local(async move {
            if let Ok(v) = crate::api::alarm::get_alarm().await {
                alarm.try_set(Some(v));
            }
        });
    });
    let watches = RwSignal::new(None::<Vec<nidoo_api::NetworkWatchView>>);
    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(v) = crate::api::list_network_watches().await {
                watches.try_set(Some(v));
            }
        });
    });
    let status = move |page: SettingsPage| match page {
        SettingsPage::Home
        | SettingsPage::Location
        | SettingsPage::Weather
        | SettingsPage::SolarForecast
        | SettingsPage::Cameras
        | SettingsPage::Push => String::new(),
        SettingsPage::Alarm => alarm
            .get()
            .map(|v| settings_status(&v, lang()))
            .unwrap_or_default(),
        SettingsPage::NetworkPresence => watches
            .with(|w| w.as_ref().map(|w| watched_label(w, lang())))
            .unwrap_or_default(),
        SettingsPage::Appearance => theme.get().label().to_owned(),
        SettingsPage::Zigbee2mqtt => zigbee_status(
            lang(),
            house.devices.with(|ds| pending_updates(ds).len()),
            house.bridge.with(|b| b.state),
        ),
        SettingsPage::Broker => house.connection.with(|c| connection_word(lang(), c)),
        SettingsPage::Database | SettingsPage::Mcp => String::new(),
        SettingsPage::About => VERSION.to_owned(),
    };

    view! {
        <div class="sheet">
            <h1 class="settings__title">{move || text(lang(), Key::Settings)}</h1>
            <ul class="settings__list">
                {SettingsPage::ALL
                    .into_iter()
                    .map(|page| {
                        view! {
                            <li>
                                <A href=page.href() attr:class="settings__item">
                                    <span class="settings__item-text">
                                        <span class="settings__item-title">
                                            {move || page.title(lang())}
                                        </span>
                                        <span class="settings__item-hint">
                                            {move || page.hint(lang())}
                                        </span>
                                    </span>
                                    <span class="settings__item-status">{move || status(page)}</span>
                                    <span class="settings__item-chevron" aria-hidden="true">
                                        "›"
                                    </span>
                                </A>
                            </li>
                        }
                    })
                    .collect_view()}
            </ul>
            <A href="/setup" attr:class="chip settings__all">
                {move || setup_text(lang(), SetupText::RunSetupAgain)}
            </A>
        </div>
    }
}

/// A Settings sub-page (`/settings/:page`), with a Back link to where it was opened from.
#[component]
pub fn SettingsPageScreen() -> impl IntoView {
    let lang = use_lang();
    let params = use_params_map();
    let page = Memo::new(move |_| {
        params
            .read()
            .get("page")
            .as_deref()
            .and_then(SettingsPage::from_slug)
    });
    view! {
        <div class="sheet">
            <header class="sheet__header">
                <BackLink />
                <div class="sheet__titles">
                    <h1 class="sheet__title">
                        {move || {
                            page.get()
                                .map_or_else(
                                    || text(lang(), Key::Settings),
                                    |p| p.title(lang()),
                                )
                        }}
                    </h1>
                </div>
            </header>
            {move || match page.get() {
                Some(SettingsPage::Home) => view! { <HomeSettingsPanel /> }.into_any(),
                Some(SettingsPage::Location) => view! { <LocationPage /> }.into_any(),
                Some(SettingsPage::Weather) => view! { <WeatherPage /> }.into_any(),
                Some(SettingsPage::SolarForecast) => view! { <SolarPage /> }.into_any(),
                Some(SettingsPage::Cameras) => view! { <CamerasPage /> }.into_any(),
                Some(SettingsPage::Push) => view! { <PushPage /> }.into_any(),
                Some(SettingsPage::Alarm) => view! { <AlarmSettingsPage /> }.into_any(),
                Some(SettingsPage::NetworkPresence) => {
                    view! { <NetworkSettingsPage /> }.into_any()
                }
                Some(SettingsPage::Appearance) => view! { <AppearancePage /> }.into_any(),
                Some(SettingsPage::Zigbee2mqtt) => view! { <ZigbeePage /> }.into_any(),
                Some(SettingsPage::Broker) => view! { <BrokerPage /> }.into_any(),
                Some(SettingsPage::Database) => view! { <DatabasePage /> }.into_any(),
                Some(SettingsPage::Mcp) => view! { <McpPage /> }.into_any(),
                Some(SettingsPage::About) => view! { <AboutPage /> }.into_any(),
                None => {
                    view! {
                        <p class="settings__empty">{move || settings_text(lang(), T::NoSuchPage)}</p>
                        <A href="/settings" attr:class="chip settings__all">
                            {move || settings_text(lang(), T::AllSettings)}
                        </A>
                    }
                        .into_any()
                }
            }}
        </div>
    }
}

/// The hub's setup status, read once. A failed read keeps the hub's wording so the page can
/// show it instead of a form that would save over the stored values.
fn load_status() -> RwSignal<Option<Result<SetupStatus, String>>> {
    let status = RwSignal::new(None);
    spawn_local(async move {
        let result = crate::api::setup::get_setup().await.map_err(|e| e.error);
        status.try_set(Some(result));
    });
    status
}

/// Why the stored values could not be read.
fn load_failed(lang: Lang, error: String) -> impl IntoView {
    view! {
        <p class="settings__empty" role="alert">
            {format!("{} {error}", setup_text(lang, SetupText::LoadFailed))}
        </p>
    }
}

/// The home's place, edited with the wizard's form. A running hub restarts on every save, so
/// the overlay reloads this page and `on_saved` has nothing to do.
#[component]
fn LocationPage() -> impl IntoView {
    let lang = use_lang();
    let status = load_status();
    view! {
        <section class="sheet__panel">
            {move || {
                status
                    .get()
                    .map(|loaded| match loaded {
                        Ok(s) => {
                            view! {
                                <LocationForm
                                    initial=s.location
                                    applying_target="/settings/location".to_owned()
                                    on_saved=|_| {}
                                />
                            }
                                .into_any()
                        }
                        Err(e) => load_failed(lang(), e).into_any(),
                    })
            }}
        </section>
    }
}

/// The weather switch and refresh interval, edited with the wizard's form.
#[component]
fn WeatherPage() -> impl IntoView {
    view! {
        <section class="sheet__panel">
            <WeatherForm applying_target="/settings/weather".to_owned() />
        </section>
    }
}

/// The Solcast forecast, edited with the wizard's form.
#[component]
fn SolarPage() -> impl IntoView {
    view! {
        <section class="sheet__panel">
            <SolcastForm applying_target="/settings/solar-forecast".to_owned() />
        </section>
    }
}

/// The Frigate connection, edited with the wizard's form.
#[component]
fn CamerasPage() -> impl IntoView {
    view! {
        <section class="sheet__panel">
            <FrigateForm applying_target="/settings/cameras".to_owned() />
        </section>
    }
}

/// The push service-account key, edited with the wizard's form.
#[component]
fn PushPage() -> impl IntoView {
    view! {
        <section class="sheet__panel">
            <PushForm applying_target="/settings/push-notifications".to_owned() />
        </section>
    }
}

/// The database connection, edited with the wizard's form.
#[component]
fn DatabasePage() -> impl IntoView {
    let lang = use_lang();
    let status = load_status();
    view! {
        <section class="sheet__panel">
            {move || {
                status
                    .get()
                    .map(|loaded| match loaded {
                        Ok(s) => {
                            view! {
                                <DatabaseForm
                                    initial=s.database
                                    applying_target="/settings/database".to_owned()
                                    on_saved=|_| {}
                                />
                            }
                                .into_any()
                        }
                        Err(e) => load_failed(lang(), e).into_any(),
                    })
            }}
        </section>
    }
}

#[component]
fn AppearancePage() -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    view! {
        <Panel title=T::Theme>
            <ThemeSwitch />
            <p class="settings__hint">{move || settings_text(lang(), T::ThemeHint)}</p>
        </Panel>
        <Panel title=T::EnergyView>
            <EnergyViewSwitch />
            <EnergyPreview view=Signal::derive(move || energy_view(house)) />
            <p class="settings__hint">{move || settings_text(lang(), T::EnergyViewHint)}</p>
        </Panel>
    }
}

/// The Zigbee2MQTT sub-page: the bridge, the firmware updates waiting, the counts and the way
/// to the devices list.
#[component]
pub fn ZigbeePage() -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let bridge = house.bridge;
    let primary = move || {
        house
            .areas
            .with(|areas| areas.iter().map(|a| a.primary_count).sum::<usize>())
    };
    view! {
        <Panel title=T::Bridge>
            <dl class="sheet__rows">
                <Row
                    label=T::Version
                    value=move || bridge.with(|b| b.version.clone().unwrap_or_else(|| DASH.to_owned()))
                />
                <Row
                    label=T::Coordinator
                    value=move || {
                        bridge.with(|b| b.coordinator.clone().unwrap_or_else(|| DASH.to_owned()))
                    }
                />
                <Row
                    label=T::PermitJoin
                    value=move || yes_no(lang(), bridge.with(|b| b.permit_join))
                />
                <div class="sheet__row">
                    <dt>{move || settings_text(lang(), T::State)}</dt>
                    <dd>
                        <span class="settings__state">
                            <span class=move || availability_class(bridge.with(|b| b.state))></span>
                            {move || availability_text(lang(), bridge.with(|b| b.state))}
                        </span>
                    </dd>
                </div>
            </dl>
        </Panel>
        <Panel title=T::UpdatesAvailable>
            {move || {
                let pending = house.devices.with(|ds| pending_updates(ds));
                if pending.is_empty() {
                    view! {
                        <p class="settings__empty">{move || settings_text(lang(), T::AllUpToDate)}</p>
                    }
                        .into_any()
                } else {
                    view! {
                        <ul class="settings__updates">
                            {pending
                                .into_iter()
                                .map(|(d, u)| {
                                    view! {
                                        <li>
                                            <A href="/devices">{d.friendly_name.clone()}</A>
                                            <span>
                                                {format!(
                                                    "{} → {}",
                                                    u.installed.unwrap_or_default(),
                                                    u.latest.unwrap_or_default(),
                                                )}
                                            </span>
                                        </li>
                                    }
                                })
                                .collect_view()}
                        </ul>
                    }
                        .into_any()
                }
            }}
        </Panel>
        <Panel title=T::Counts>
            <dl class="sheet__rows">
                <Row label=T::Devices value=move || house.devices.with(Vec::len).to_string() />
                <Row label=T::Entities value=move || house.entities.with(|m| m.len()).to_string() />
                <Row label=T::PrimaryEntities value=move || primary().to_string() />
                <Row label=T::Areas value=move || house.areas.with(Vec::len).to_string() />
                <Row label=T::IgnoredTopics value=move || house.ignored.with(Vec::len).to_string() />
            </dl>
        </Panel>
        <Panel title=T::Devices>
            <p class="settings__hint">{move || settings_text(lang(), T::DevicesHint)}</p>
            <A href="/devices" attr:class="chip">
                {move || settings_text(lang(), T::OpenZ2mDevices)}
            </A>
        </Panel>
    }
}

#[component]
fn BrokerPage() -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let status = load_status();
    let reason = move || match house.connection.get() {
        ConnectionStatus::Offline { reason } => Some(reason),
        _ => None,
    };
    view! {
        <Panel title=T::Connection>
            <dl class="sheet__rows">
                <Row
                    label=T::Status
                    value=move || house.connection.with(|c| connection_word(lang(), c))
                />
                {move || {
                    reason()
                        .map(|r| {
                            view! { <Row label=T::Reason value=r /> }
                        })
                }}
            </dl>
        </Panel>
        <section class="sheet__panel">
            {move || {
                status
                    .get()
                    .map(|loaded| match loaded {
                        Ok(s) => {
                            view! {
                                <MqttForm
                                    initial=s.mqtt
                                    applying_target="/settings/broker".to_owned()
                                    on_saved=|_| {}
                                />
                            }
                                .into_any()
                        }
                        Err(e) => load_failed(lang(), e).into_any(),
                    })
            }}
        </section>
    }
}

#[component]
fn McpPage() -> impl IntoView {
    let lang = use_lang();
    let origin = origin();
    let endpoint = format!("{origin}/mcp");
    view! {
        <Panel title=T::Endpoint>
            <dl class="sheet__rows">
                <Row label=T::Endpoint value=endpoint />
                <Row
                    label=T::Transport
                    value=move || settings_text(lang(), T::TransportValue).to_owned()
                />
            </dl>
        </Panel>
        <Panel title=T::SetUpAssistant>
            <Snippet title=T::ClaudeCode text=claude_code_snippet(&origin) />
            <Snippet title=T::CodexCli text=codex_snippet(&origin) />
            <Snippet title=T::ClaudeDesktop text=claude_desktop_snippet(&origin) />
        </Panel>
    }
}

#[component]
fn AboutPage() -> impl IntoView {
    view! {
        <Panel title=T::Hub>
            <dl class="sheet__rows">
                <Row label=T::Product value=settings_text(Lang::En, T::ProductName) />
                <Row label=T::Version value=VERSION />
                <Row label=T::HubOrigin value=origin() />
            </dl>
        </Panel>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: &str = "http://192.168.1.10:8081";

    #[test]
    fn claude_code_snippet_has_endpoint() {
        let s = claude_code_snippet(ORIGIN);
        assert_eq!(
            s,
            "claude mcp add --transport http oikos http://192.168.1.10:8081/mcp"
        );
    }

    #[test]
    fn codex_snippet_has_endpoint() {
        let s = codex_snippet(ORIGIN);
        assert!(s.starts_with("[mcp_servers.oikos]\n"));
        assert!(s.contains("url = \"http://192.168.1.10:8081/mcp\""));
    }

    #[test]
    fn claude_desktop_snippet_is_json_with_origin() {
        let s = claude_desktop_snippet(ORIGIN);
        assert!(s.lines().count() > 1);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["mcpServers"]["oikos"]["args"][2], ORIGIN);
        assert_eq!(v["mcpServers"]["oikos"]["command"], "<path>/nidoo");
    }

    #[test]
    fn every_page_round_trips_its_slug() {
        for page in SettingsPage::ALL {
            assert_eq!(SettingsPage::from_slug(page.slug()), Some(page));
            assert_eq!(page.href(), format!("/settings/{}", page.slug()));
        }
        assert_eq!(SettingsPage::from_slug("nope"), None);
        assert_eq!(SettingsPage::Zigbee2mqtt.title(Lang::En), "Zigbee2MQTT");
        assert_eq!(SettingsPage::About.title(Lang::Fr), "À propos");
    }

    #[test]
    fn zigbee_status_prefers_updates_over_the_bridge_state() {
        let en = |n, state| zigbee_status(Lang::En, n, state);
        assert_eq!(en(0, Availability::Online), "Online");
        assert_eq!(en(0, Availability::Offline), "Offline");
        assert_eq!(en(1, Availability::Online), "1 update");
        assert_eq!(en(3, Availability::Offline), "3 updates");
        assert_eq!(zigbee_status(Lang::Fr, 0, Availability::Online), "En ligne");
        assert_eq!(
            zigbee_status(Lang::Fr, 1, Availability::Online),
            "1 mise à jour"
        );
        assert_eq!(
            zigbee_status(Lang::Fr, 3, Availability::Offline),
            "3 mises à jour"
        );
    }
}
