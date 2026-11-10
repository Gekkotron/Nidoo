//! The hub: the registry behind a lock, the event broadcast, the connection status and the
//! three writers (commands, overrides, areas). The MQTT task feeds it through [`Hub::ingest`]
//! and drains its publishes from the channel given to [`Hub::new`]; the HTTP layer reads it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{
    Arc, Mutex, MutexGuard, OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak,
};
use std::time::Duration;

use crate::alerts::{AlertStore, Applied};
use crate::ambiances::AmbianceStore;
use crate::dashboards::DashboardStore;
use crate::flows::FlowStore;
use crate::groups::{self, ApplyOutcome, GroupStore};
use crate::history::History;
use crate::media::MediaProxy;
use crate::network::NetworkStore;
use crate::pictures::{MAX_PICTURE_BYTES, Picture, Pictures};
use crate::virtual_entities as virtuals;
use nidoo_api::{
    AlertView, AreaView, CommandResponse, ConnectionStatus, EntityView, GroupMemberView, GroupSpec,
    GroupView, IgnoredView, OverridePatch, Snapshot, VirtualKind, VirtualSpec, WatchStatus,
    WsEvent, area_views, device_views, entity_views, mask_secret, snapshot,
};
use nidoo_core::command::{CommandError, Publish, echo_confirms, map_command};
use nidoo_core::model::{
    AreasDocument, Availability, Command, Entity, Kind, PatchError, StateValue, UNASSIGNED_AREA,
    UiOverride,
};
use nidoo_core::registry::{Change, Ignored, Inbound, Registry, RegistryConfig};
use nidoo_core::types::command::expects_echo;
use nidoo_push::fcm::{FcmError, FcmTransport, HttpFcm, ServiceAccount};
use nidoo_push::{PushService, Rejected};
use tokio::sync::{broadcast, mpsc, watch};

use crate::alarm::{ALARM_ENTITY_ID, AlarmHub};
use crate::climate::ClimateHub;
use crate::clock;
use crate::config::{AclSource, Config};
pub use crate::flows::EngineRequest;

/// Events kept for a slow WebSocket client before it starts missing some.
const EVENT_CAPACITY: usize = 1024;

/// Why a request to the hub failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HubError {
    #[error("unknown entity `{0}`")]
    UnknownEntity(String),
    /// The entity cannot take the request right now; nothing was published.
    #[error("rejected: {0}")]
    Rejected(String),
    #[error(transparent)]
    Patch(#[from] PatchError),
    #[error(transparent)]
    Command(#[from] CommandError),
    /// The publish was refused (the broker link is down) or could not be handed to the MQTT
    /// task; nothing was applied locally.
    #[error("publish: {0}")]
    Publish(String),
    #[error("unknown alert `{0}`")]
    UnknownAlert(String),
    #[error("unknown action `{0}`")]
    UnknownAction(String),
    #[error("unknown flow `{0}`")]
    UnknownFlow(String),
    #[error("unknown area `{0}`")]
    UnknownArea(String),
    #[error("unknown dashboard `{0}`")]
    UnknownDashboard(String),
    #[error("unknown ambiance `{0}`")]
    UnknownAmbiance(String),
    #[error("unknown assistant turn `{0}`")]
    UnknownTurn(String),
    /// The flow cannot be saved or run: it does not validate, or names something the house
    /// does not have.
    #[error("invalid flow: {0}")]
    InvalidFlow(String),
}

/// A message on a group's command topic that is a command: a JSON object with a `type`.
fn parse_group_command(payload: &[u8]) -> Option<Command> {
    let value: serde_json::Value = serde_json::from_slice(payload).ok()?;
    value.get("type")?;
    serde_json::from_value(value).ok()
}

/// The FCM HTTP v1 endpoint.
const FCM_BASE: &str = "https://fcm.googleapis.com/";

/// The FCM sender for `[push] credentials`, or `None` (history only) with one warning: no
/// credentials configured, or a key file that cannot be read. Only the path is ever logged.
fn push_transport(cfg: &crate::config::PushConfig) -> Option<Arc<dyn FcmTransport>> {
    if let Some(raw) = cfg.service_account.as_deref() {
        return match ServiceAccount::from_json(raw) {
            Ok(account) => {
                let base = url::Url::parse(FCM_BASE).expect("FCM base URL parses");
                Some(Arc::new(HttpFcm::new(account, base)))
            }
            Err(source) => {
                // Only the position: a serde message could quote part of the key.
                tracing::warn!(
                    line = source.line(),
                    column = source.column(),
                    "push sending is off: the stored service account is not a service-account key; alerts are kept in the history only"
                );
                None
            }
        };
    }
    let Some(path) = cfg.credentials.as_deref() else {
        tracing::warn!(
            "push sending is off: no [push] credentials and no secrets/firebase.json next to the config; alerts are kept in the history only"
        );
        return None;
    };
    let account = match ServiceAccount::load(path) {
        Ok(account) => account,
        Err(FcmError::Read { source, .. }) => {
            tracing::warn!(
                path = %path.display(),
                error = %source,
                "push sending is off: cannot read the [push] credentials file; alerts are kept in the history only"
            );
            return None;
        }
        Err(FcmError::Parse { source, .. }) => {
            // Only the position: a serde message could quote part of the key file.
            tracing::warn!(
                path = %path.display(),
                line = source.line(),
                column = source.column(),
                "push sending is off: the [push] credentials file is not a service-account key; alerts are kept in the history only"
            );
            return None;
        }
    };
    let base = url::Url::parse(FCM_BASE).expect("FCM base URL parses");
    Some(Arc::new(HttpFcm::new(account, base)))
}

/// The `install_id` the web sends in a reply; the app sends its own install id.
const WEB_INSTALL_ID: &str = "nidoo-web";

/// The reply the app's `ReplyWorker` publishes, field order included.
#[derive(serde::Serialize)]
struct Reply<'a> {
    id: &'a str,
    action: &'a str,
    install_id: &'a str,
    ts: u64,
}

/// What [`Hub::entity_detail`] returns: the view plus what the view leaves out.
#[derive(Clone, Debug, PartialEq)]
pub struct EntityDetail {
    pub view: EntityView,
    /// The expanded discovery config the entity was built from.
    pub bindings: serde_json::Value,
    /// The override retained on `<ui_prefix>/<id>`, verbatim.
    pub raw_override: Option<UiOverride>,
}

/// A report on a state topic of `id`, with the link epoch it was ingested in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateSeen {
    pub id: String,
    /// [`Hub::link_epoch`] when the report arrived; a report from before the latest link drop
    /// is stale and must not seed anything.
    pub epoch: u64,
}

/// A message on a state topic of `id`, whatever the entity read from it. `read` is true when
/// the entity read something (a [`StateSeen`] went out too) and false when the value stayed
/// unknown (a `null` field, a missing key).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateTouched {
    pub id: String,
    /// [`Hub::link_epoch`] when the message arrived.
    pub epoch: u64,
    pub read: bool,
}

/// What [`Hub::run_group_migration_once`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationOutcome {
    /// The ids of the groups created.
    pub created: Vec<String>,
    /// Why nothing was attempted; `None` when the migration ran.
    pub skipped_reason: Option<&'static str>,
}

/// The broker ACL watchdog and, for the topic source, the log filter it listens on.
struct AclWatchdog {
    topic: Option<String>,
    dog: Mutex<nidoo_watchdog::AclWatchdog>,
}

pub struct Hub {
    /// The hub's own `Arc`, so a message handler can spawn work that outlives its `&self`.
    pub(crate) me: Weak<Hub>,
    registry: RwLock<Registry>,
    events: broadcast::Sender<WsEvent>,
    /// The ids whose state was reported, changed or not: Zigbee2MQTT echoes the whole device
    /// state on every `/set`, so a no-op command is confirmed by its echo.
    state_seen: broadcast::Sender<StateSeen>,
    /// The ids a message arrived for on a state topic, including those whose value stayed
    /// unknown. Only the flow engine's liveness checks read it; command confirmation and the
    /// seeding consumers stay on `state_seen`.
    state_touched: broadcast::Sender<StateTouched>,
    /// Woken when the set of topics to subscribe to may have changed outside a broker message
    /// (a local flow write); the MQTT task diffs its subscriptions on it.
    pub(crate) subscriptions_changed: Arc<tokio::sync::Notify>,
    /// Counts the times the broker link left `Online`.
    link_epoch: AtomicU64,
    connection: watch::Sender<ConnectionStatus>,
    publisher: mpsc::Sender<Publish>,
    pub(crate) ui_prefix: String,
    /// The first discovery prefix: virtual entities are published under it.
    pub(crate) discovery_prefix: String,
    /// Push notifications: the service owns `<prefix>/history`; the alert store decodes the
    /// broker's echo of it for the web and the API.
    push: Arc<PushService>,
    alerts: Mutex<AlertStore>,
    /// The Frigate event bridge and the topic it reads, when `[frigate] bridge` is on.
    frigate: Option<(String, Mutex<nidoo_frigate::FrigateBridge>)>,
    /// The gateway health watchdog, when `[watchdog.gateway]` is on.
    gateway_watchdog: Option<Mutex<nidoo_watchdog::GatewayWatchdog>>,
    /// The broker ACL watchdog, when `[watchdog.acl]` is on.
    acl_watchdog: Option<AclWatchdog>,
    /// The retained flow definitions and their runs.
    flow_store: Mutex<FlowStore>,
    /// The flow label catalog, retained on `flow_labels_topic`.
    flow_labels: Mutex<nidoo_core::flow::FlowLabelCatalog>,
    /// `[topics] flow_labels`.
    pub(crate) flow_labels_topic: String,
    /// Held by every write that touches labels across flows (catalog edits, label switches,
    /// flow saves), so each one reads the flows and the catalog as the last one left them.
    pub(crate) labels_pass: tokio::sync::Mutex<()>,
    /// Every reading of the entities an `if` averages; the engine sets which on reload.
    recent_readings: Mutex<crate::flows::readings::RecentReadings>,
    /// The passes of every `limit` node still inside its window, by (flow, node). Runs execute
    /// in tasks that only hold the hub, so the counts live here, not in the engine.
    limits: Mutex<crate::flows::LimitCounts>,
    group_store: Mutex<GroupStore>,
    /// `[groups] enabled`: group writes and listing are refused while it is false.
    groups_enabled: bool,
    /// The retained network watch definitions.
    network_store: Mutex<NetworkStore>,
    /// `[network] enabled`: watch writes and listing are refused while it is false.
    pub(crate) network_enabled: bool,
    /// The last verdict of each watch, by id.
    pub(crate) network_status: Mutex<BTreeMap<String, WatchStatus>>,
    /// Bumped on every change of the network store.
    network_rev: watch::Sender<u64>,
    /// The climate engine's documents, present only while `[climate] enabled`.
    pub(crate) climate: Option<ClimateHub>,
    /// The alarm's documents; the engine always listens and idles while the alarm is off.
    pub(crate) alarm: AlarmHub,
    /// `[weather] enabled` and `refresh_minutes`.
    pub(crate) weather_enabled: bool,
    pub(crate) weather_notify_unreachable: bool,
    pub(crate) solar_notify_unreachable: bool,
    pub(crate) outages: Mutex<crate::outage::Outages>,
    /// `[system] notify`: push when the hub itself, or a service it depends on, goes down.
    pub(crate) system_notify: std::sync::atomic::AtomicBool,
    /// Device commands published since boot (one per publish, group members each count).
    pub(crate) commands_sent: AtomicU64,
    /// The broker's last retained-message report; `None` until the first one.
    pub(crate) system_retained: Mutex<Option<crate::system::RetainedReport>>,
    /// The system task's figures, service states and published sensors.
    pub(crate) system: Mutex<crate::system::task::SystemState>,
    pub(crate) weather_refresh_minutes: u64,
    pub(crate) weather_city: String,
    pub(crate) weather_country: Option<String>,
    pub(crate) weather_coordinates: Option<(f64, f64)>,
    pub(crate) weather: Mutex<crate::weather::hub::WeatherSlot>,
    /// `[solcast]`: `enabled`, `sites`, `daily_limit` and `api_key_file`.
    pub(crate) solar_enabled: bool,
    pub(crate) solar_sites: Vec<String>,
    pub(crate) solar_daily_limit: u32,
    pub(crate) solar_key_file: Option<std::path::PathBuf>,
    /// The API key itself, when set from the browser; never logged.
    pub(crate) solar_api_key: Option<String>,
    pub(crate) solar: Mutex<crate::solar::hub::SolarSlot>,
    /// Set once the broker acknowledged the on-connect subscription: the retained solar cache
    /// has been requested, and the poller may read the ledger.
    pub(crate) solar_subscribed: tokio::sync::watch::Sender<bool>,
    /// The retained dashboard definitions, under `<ui_prefix>/dashboards/<id>`.
    dashboard_store: Mutex<DashboardStore>,
    /// Held while a dashboard write picks orders from the store and publishes, so two writers
    /// never take the same `order`. Never held across an `await`.
    pub(crate) dashboards_write: Mutex<()>,
    /// The retained ambiance definitions, under `<ui_prefix>/ambiances/<id>`.
    ambiance_store: Mutex<AmbianceStore>,
    /// Ids of the ambiances being applied right now.
    pub(crate) ambiances_running: Mutex<BTreeSet<String>>,
    /// The home settings document, retained on `<ui_prefix>/_home`.
    pub(crate) home_settings: Mutex<nidoo_api::HomeSettings>,
    /// The assistant's pending questions and turn log.
    pub(crate) assist: crate::assist::AssistState,
    /// The topic prefix flows are retained under, one topic per flow id.
    automation_prefix: String,
    /// The time zone schedules and their preview run in.
    zone: chrono_tz::Tz,
    /// The house (latitude, longitude), Paris when `[flows]` sets none.
    house_point: (f64, f64),
    /// Whether `house_point` is the Paris fallback, latched at startup.
    fallback_active: bool,
    /// The `[flows]` section as configured.
    flows_config: crate::config::FlowsConfig,
    /// Requests for the flow engine; the engine takes the receiving end once.
    engine_tx: mpsc::UnboundedSender<EngineRequest>,
    engine_rx: Mutex<Option<mpsc::UnboundedReceiver<EngineRequest>>>,
    /// Frigate media served through the hub; `None` without a `[frigate] url`.
    media: Option<Arc<MediaProxy>>,
    /// The latest picture of each camera or image entity.
    pictures: Arc<Pictures>,
    confirm_timeout: Duration,
    subscription_debounce: Duration,
    /// Entities whose template failure was already logged, so each failure is logged once.
    logged_failures: Mutex<BTreeSet<String>>,
    /// Ignored discovery topics (with their reason) already logged, so each is logged once.
    logged_ignored: Mutex<BTreeSet<(String, String)>>,
    /// Set once at startup when `[history] url` is configured; receives one [`Incoming`]
    /// per `Change::StateChanged` emitted by [`Self::ingest`].
    history: OnceLock<Arc<History>>,
    tts: OnceLock<Arc<crate::assist::tts::Tts>>,
    stt: OnceLock<Arc<crate::assist::stt::Stt>>,
    /// The voice service's way in for questions the hub asks out loud.
    pub(crate) voice_asks: OnceLock<tokio::sync::mpsc::Sender<crate::assist::stt::ask::AskRequest>>,
    /// `nidoo-sound`'s "sentence played" reports, for the voice service.
    spoken: broadcast::Sender<crate::sound::Spoken>,
    /// The setup context, set once at startup in Run mode; the settings MCP tools use it.
    pub(crate) setup: OnceLock<Arc<crate::setup::SetupCtx>>,
}

impl Hub {
    /// The house time zone, `[flows] timezone`.
    pub fn zone(&self) -> chrono_tz::Tz {
        self.zone
    }

    /// The house (latitude, longitude), `[flows] latitude`/`longitude` or Paris.
    pub fn house_point(&self) -> (f64, f64) {
        self.house_point
    }

    /// The `[flows]` settings as the UI reads them.
    pub fn flows_config_view(&self) -> nidoo_api::FlowsConfigView {
        nidoo_api::FlowsConfigView {
            latitude: self.flows_config.latitude,
            longitude: self.flows_config.longitude,
            timezone: self.zone.name().to_owned(),
            fallback_active: self.fallback_active,
        }
    }

    /// True when `house_point` is the Paris fallback.
    pub fn fallback_active(&self) -> bool {
        self.fallback_active
    }

    pub fn new(cfg: &Config, publisher: mpsc::Sender<Publish>) -> Arc<Hub> {
        let registry = Registry::new(RegistryConfig {
            discovery_prefixes: cfg.topics.discovery_prefixes.clone(),
            ui_prefix: cfg.topics.ui_prefix.clone(),
            z2m_base: cfg.topics.z2m_base.clone(),
            reserved: RegistryConfig::default_reserved(),
        });
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let (state_seen, _) = broadcast::channel(EVENT_CAPACITY);
        let (state_touched, _) = broadcast::channel(EVENT_CAPACITY);
        let (connection, _) = watch::channel(ConnectionStatus::Connecting);
        let media = MediaProxy::new(cfg.frigate.clone());
        let alerts = AlertStore::with_media_origin(media.as_ref().map(|m| m.origin().to_owned()));
        let (engine_tx, engine_rx) = mpsc::unbounded_channel();
        let push = PushService::new(
            cfg.push.settings(),
            publisher.clone(),
            push_transport(&cfg.push),
            clock::now,
        );
        Arc::new_cyclic(|me| Hub {
            me: me.clone(),
            registry: RwLock::new(registry),
            events,
            state_seen,
            state_touched,
            subscriptions_changed: Arc::new(tokio::sync::Notify::new()),
            link_epoch: AtomicU64::new(0),
            connection,
            publisher,
            ui_prefix: cfg.topics.ui_prefix.clone(),
            discovery_prefix: cfg
                .topics
                .discovery_prefixes
                .first()
                .cloned()
                .unwrap_or_else(|| "nidoo".to_owned()),
            push,
            alerts: Mutex::new(alerts),
            frigate: cfg.frigate.bridge_settings().map(|settings| {
                (
                    cfg.frigate.events_topic(),
                    Mutex::new(nidoo_frigate::FrigateBridge::new(settings)),
                )
            }),
            gateway_watchdog: cfg.watchdog.gateway.enabled.then(|| {
                Mutex::new(nidoo_watchdog::GatewayWatchdog::new(
                    cfg.watchdog.gateway.prefix.clone(),
                ))
            }),
            acl_watchdog: cfg.watchdog.acl.enabled.then(|| AclWatchdog {
                // The file source is fed by `on_acl_line`, through the tail task in main.
                topic: (cfg.watchdog.acl.source == AclSource::Topic)
                    .then(|| cfg.watchdog.acl.log_topic.clone()),
                dog: Mutex::new(nidoo_watchdog::AclWatchdog::new(cfg.watchdog.acl.cooldown)),
            }),
            flow_store: Mutex::new(FlowStore::new(cfg.automation.runs_kept)),
            recent_readings: Mutex::new(Default::default()),
            limits: Mutex::new(Default::default()),
            group_store: Mutex::new(GroupStore::default()),
            groups_enabled: cfg.groups.enabled,
            network_store: Mutex::new(NetworkStore::default()),
            network_enabled: cfg.network.enabled,
            network_status: Mutex::new(BTreeMap::new()),
            network_rev: watch::channel(0).0,
            climate: cfg
                .climate
                .enabled
                .then(|| ClimateHub::new(&cfg.climate.prefix)),
            alarm: AlarmHub::new(&cfg.alarm.prefix),
            weather_enabled: cfg.weather.enabled,
            weather_notify_unreachable: cfg.weather.notify_unreachable,
            solar_notify_unreachable: cfg.solcast.notify_unreachable,
            system_notify: std::sync::atomic::AtomicBool::new(cfg.system.notify),
            commands_sent: AtomicU64::new(0),
            system_retained: Mutex::new(None),
            system: Mutex::new(crate::system::task::SystemState::new(
                cfg.system.notify,
                clock::now(),
            )),
            outages: Mutex::new(crate::outage::Outages::new(
                cfg.weather.notify_unreachable,
                cfg.solcast.notify_unreachable,
            )),
            weather_refresh_minutes: cfg.weather.refresh_minutes,
            weather_city: cfg.weather.city.trim().to_owned(),
            weather_country: cfg.weather.country.clone(),
            weather_coordinates: cfg.weather.latitude.zip(cfg.weather.longitude),
            weather: Mutex::default(),
            solar_enabled: cfg.solcast.enabled,
            solar_sites: cfg.solcast.sites.clone(),
            solar_daily_limit: cfg.solcast.daily_limit,
            solar_key_file: cfg.solcast.api_key_file.clone(),
            solar_api_key: cfg.solcast.api_key.clone(),
            solar: Mutex::default(),
            solar_subscribed: tokio::sync::watch::channel(false).0,
            dashboard_store: Mutex::new(DashboardStore::new()),
            dashboards_write: Mutex::new(()),
            ambiance_store: Mutex::new(AmbianceStore::new()),
            ambiances_running: Mutex::new(BTreeSet::new()),
            home_settings: Mutex::new(nidoo_api::HomeSettings::default()),
            assist: crate::assist::AssistState::default(),
            automation_prefix: cfg.topics.automation_prefix.clone(),
            flow_labels: Mutex::default(),
            flow_labels_topic: cfg.topics.flow_labels.trim().to_owned(),
            labels_pass: tokio::sync::Mutex::new(()),
            zone: crate::flows::house_zone(&cfg.flows),
            house_point: cfg.flows.house_point(),
            fallback_active: cfg.flows.fallback_active(),
            flows_config: cfg.flows.clone(),
            engine_tx,
            engine_rx: Mutex::new(Some(engine_rx)),
            media,
            pictures: Arc::new(Pictures::new()),
            confirm_timeout: Duration::from_millis(cfg.commands.confirm_timeout_ms),
            subscription_debounce: Duration::from_millis(cfg.commands.subscription_debounce_ms),
            logged_failures: Mutex::new(BTreeSet::new()),
            logged_ignored: Mutex::new(BTreeSet::new()),
            history: OnceLock::new(),
            tts: OnceLock::new(),
            stt: OnceLock::new(),
            voice_asks: OnceLock::new(),
            spoken: broadcast::channel(64).0,
            setup: OnceLock::new(),
        })
    }

    /// Hands the recorder to the hub; the second call is a no-op.
    pub fn set_history(&self, history: Arc<History>) {
        let _ = self.history.set(history);
    }

    /// Hands the setup context to the hub; the second call is a no-op.
    pub fn set_setup(&self, ctx: Arc<crate::setup::SetupCtx>) {
        let _ = self.setup.set(ctx);
    }

    /// Hands the speech facade to the hub; the second call is a no-op.
    pub fn set_tts(&self, tts: Arc<crate::assist::tts::Tts>) {
        let _ = self.tts.set(tts);
    }

    /// The speech facade, when one is configured.
    pub fn tts(&self) -> Option<&Arc<crate::assist::tts::Tts>> {
        self.tts.get()
    }

    /// Hands the speech input facade to the hub; the second call is a no-op.
    pub fn set_stt(&self, stt: Arc<crate::assist::stt::Stt>) {
        let _ = self.stt.set(stt);
    }

    /// The speech input facade, when one is configured.
    pub fn stt(&self) -> Option<&Arc<crate::assist::stt::Stt>> {
        self.stt.get()
    }

    /// `nidoo-sound`'s reports that a sentence finished playing.
    pub fn subscribe_spoken(&self) -> broadcast::Receiver<crate::sound::Spoken> {
        self.spoken.subscribe()
    }

    /// The recorder, when one is configured.
    pub fn history(&self) -> Option<&Arc<History>> {
        self.history.get()
    }

    /// The `limit` counts, taken as they are when a thread panicked while holding them.
    pub(crate) fn limits(&self) -> MutexGuard<'_, crate::flows::LimitCounts> {
        self.limits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn recent_readings(&self) -> MutexGuard<'_, crate::flows::readings::RecentReadings> {
        self.recent_readings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Keeps every reading of the entities in `windows`, each for its window in microseconds.
    pub fn set_averaged(&self, windows: std::collections::BTreeMap<String, i64>) {
        self.recent_readings().set_windows(windows);
    }

    /// The readings of `id` received at or after `from` while it was averaged, oldest first.
    pub fn readings_since(&self, id: &str, from: i64) -> Vec<nidoo_core::history::buckets::Sample> {
        self.recent_readings().since(id, from)
    }

    /// Records the numeric reading of each averaged entity in `seen`, repeats included.
    fn record_readings(&self, seen: &[String], at: i64) {
        let wanted: Vec<&String> = {
            let recent = self.recent_readings();
            seen.iter().filter(|id| recent.wants(id)).collect()
        };
        if wanted.is_empty() {
            return;
        }
        let nums: Vec<(&String, f64)> = {
            let reg = self.read();
            wanted
                .into_iter()
                .filter_map(|id| {
                    let num = history_num_of(reg.entity(id)?, &reg.state(id)?.value)?;
                    Some((id, num))
                })
                .collect()
        };
        let mut recent = self.recent_readings();
        for (id, num) in nums {
            recent.push(id, at, num);
        }
    }

    /// Routes and applies one broker message, broadcasts the resulting events and reports the
    /// changes (the MQTT task watches for [`Change::SubscriptionsChanged`]).
    pub fn ingest(&self, topic: &str, payload: &[u8]) -> Vec<Change> {
        if topic == self.push.history_topic() {
            self.ingest_history(topic, payload);
            return Vec::new();
        }
        if topic == crate::home::topic(&self.ui_prefix) {
            self.ingest_home(payload);
            return Vec::new();
        }
        if topic == self.flow_labels_topic {
            self.ingest_flow_labels(payload);
            return Vec::new();
        }
        if let Some(id) = self.dashboard_id_of(topic) {
            // Dashboard structure is not pushed over the WebSocket, so there is nothing to
            // broadcast; the store is read on demand.
            self.ingest_dashboard(id, payload);
            return Vec::new();
        }
        if let Some(id) = self.ambiance_id_of(topic) {
            self.ingest_ambiance(id, payload);
            return Vec::new();
        }
        if let Some(id) = self.flow_id_of(topic) {
            // A flow may start or stop watching a button, so its command topic may change.
            return if self.ingest_flow(id, payload) {
                vec![Change::SubscriptionsChanged]
            } else {
                Vec::new()
            };
        }
        if self.ingest_group(topic, payload) {
            return Vec::new();
        }
        if self.ingest_network(topic, payload) {
            return Vec::new();
        }
        let at = clock::now();
        let at_micros = clock::now_micros();
        let history_feed_enabled = self.history.get().is_some();
        let (changes, events, failing, seen, touched_unread, ignored, history_feed) = {
            let mut reg = self.write();
            let inbound = reg.route(topic, payload.to_vec(), at);
            let is_state = matches!(inbound, Inbound::State { .. });
            let is_override = matches!(inbound, Inbound::Override { .. });
            let changes = reg.apply(inbound);
            let events = events_for(&reg, &changes);
            let ignored = changes
                .contains(&Change::IgnoredChanged)
                .then(|| reg.ignored().to_vec());
            // The registry drops an override it cannot decode without a trace; say so here.
            if is_override && !payload.is_empty() && UiOverride::decode(payload).is_none() {
                tracing::warn!(%topic, "invalid override payload ignored");
            }
            // A message on a topic that only carries an entity's availability says nothing
            // about its state, so it does not count as the entity speaking.
            let spoke = |id: &&String| {
                reg.entity(id)
                    .is_some_and(|entity| !entity.bindings.is_availability_only(topic))
            };
            let seen: Vec<String> = if is_state {
                changes
                    .iter()
                    .filter_map(|change| match change {
                        Change::StateChanged(id) | Change::StateReported(id) => Some(id),
                        _ => None,
                    })
                    .filter(spoke)
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            };
            // The entities the message reached but read nothing from (`StateTouched`).
            let touched_unread: Vec<String> = if is_state {
                changes
                    .iter()
                    .filter_map(|change| match change {
                        Change::StateTouched(id) => Some(id),
                        _ => None,
                    })
                    .filter(spoke)
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            };
            let history_feed: Vec<(String, nidoo_core::model::EntityState, Option<f64>)> =
                if history_feed_enabled {
                    changes
                        .iter()
                        .filter_map(|change| match change {
                            Change::StateChanged(id) => {
                                let entity = reg.entity(id)?;
                                // A secret is never offered to history.
                                if entity
                                    .capabilities
                                    .generic
                                    .as_ref()
                                    .is_some_and(|g| g.secret)
                                {
                                    return None;
                                }
                                let state = reg.state(id)?;
                                let num = history_num_of(entity, &state.value);
                                Some((id.clone(), state.clone(), num))
                            }
                            _ => None,
                        })
                        .collect()
                } else {
                    Vec::new()
                };
            (
                changes,
                events,
                reg.template_failures().clone(),
                seen,
                touched_unread,
                ignored,
                history_feed,
            )
        };
        if let Some(history) = self.history.get() {
            for (id, state, num) in history_feed {
                history.offer(nidoo_core::history::Incoming {
                    at: at_micros,
                    id,
                    state,
                    num,
                });
            }
        }
        self.log_new_template_failures(&failing);
        if let Some(ignored) = ignored {
            self.log_new_ignored(&ignored);
        }
        for event in events {
            self.broadcast(event);
        }
        // A member whose value or availability changed, or that reported, moves its groups.
        let mut touched: BTreeSet<String> = changes
            .iter()
            .filter_map(|change| match change {
                Change::StateChanged(id) => Some(id.clone()),
                _ => None,
            })
            .collect();
        self.record_readings(&seen, at_micros);
        for id in seen {
            touched.insert(id.clone());
            // An error only means no command is waiting right now.
            let epoch = self.link_epoch();
            let _ = self.state_touched.send(StateTouched {
                id: id.clone(),
                epoch,
                read: true,
            });
            let _ = self.state_seen.send(StateSeen { id, epoch });
        }
        for id in touched_unread {
            let epoch = self.link_epoch();
            let _ = self.state_touched.send(StateTouched {
                id,
                epoch,
                read: false,
            });
        }
        for id in &touched {
            self.on_member_state_change(id);
        }
        self.with_button_subscriptions(changes)
    }

    /// Puts recorded values back on the entities that have not reported since boot (see
    /// [`nidoo_core::registry::Registry::restore_state`]) and broadcasts them like an ingest.
    /// A restore is not a report: nothing goes to history (the row is already its latest), to
    /// the readings or to `state_seen`, and a secret entity is skipped. `values` carry the
    /// recorded time in microseconds. Returns how many entities were restored.
    pub fn restore_recorded_states(&self, values: Vec<(String, i64, StateValue)>) -> usize {
        let (changes, events) = {
            let mut reg = self.write();
            let mut changes = Vec::new();
            for (id, at_micros, value) in values {
                if reg
                    .entity(&id)
                    .and_then(|entity| entity.capabilities.generic.as_ref())
                    .is_some_and(|g| g.secret)
                {
                    continue;
                }
                let at_secs = u64::try_from(at_micros / 1_000_000).unwrap_or(0);
                changes.extend(reg.restore_state(&id, value, at_secs));
            }
            let events = events_for(&reg, &changes);
            (changes, events)
        };
        for event in events {
            self.broadcast(event);
        }
        for change in &changes {
            if let Change::StateChanged(id) = change {
                self.on_member_state_change(id);
            }
        }
        changes.len()
    }

    /// [`Self::ingest`] for a message that carries the broker's retain flag. A press is an
    /// event: a non-retained message on the command topic of a button some flow watches
    /// broadcasts [`WsEvent::ButtonPressed`]. A retained one is a replay, never a press.
    pub fn ingest_message(&self, topic: &str, payload: &[u8], retain: bool) -> Vec<Change> {
        // A climate hold command from an app, never a device.
        if self.ingest_climate_command(topic, payload, retain) {
            return Vec::new();
        }
        // A report for the voice service, never a device.
        if topic == crate::sound::SPOKEN_TOPIC {
            if let Some(spoken) = crate::sound::parse_spoken(payload) {
                let _ = self.spoken.send(spoken);
            }
            return Vec::new();
        }
        // The broker's own figure, not a device.
        if self.ingest_system_retained(topic, payload) {
            return Vec::new();
        }
        // A weather device left on the broker by an earlier run is cleared, not applied, and
        // the retained cache is the poller's data.
        if self.clear_disabled_weather(topic, payload, retain)
            || self.ingest_weather_cache(topic, payload)
        {
            return Vec::new();
        }
        // The same for a solar forecast device, and the retained cache is the poller's data.
        if self.clear_disabled_solar(topic, payload, retain)
            || self.ingest_solar_cache(topic, payload)
        {
            return Vec::new();
        }
        // Climate documents are the engine's own data, never devices.
        if self.ingest_climate(topic, payload) {
            return Vec::new();
        }
        // The same for the alarm's documents.
        if self.ingest_alarm(topic, payload, retain) {
            return Vec::new();
        }
        if !retain && topic == self.push.responses_topic() {
            // Replies to the alarm's alerts go to the alarm, the rest to the climate engine.
            let alarm_reply = serde_json::from_slice::<serde_json::Value>(payload)
                .ok()
                .and_then(|doc| doc.get("id").and_then(|id| id.as_str().map(str::to_owned)))
                .is_some_and(|id| id.starts_with(crate::alarm::effects::ALERT_PREFIX));
            if alarm_reply || self.climate.is_some() {
                if let Some(hub) = self.me.upgrade() {
                    if alarm_reply {
                        hub.on_alarm_reply(payload);
                    } else {
                        hub.on_climate_reply(payload);
                    }
                }
                return Vec::new();
            }
        }
        if self.push.wants(topic) {
            // Then on: the history echo still feeds the alert store below.
            self.push.on_message(topic, payload, retain);
        }
        if let Some((events, _)) = &self.frigate
            && topic == events
            && !retain
        {
            self.on_frigate_event(payload);
        }
        // Health states are retained: a reconnect replays them, and an unchanged one is silent.
        if let Some(dog) = &self.gateway_watchdog {
            let alert = {
                let mut dog = dog.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                if dog.wants(topic) {
                    dog.on_message(topic, payload, clock::now())
                } else {
                    None
                }
            };
            self.raise_system(alert);
        }
        if !retain
            && let Some(acl) = &self.acl_watchdog
            && acl
                .topic
                .as_deref()
                .is_some_and(|filter| nidoo_watchdog::topic_matches(filter, topic))
        {
            self.on_acl_line(&String::from_utf8_lossy(payload));
        }
        self.capture_pictures(topic, payload);
        if !retain {
            let pressed = self.watched_button_topics().remove(topic);
            if let Some(ids) = pressed {
                for id in ids {
                    self.broadcast(WsEvent::ButtonPressed { id });
                }
                return Vec::new();
            }
            for (id, signal) in self.signals_of(topic, payload) {
                self.record_signal(&id, &signal);
                self.broadcast(WsEvent::SignalReceived { id, signal });
            }
            if let Some(group_id) = self.group_command_id(topic)
                && let Some(cmd) = parse_group_command(payload)
                && let Some(hub) = self.me.upgrade()
                && tokio::runtime::Handle::try_current().is_ok()
            {
                tokio::spawn(async move {
                    let _ = hub.on_group_command(&group_id, cmd).await;
                });
                return Vec::new();
            }
        }
        self.ingest(topic, payload)
    }

    /// The in-memory pictures the route serves.
    pub fn pictures(&self) -> &Arc<Pictures> {
        &self.pictures
    }

    /// Keeps what a message says about the picture of the camera and image entities reading
    /// `topic`: the bytes (decoded from base64 when `image_encoding` says so) or a URL.
    fn capture_pictures(&self, topic: &str, payload: &[u8]) {
        use base64::Engine as _;
        use nidoo_core::types::{Shape, type_def};
        let mut found = Vec::new();
        {
            let reg = self.read();
            for id in reg.entities_on_topic(topic) {
                let Some(entity) = reg.entity(id) else {
                    continue;
                };
                let Some(field) = type_def(&entity.component)
                    .filter(|def| def.primary.as_deref() == Some("picture"))
                    .and_then(|def| def.fields.get("picture"))
                    .filter(|field| field.shape == Shape::Picture)
                else {
                    continue;
                };
                let cfg = &entity.bindings.config;
                let is = |key: &str| cfg.get(key).and_then(|v| v.as_str()) == Some(topic);
                if is(&field.topic) {
                    let body = if cfg.get("image_encoding").and_then(|v| v.as_str()) == Some("b64")
                    {
                        let text = String::from_utf8_lossy(payload);
                        match base64::engine::general_purpose::STANDARD.decode(text.trim()) {
                            Ok(body) => body,
                            Err(err) => {
                                tracing::warn!(%id, %err, "picture is not valid base64");
                                continue;
                            }
                        }
                    } else {
                        payload.to_vec()
                    };
                    if body.len() > MAX_PICTURE_BYTES {
                        tracing::warn!(%id, size = body.len(), "picture over 5 MiB dropped");
                        continue;
                    }
                    let content_type = cfg
                        .get("content_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("image/jpeg")
                        .to_owned();
                    found.push((
                        id.clone(),
                        Picture::Bytes {
                            content_type,
                            body: body.into(),
                        },
                    ));
                } else if is("url_topic") {
                    let text = String::from_utf8_lossy(payload);
                    let url = match cfg.get("url_template").and_then(|v| v.as_str()) {
                        Some(template) => match reg.templates().render(template, &text) {
                            Ok(url) => url,
                            Err(err) => {
                                tracing::warn!(%id, %err, "picture url template failed");
                                continue;
                            }
                        },
                        None => text.into_owned(),
                    };
                    let url = url.trim().to_owned();
                    if !url.is_empty() {
                        found.push((id.clone(), Picture::Url(url)));
                    }
                }
            }
        }
        for (id, picture) in found {
            self.pictures.put(&id, picture);
        }
    }

    /// Stores the signal as the entity's `last` and `at` (ms) and announces the state change,
    /// so the sheet shows it and a repeated signal still counts.
    fn record_signal(&self, id: &str, signal: &str) {
        let at_ms = clock::now_micros().max(0) as u64 / 1000;
        let events = {
            let mut reg = self.write();
            let changes = reg.record_signal(id, signal, at_ms);
            events_for(&reg, &changes)
        };
        for event in events {
            self.broadcast(event);
        }
    }

    /// The signals a message carries: one per signal-placed entity (remote, event, tag) it
    /// belongs to.
    fn signals_of(&self, topic: &str, payload: &[u8]) -> Vec<(String, String)> {
        use nidoo_core::types::{Placement, signals::signal_of, type_def};
        let reg = self.read();
        reg.entities_on_topic(topic)
            .iter()
            .filter_map(|id| reg.entity(id))
            .filter_map(|entity| {
                let def =
                    type_def(&entity.component).filter(|d| d.placement == Placement::Signal)?;
                let signal = signal_of(entity, def, topic, payload, reg.templates())?;
                Some((entity.unique_id.clone(), signal))
            })
            .collect()
    }

    /// The id of the group whose command topic `topic` is, when the definition is known.
    fn group_command_id(&self, topic: &str) -> Option<String> {
        let id = topic
            .strip_prefix(self.discovery_prefix.as_str())?
            .strip_prefix("/group/")?;
        if id.is_empty() || id.contains('/') {
            return None;
        }
        self.group_store().get(id).is_some().then(|| id.to_owned())
    }

    /// A button appearing or any entity vanishing may change the button topics to subscribe to.
    fn with_button_subscriptions(&self, mut changes: Vec<Change>) -> Vec<Change> {
        if changes.contains(&Change::SubscriptionsChanged) {
            return changes;
        }
        let reg = self.read();
        let touches_button = changes.iter().any(|change| match change {
            Change::EntityUpserted(id) => reg
                .entity(id)
                .is_some_and(|entity| entity.kind == nidoo_core::model::Kind::Button),
            Change::EntityRemoved(_) => true,
            _ => false,
        });
        drop(reg);
        if touches_button {
            changes.push(Change::SubscriptionsChanged);
        }
        changes
    }

    pub fn snapshot(&self) -> Snapshot {
        let connection = self.connection.borrow().clone();
        let mut snapshot = snapshot(&self.read(), connection, clock::now());
        snapshot.alerts = self.alerts();
        snapshot.flows = self.flows();
        snapshot
    }

    /// The Frigate media proxy, when `[frigate] url` is configured.
    pub fn media(&self) -> Option<Arc<MediaProxy>> {
        self.media.clone()
    }

    /// The push service (always built; it sends nothing without `[push] credentials`).
    pub fn push(&self) -> &Arc<PushService> {
        &self.push
    }

    /// The broker accepted the connection: the push token map is rebuilt from scratch.
    pub fn on_connected(&self) {
        self.push.on_connected();
    }

    /// The broker acknowledged the full on-connect subscription (push filters included): the
    /// retained tokens are on their way, the push service starts its settle delay.
    pub fn on_suback(&self) {
        self.push.on_subscribed();
        self.solar_subscribed.send_replace(true);
    }

    /// Turns `true` once the broker acknowledged the on-connect subscription (the solar poller
    /// waits for it before reading the retained cache).
    pub fn solar_subscribed(&self) -> tokio::sync::watch::Receiver<bool> {
        self.solar_subscribed.subscribe()
    }

    /// The bridge's lock is released before the alert goes to the push service.
    fn on_frigate_event(&self, payload: &[u8]) {
        let Some((_, bridge)) = &self.frigate else {
            return;
        };
        let alert = bridge
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .on_event(payload, clock::now());
        if let Some(alert) = alert
            && let Err(rejected) = self.notify("frigate", &alert)
        {
            tracing::warn!(?rejected, "a Frigate alert was rejected");
        }
    }

    /// One Mosquitto log line for the ACL watchdog: from the log topic, or from the tail task
    /// when the source is a file. The watchdog's lock is released before the alert goes out.
    pub fn on_acl_line(&self, line: &str) {
        let Some(acl) = &self.acl_watchdog else {
            return;
        };
        let alert = acl
            .dog
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .on_line(line, clock::now());
        self.raise_system(alert);
    }

    pub(crate) fn raise_system(&self, alert: Option<serde_json::Value>) {
        if let Some(alert) = alert
            && let Err(rejected) = self.notify("system", &alert)
        {
            tracing::warn!(?rejected, "a watchdog alert was rejected");
        }
    }

    /// Topics to subscribe to beyond the push ones: Frigate's events when the bridge is on, the
    /// gateway health states and the broker log topic when their watchdogs are.
    pub fn extra_filters(&self) -> Vec<String> {
        let gateway = self.gateway_watchdog.iter().map(|dog| {
            dog.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .filter()
        });
        let acl = self.acl_watchdog.iter().filter_map(|acl| acl.topic.clone());
        let climate = self
            .climate_prefix()
            .into_iter()
            .flat_map(nidoo_climate::topics::filters)
            // Always: the filters are fixed at (re)connect, and the alarm can be enabled later.
            .chain(std::iter::once(self.push.responses_topic()));
        let alarm = nidoo_alarm::topics::filters(&self.alarm.prefix);
        self.frigate
            .iter()
            .map(|(topic, _)| topic.clone())
            .chain(gateway)
            .chain(acl)
            .chain(climate)
            .chain(alarm)
            .chain(std::iter::once(
                crate::system::services::RETAINED_TOPIC.to_owned(),
            ))
            .chain(std::iter::once(crate::sound::SPOKEN_TOPIC.to_owned()))
            .collect()
    }

    /// Raises a push alert from inside the hub (Frigate bridge, watchdogs, flows, MCP), as if
    /// `payload` arrived on `<prefix>/notification/<source>`. Returns the alert id.
    pub fn notify(&self, source: &str, payload: &serde_json::Value) -> Result<String, Rejected> {
        self.push.submit(source, payload)
    }

    /// The push alerts, newest first, as the retained `<prefix>/history` holds them.
    pub fn alerts(&self) -> Vec<AlertView> {
        self.alerts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .list()
            .to_vec()
    }

    /// Publishes the reply the app would publish when the user taps `action` on alert `id`.
    pub fn reply(&self, id: &str, action: &str) -> Result<(), HubError> {
        {
            let alerts = self
                .alerts
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let alert = alerts
                .find(id)
                .ok_or_else(|| HubError::UnknownAlert(id.to_owned()))?;
            if !alert.actions.iter().any(|a| a.id == action) {
                return Err(HubError::UnknownAction(action.to_owned()));
            }
        }
        let payload = serde_json::to_string(&Reply {
            id,
            action,
            install_id: WEB_INSTALL_ID,
            ts: clock::now(),
        })
        .map_err(|err| HubError::Publish(err.to_string()))?;
        self.publish(Publish {
            topic: self.push.responses_topic(),
            payload,
            retain: false,
        })
    }

    /// A history payload never touches the registry: it is decoded into the alert store and,
    /// when the list changed, broadcast. A payload that is not an array is logged once.
    fn ingest_history(&self, topic: &str, payload: &[u8]) {
        let applied = self
            .alerts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .apply(payload);
        match applied {
            Applied::Changed(alerts) => self.broadcast(WsEvent::AlertsChanged { alerts }),
            Applied::Unchanged => {}
            Applied::Invalid { first_time: true } => {
                tracing::warn!(%topic, "invalid history payload ignored");
            }
            Applied::Invalid { first_time: false } => {}
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WsEvent> {
        self.events.subscribe()
    }

    pub fn connection(&self) -> watch::Receiver<ConnectionStatus> {
        self.connection.subscribe()
    }

    /// Records the broker link's status and broadcasts it when it changed.
    pub fn set_connection(&self, status: ConnectionStatus) {
        let changed = self.connection.send_if_modified(|current| {
            if *current == status {
                false
            } else {
                *current = status.clone();
                true
            }
        });
        if changed {
            if status != ConnectionStatus::Online {
                self.link_epoch.fetch_add(1, Ordering::SeqCst);
            }
            self.broadcast(WsEvent::ConnectionChanged(status));
        }
    }

    /// Every topic some entity reads.
    pub fn subscription_topics(&self) -> BTreeSet<String> {
        let mut topics = self.read().subscription_topics().clone();
        topics.extend(self.watched_button_topics().into_keys());
        topics
    }

    /// The wake-up the MQTT task waits on to re-diff its subscriptions.
    pub fn subscriptions_wakeup(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.subscriptions_changed)
    }

    /// How long the MQTT task waits after a subscription change before diffing.
    pub fn subscription_debounce(&self) -> Duration {
        self.subscription_debounce
    }

    /// Entity and device counts, for logging.
    pub fn counts(&self) -> (usize, usize) {
        let reg = self.read();
        (reg.entities().count(), reg.devices().count())
    }

    /// How many ignored discovery topics are currently logged as such (invalid payloads, not
    /// ignored components); for tests and diagnostics.
    pub fn logged_ignored_len(&self) -> usize {
        self.logged_ignored
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    /// The topics no entity could be made from, with the reason.
    pub fn ignored(&self) -> Vec<IgnoredView> {
        self.read()
            .ignored()
            .iter()
            .map(IgnoredView::from)
            .collect()
    }

    pub fn entity_view(&self, id: &str) -> Option<EntityView> {
        let reg = self.read();
        let entity = reg.entity(id)?;
        let state = reg.state(id).cloned().unwrap_or_default();
        Some(EntityView::from(entity, &state))
    }

    /// The entity's view with its expanded discovery config (topics, templates, payload
    /// constants) and the override as retained, `None` when none is retained for it.
    pub fn entity_detail(&self, id: &str) -> Option<EntityDetail> {
        let reg = self.read();
        let entity = reg.entity(id)?;
        let state = reg.state(id).cloned().unwrap_or_default();
        Some(EntityDetail {
            view: EntityView::from(entity, &state),
            bindings: entity.bindings.config.clone(),
            raw_override: reg.raw_override(id).cloned(),
        })
    }

    /// How many times the broker link left `Online`.
    pub fn link_epoch(&self) -> u64 {
        self.link_epoch.load(Ordering::SeqCst)
    }

    /// A receiver of every id whose state topic was reported, changed or not. The flow engine
    /// uses it to re-seed an entity whose retained replay was identical to its last state,
    /// which the registry (it only emits real changes) does not announce.
    pub fn subscribe_state_seen(&self) -> broadcast::Receiver<StateSeen> {
        self.state_seen.subscribe()
    }

    /// A receiver of every id a state topic message arrived for, read or not (see
    /// [`StateTouched`]).
    pub fn subscribe_state_touched(&self) -> broadcast::Receiver<StateTouched> {
        self.state_touched.subscribe()
    }

    /// Maps `cmd`, publishes it and waits up to `confirm_timeout_ms` for the entity's value to
    /// change or for a report on one of its state topics (a no-op command is echoed
    /// unchanged). An availability message alone confirms nothing. An unknown or offline
    /// entity is rejected before anything is published, and a broker link that is down is
    /// [`HubError::Publish`] right away rather than a wait for an echo that cannot come.
    pub async fn send_command(&self, id: &str, cmd: Command) -> Result<CommandResponse, HubError> {
        if id == ALARM_ENTITY_ID {
            let Some(hub) = self.me.upgrade() else {
                return Err(HubError::Rejected("hub is shutting down".to_owned()));
            };
            return Ok(hub.alarm_command(cmd).await);
        }
        if let Some(group_id) = id.strip_prefix("group_") {
            if !self.groups_enabled {
                return Ok(CommandResponse {
                    accepted: false,
                    reason: Some("groups disabled".to_owned()),
                    state: None,
                });
            }
            return Ok(self.on_group_command(group_id, cmd).await);
        }
        // Subscribed before publishing, so a fast echo cannot slip between the two.
        let mut events = self.subscribe();
        let mut seen = self.state_seen.subscribe();
        let (publishes, before, no_echo_expected, kind) = {
            let reg = self.read();
            let entity = reg
                .entity(id)
                .ok_or_else(|| HubError::UnknownEntity(id.to_owned()))?;
            let state = reg.state(id).cloned().unwrap_or_default();
            if state.availability == Availability::Offline {
                return Err(HubError::Rejected("entity is offline".to_owned()));
            }
            // A generic control with no state topic to echo on (a press, a scene) is done
            // once published.
            let no_echo_expected = match &cmd {
                Command::Control { control, .. } if entity.kind == Kind::Generic => {
                    nidoo_core::types::type_def(&entity.component)
                        .is_some_and(|def| !expects_echo(def, entity, control))
                }
                _ => false,
            };
            (
                map_command(entity, &state, &cmd, reg.templates())?,
                state.value,
                no_echo_expected,
                entity.protocol_kind,
            )
        };
        for publish in publishes {
            self.publish(publish)?;
            self.commands_sent.fetch_add(1, Ordering::Relaxed);
        }
        if no_echo_expected {
            return Ok(CommandResponse {
                accepted: true,
                reason: None,
                state: None,
            });
        }

        let confirmed = tokio::time::timeout(self.confirm_timeout, async {
            loop {
                tokio::select! {
                    event = events.recv() => match event {
                        // Only a value change counts here: an availability flip is a
                        // `StateChanged` too, and an echo with the same value comes through
                        // `state_seen` (which excludes availability-only topics).
                        Ok(WsEvent::StateChanged { id: changed, state })
                            if changed == id
                                && state.value != before
                                && echo_confirms(kind, &cmd, &before, &state.value) =>
                        {
                            return Some(state);
                        }
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => return None,
                    },
                    seen_id = seen.recv() => match seen_id {
                        Ok(seen_id) if seen_id.id == id => {
                            let state = self.read().state(id).cloned().unwrap_or_default();
                            // A media player's state also moves when someone else uses it:
                            // only the report its command asked for is the echo.
                            if echo_confirms(kind, &cmd, &before, &state.value) {
                                return Some(state);
                            }
                        }
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => return None,
                    },
                }
            }
        })
        .await
        .ok()
        .flatten();

        match confirmed {
            Some(mut state) => {
                if let Some(entity) = self.read().entity(id) {
                    mask_secret(entity, &mut state);
                }
                Ok(CommandResponse {
                    accepted: true,
                    reason: None,
                    state: Some(state),
                })
            }
            None => {
                let reason = format!(
                    "no state change within {} ms",
                    self.confirm_timeout.as_millis()
                );
                self.broadcast(WsEvent::CommandFailed {
                    id: id.to_owned(),
                    reason: reason.clone(),
                });
                Ok(CommandResponse {
                    accepted: false,
                    reason: Some(reason),
                    state: None,
                })
            }
        }
    }

    /// The present members of `spec`, as views.
    fn group_member_views(&self, spec: &GroupSpec) -> Vec<EntityView> {
        let reg = self.read();
        spec.members
            .iter()
            .filter_map(|member| {
                let entity = reg.entity(member)?;
                let state = reg.state(member).cloned().unwrap_or_default();
                Some(EntityView::from(entity, &state))
            })
            .collect()
    }

    /// Fans `cmd` out to the reachable members of group `id` and waits up to two seconds for
    /// each of them to report. The group's own state is not published here: it follows the
    /// members' echoes ([`Self::on_member_state_change`]).
    pub async fn on_group_command(&self, id: &str, cmd: Command) -> CommandResponse {
        let reject = |reason: String| CommandResponse {
            accepted: false,
            reason: Some(reason),
            state: None,
        };
        if !self.groups_enabled {
            return reject("groups disabled".to_owned());
        }
        let Some(spec) = self.group_store().get(id).cloned() else {
            return reject("unknown group".to_owned());
        };
        // Subscribed before publishing, so a fast echo cannot slip between the two.
        let mut seen = self.state_seen.subscribe();
        let members = self.group_member_views(&spec);
        let mut targeted = BTreeSet::new();
        let mut publishes = Vec::new();
        {
            let reg = self.read();
            for target in groups::fanout::targets(&spec, &members) {
                let Some(member) = members.iter().find(|m| m.id == target) else {
                    continue;
                };
                let Some(member_cmd) = groups::fanout::translate(&spec, &cmd, member) else {
                    continue;
                };
                let (Some(entity), Some(state)) = (reg.entity(&target), reg.state(&target)) else {
                    continue;
                };
                match map_command(entity, state, &member_cmd, reg.templates()) {
                    Ok(mapped) => {
                        publishes.extend(mapped);
                        targeted.insert(target);
                    }
                    Err(err) => {
                        tracing::warn!(group = %id, member = %target, %err, "group member skipped")
                    }
                }
            }
        }
        let mut waiting: Vec<String> = spec
            .members
            .iter()
            .filter(|m| targeted.contains(*m))
            .cloned()
            .collect();
        if waiting.is_empty() {
            return reject("no reachable member".to_owned());
        }
        for publish in publishes {
            if let Err(err) = self.publish(publish) {
                return reject(err.to_string());
            }
            self.commands_sent.fetch_add(1, Ordering::Relaxed);
        }
        let window = Duration::from_millis(2000);
        let _ = tokio::time::timeout(window, async {
            while !waiting.is_empty() {
                match seen.recv().await {
                    Ok(report) => {
                        waiting.retain(|m| *m != report.id);
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        })
        .await;
        if waiting.is_empty() {
            return CommandResponse {
                accepted: true,
                reason: None,
                state: None,
            };
        }
        let reason = format!("{}: no echo", waiting.join(", "));
        self.broadcast(WsEvent::CommandFailed {
            id: groups::topics::unique_id(id),
            reason: reason.clone(),
        });
        reject(reason)
    }

    /// Recomputes and republishes every group that lists `member_id`: the combined state when
    /// it changed, and the availability (online while any targeted member is reachable). A
    /// group is never a member of a group, so group ids are ignored, which also keeps the
    /// group's own state echoes from looping back here.
    pub fn on_member_state_change(&self, member_id: &str) {
        if member_id.starts_with("group_") {
            return;
        }
        let affected: Vec<(String, GroupSpec)> = self
            .group_store()
            .all()
            .iter()
            .filter(|(_, spec)| spec.members.iter().any(|m| m == member_id))
            .map(|(id, spec)| (id.clone(), spec.clone()))
            .collect();
        let prefix = &self.discovery_prefix;
        for (group_id, spec) in affected {
            let Some(group) = self.entity_view(&groups::topics::unique_id(&group_id)) else {
                continue;
            };
            let members = self.group_member_views(&spec);
            let reachable = !groups::fanout::targets(&spec, &members).is_empty();
            let matching: Vec<EntityView> = members
                .into_iter()
                .filter(|m| m.kind == spec.kind)
                .collect();

            if let Some(state) = groups::state::combine(spec.kind, &matching) {
                let topic = groups::topics::state_topic(prefix, &group_id);
                let payload = groups::state::encode(&state);
                self.ingest(&topic, payload.as_bytes());
                let changed = self
                    .entity_view(&group.id)
                    .is_some_and(|now| now.state.value != group.state.value);
                if changed {
                    self.republish(topic, payload);
                }
            }

            let topic = groups::topics::availability_topic(prefix, &group_id);
            let payload = if reachable { "online" } else { "offline" };
            self.ingest(&topic, payload.as_bytes());
            let flipped = self
                .entity_view(&group.id)
                .is_some_and(|now| now.state.availability != group.state.availability);
            if flipped {
                self.republish(topic, payload.to_owned());
            }
        }
    }

    /// Publishes a group topic retained; a refused publish is logged, the local copy stays.
    fn republish(&self, topic: String, payload: String) {
        if let Err(err) = self.publish(Publish {
            topic: topic.clone(),
            payload,
            retain: true,
        }) {
            tracing::warn!(%topic, %err, "group publish refused");
        }
    }

    /// Patches the entity's override, publishes it retained and applies it locally so the
    /// returned view already reflects it (the broker's echo is then a no-op). The `area` is
    /// stored canonically: a declared name becomes its id, a blank string clears the field.
    pub async fn set_override(
        &self,
        id: &str,
        patch: &OverridePatch,
    ) -> Result<EntityView, HubError> {
        let (current, announced) = {
            let reg = self.read();
            let Some(entity) = reg.entity(id) else {
                return Err(HubError::UnknownEntity(id.to_owned()));
            };
            let announced = entity
                .capabilities
                .generic
                .as_ref()
                .map(|g| g.controls.clone())
                .unwrap_or_default();
            (reg.raw_override(id).cloned().unwrap_or_default(), announced)
        };
        let mut next = current.apply_patch(patch)?;
        if let Some(main) = next.main_control.as_deref()
            && main != "none"
            && !announced.iter().any(|c| c == main)
        {
            return Err(HubError::Patch(PatchError::InvalidValue {
                field: "mainControl".to_owned(),
                expected: "one of the entity's announced controls or \"none\"".to_owned(),
            }));
        }
        next.area = next
            .area
            .as_deref()
            .map(str::trim)
            .filter(|area| !area.is_empty())
            .map(|area| self.read().resolve_area(area));
        self.write_override(id, &next)
    }

    /// Removes the entity's override (an empty retained payload).
    pub async fn clear_override(&self, id: &str) -> Result<EntityView, HubError> {
        if self.read().entity(id).is_none() {
            return Err(HubError::UnknownEntity(id.to_owned()));
        }
        self.write_override(id, &UiOverride::default())
    }

    /// The declared areas document as last retained.
    pub fn areas_document(&self) -> AreasDocument {
        self.read().areas_document().clone()
    }

    /// Replaces the declared areas: publishes the document retained and applies it locally.
    pub async fn set_areas(&self, doc: AreasDocument) -> Result<Vec<AreaView>, HubError> {
        let topic = format!("{}/_areas", self.ui_prefix);
        let payload = doc.encode();
        self.publish(Publish {
            topic: topic.clone(),
            payload: payload.clone(),
            retain: true,
        })?;
        self.ingest(&topic, payload.as_bytes());
        Ok(area_views(&self.read()))
    }

    /// Publishes the entity type table retained on `<ui_prefix>/_types`, for the app.
    pub async fn publish_types(&self) -> Result<(), HubError> {
        self.publish(Publish {
            topic: format!("{}/_types", self.ui_prefix),
            payload: nidoo_core::types::table_json().to_owned(),
            retain: true,
        })
    }

    /// Renames area `id` in the declared document, the app's other fields kept; an implicit
    /// area (carried only by entities) becomes declared. `Unassigned` cannot be renamed.
    pub async fn rename_area(&self, id: &str, name: &str) -> Result<Vec<AreaView>, HubError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(HubError::Rejected(
                "an area name cannot be blank".to_owned(),
            ));
        }
        if id == UNASSIGNED_AREA {
            return Err(HubError::Rejected(
                "`Unassigned` cannot be renamed".to_owned(),
            ));
        }
        let mut doc = {
            let reg = self.read();
            if !reg.effective_areas().iter().any(|area| area.id == id) {
                return Err(HubError::UnknownArea(id.to_owned()));
            }
            reg.areas_document().clone()
        };
        doc.rename(id, name);
        self.set_areas(doc).await
    }

    /// Creates or replaces virtual entity `id`: publishes its discovery config retained and
    /// applies it locally. The initial state (`OFF`, the number's `min`, or `initial`) is
    /// published retained only when the entity did not exist, so redefining one keeps its
    /// state (a select whose state left its options goes back to its first option); a button
    /// has none. A changed kind clears the old config topic.
    pub async fn set_virtual(&self, id: &str, spec: VirtualSpec) -> Result<EntityView, HubError> {
        virtuals::validate_id(id).map_err(HubError::Rejected)?;
        let initial = virtuals::initial_state(&spec).map_err(HubError::Rejected)?;
        let unique_id = virtuals::unique_id(id);
        let previous = self
            .read()
            .entity(&unique_id)
            .map(|entity| entity.discovery_topic.clone());

        let config_topic = virtuals::config_topic(&self.discovery_prefix, spec.kind, id);
        let config = virtuals::build_config(&self.discovery_prefix, id, &spec).to_string();
        self.publish(Publish {
            topic: config_topic.clone(),
            payload: config.clone(),
            retain: true,
        })?;
        if let Some(old) = previous.as_deref().filter(|old| {
            *old != config_topic && virtuals::is_config_topic(&self.discovery_prefix, id, old)
        }) {
            self.publish(Publish {
                topic: old.to_owned(),
                payload: String::new(),
                retain: true,
            })?;
        }
        self.ingest(&config_topic, config.as_bytes());

        // A new entity gets its initial state; a redefined select whose current value fell
        // outside the new options goes back to the first option.
        let reset = match (&previous, &spec.options) {
            (None, _) => true,
            (Some(_), Some(options)) if spec.kind == VirtualKind::Select => !matches!(
                self.entity_view(&unique_id).map(|view| view.state.value),
                Some(StateValue::Text(current)) if options.contains(&current)
            ),
            _ => false,
        };
        let state = match (&previous, &spec.options) {
            (Some(_), Some(options)) if spec.kind == VirtualKind::Select => {
                options.first().cloned()
            }
            _ => initial,
        };
        if let (true, Some(state)) = (reset, state) {
            let topic = virtuals::state_topic(&self.discovery_prefix, id);
            self.publish(Publish {
                topic: topic.clone(),
                payload: state.clone(),
                retain: true,
            })?;
            self.ingest(&topic, state.as_bytes());
        }
        self.entity_view(&unique_id)
            .ok_or(HubError::UnknownEntity(unique_id))
    }

    /// Removes virtual entity `id`: an empty retained payload on its config topic and on its
    /// state topic, so the broker forgets both. Anything that is not a virtual entity is unknown.
    pub async fn delete_virtual(&self, id: &str) -> Result<(), HubError> {
        let unique_id = virtuals::unique_id(id);
        let config_topic = self
            .read()
            .entity(&unique_id)
            .map(|entity| entity.discovery_topic.clone())
            .filter(|topic| virtuals::is_config_topic(&self.discovery_prefix, id, topic))
            .ok_or(HubError::UnknownEntity(unique_id))?;
        self.publish(Publish {
            topic: config_topic.clone(),
            payload: String::new(),
            retain: true,
        })?;
        self.publish(Publish {
            topic: virtuals::state_topic(&self.discovery_prefix, id),
            payload: String::new(),
            retain: true,
        })?;
        self.ingest(&config_topic, b"");
        Ok(())
    }

    /// Fails with a `Publish` error (503 over HTTP) while `[groups] enabled` is false.
    fn require_groups(&self) -> Result<(), HubError> {
        if self.groups_enabled {
            Ok(())
        } else {
            Err(HubError::Publish("groups disabled".to_owned()))
        }
    }

    /// Fails with a `Publish` error (503 over HTTP) while `[climate] enabled` is false.
    pub(crate) fn require_climate(&self) -> Result<&ClimateHub, HubError> {
        self.climate
            .as_ref()
            .ok_or_else(|| HubError::Publish("climate disabled".to_owned()))
    }

    /// Creates or replaces group `id`: validates everything first, then publishes the
    /// discovery config, availability and definition retained, and the combined state only
    /// when the group is new. A changed component clears the old config topic.
    pub async fn set_group(&self, id: &str, spec: GroupSpec) -> Result<GroupView, HubError> {
        self.require_groups()?;
        groups::topics::validate_id(id).map_err(HubError::Rejected)?;
        groups::topics::component_for(spec.kind).map_err(HubError::Rejected)?;
        if let Some(nested) = spec.members.iter().find(|m| m.starts_with("group_")) {
            return Err(HubError::Rejected(format!(
                "members cannot be groups: {nested}"
            )));
        }
        let previous = self.group_store().get(id).cloned();
        let mut members = Vec::new();
        for member in &spec.members {
            let Some(view) = self.entity_view(member) else {
                continue;
            };
            if view.kind == spec.kind {
                members.push(view);
            } else if !previous
                .as_ref()
                .is_some_and(|p| p.members.contains(member))
            {
                return Err(HubError::Rejected(format!(
                    "member kind does not match: {member}"
                )));
            }
        }

        let prefix = &self.discovery_prefix;
        let config_topic = groups::topics::config_topic(prefix, spec.kind, id);
        let config = groups::discovery::build_config(prefix, id, &spec, &members).to_string();
        let definition = serde_json::to_string(&spec)
            .map_err(|err| HubError::Rejected(format!("cannot encode the group: {err}")))?;
        self.publish(Publish {
            topic: config_topic.clone(),
            payload: config.clone(),
            retain: true,
        })?;
        self.publish(Publish {
            topic: groups::topics::availability_topic(prefix, id),
            payload: "online".to_owned(),
            retain: true,
        })?;
        let definition_topic = groups::topics::definition_topic(prefix, id);
        self.publish(Publish {
            topic: definition_topic.clone(),
            payload: definition.clone(),
            retain: true,
        })?;
        match &previous {
            None => {
                let payload = groups::state::combine(spec.kind, &members)
                    .map(|state| groups::state::encode(&state))
                    .unwrap_or_default();
                self.publish(Publish {
                    topic: groups::topics::state_topic(prefix, id),
                    payload,
                    retain: true,
                })?;
            }
            Some(old) if old.kind != spec.kind => {
                self.publish(Publish {
                    topic: groups::topics::config_topic(prefix, old.kind, id),
                    payload: String::new(),
                    retain: true,
                })?;
            }
            Some(_) => {}
        }
        self.ingest(&definition_topic, definition.as_bytes());
        self.ingest(&config_topic, config.as_bytes());
        self.group_view(id, &spec)
    }

    /// Removes group `id`: empty retained payloads on its definition, config, state and
    /// availability topics. The UI override is left in place.
    pub async fn delete_group(&self, id: &str) -> Result<(), HubError> {
        self.require_groups()?;
        let spec = self
            .group_store()
            .get(id)
            .cloned()
            .ok_or_else(|| HubError::UnknownEntity(groups::topics::unique_id(id)))?;
        let prefix = &self.discovery_prefix;
        let config_topic = groups::topics::config_topic(prefix, spec.kind, id);
        let definition_topic = groups::topics::definition_topic(prefix, id);
        for topic in [
            definition_topic.clone(),
            config_topic.clone(),
            groups::topics::state_topic(prefix, id),
            groups::topics::availability_topic(prefix, id),
        ] {
            self.publish(Publish {
                topic,
                payload: String::new(),
                retain: true,
            })?;
        }
        self.ingest(&definition_topic, b"");
        self.ingest(&config_topic, b"");
        Ok(())
    }

    /// Turns the derived room groups into hand-made groups once: refused while the switch is
    /// off, once the retained marker is set, or while the house is still empty (the retained
    /// sync has not settled, and a marker published now would lock the migration out).
    /// A group that fails to be created is logged and skipped.
    pub async fn run_group_migration_once(&self) -> MigrationOutcome {
        let skipped = |reason| MigrationOutcome {
            created: Vec::new(),
            skipped_reason: Some(reason),
        };
        if !self.groups_enabled {
            return skipped("groups disabled");
        }
        if self.group_store().migrated() {
            return skipped("already migrated");
        }
        let (entities, areas) = {
            let reg = self.read();
            if reg.entities().next().is_none() {
                return skipped("house empty, retained sync not settled");
            }
            let entities: std::collections::BTreeMap<String, EntityView> = entity_views(&reg)
                .into_iter()
                .map(|view| (view.id.clone(), view))
                .collect();
            (entities, area_views(&reg))
        };
        let mut created = Vec::new();
        for (id, spec) in groups::migrate::plan(&entities, &areas) {
            match self.set_group(&id, spec).await {
                Ok(_) => created.push(id),
                Err(err) => tracing::warn!(group = %id, %err, "group migration skipped a group"),
            }
        }
        let marker = Publish {
            topic: groups::topics::migration_marker_topic(&self.discovery_prefix),
            payload: "1".to_owned(),
            retain: true,
        };
        if let Err(err) = self.publish(marker) {
            tracing::warn!(%err, "group migration marker not published; it will run again");
            return MigrationOutcome {
                created,
                skipped_reason: None,
            };
        }
        self.group_store().mark_migrated();
        MigrationOutcome {
            created,
            skipped_reason: None,
        }
    }

    /// Every stored group that has its entity, with its members and the flows that use it,
    /// sorted by id.
    pub fn list_groups(&self) -> Result<Vec<GroupView>, HubError> {
        self.require_groups()?;
        let stored: Vec<(String, GroupSpec)> = self
            .group_store()
            .all()
            .iter()
            .map(|(id, spec)| (id.clone(), spec.clone()))
            .collect();
        // A definition without its discovery entity (not loaded yet, or lost) is skipped
        // rather than failing the whole listing.
        Ok(stored
            .iter()
            .filter_map(|(id, spec)| self.group_view(id, spec).ok())
            .collect())
    }

    fn group_view(&self, id: &str, spec: &GroupSpec) -> Result<GroupView, HubError> {
        let unique_id = groups::topics::unique_id(id);
        let entity = self
            .entity_view(&unique_id)
            .ok_or_else(|| HubError::UnknownEntity(unique_id.clone()))?;
        let members = spec
            .members
            .iter()
            .map(|member| match self.entity_view(member) {
                Some(view) => GroupMemberView {
                    id: member.clone(),
                    name: view.name,
                    present: true,
                    mismatch: view.kind != spec.kind,
                },
                None => GroupMemberView {
                    id: member.clone(),
                    name: member.clone(),
                    present: false,
                    mismatch: false,
                },
            })
            .collect();
        let used_by_flows = self
            .flow_store()
            .flows()
            .iter()
            .filter(|flow| {
                flow.nodes
                    .iter()
                    .any(|node| node_entity(&node.kind) == Some(unique_id.as_str()))
            })
            .map(|flow| flow.name.clone())
            .collect();
        Ok(GroupView {
            id: id.to_owned(),
            name: spec.name.clone(),
            kind: spec.kind,
            members,
            hide_members: spec.hide_members,
            entity,
            used_by_flows,
        })
    }

    fn write_override(&self, id: &str, next: &UiOverride) -> Result<EntityView, HubError> {
        let topic = format!("{}/{id}", self.ui_prefix);
        let payload = if next.is_default() {
            String::new()
        } else {
            next.encode()
        };
        self.publish(Publish {
            topic: topic.clone(),
            payload: payload.clone(),
            retain: true,
        })?;
        self.ingest(&topic, payload.as_bytes());
        self.entity_view(id)
            .ok_or_else(|| HubError::UnknownEntity(id.to_owned()))
    }

    /// Hands a publish to the MQTT task without waiting. While the broker link is not
    /// `Online` the publish is refused: a write accepted during an outage would show in the
    /// hub, never reach the broker and vanish at the next restart. A full queue (64 publishes
    /// already pending) is an error for the caller too, not a hang.
    pub(crate) fn publish(&self, publish: Publish) -> Result<(), HubError> {
        if !matches!(*self.connection.borrow(), ConnectionStatus::Online) {
            return Err(HubError::Publish("broker offline".to_owned()));
        }
        self.publisher.try_send(publish).map_err(|err| match err {
            mpsc::error::TrySendError::Full(_) => {
                HubError::Publish("publish queue is full".to_owned())
            }
            mpsc::error::TrySendError::Closed(_) => {
                HubError::Publish("the MQTT task is gone".to_owned())
            }
        })
    }

    pub(crate) fn broadcast(&self, event: WsEvent) {
        // An error only means nobody is listening right now.
        let _ = self.events.send(event);
    }

    /// Warns once per failing entity; an entity that recovers and fails again is warned again.
    fn log_new_template_failures(&self, failing: &BTreeSet<String>) {
        let mut logged = self
            .logged_failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        logged.retain(|id| failing.contains(id));
        for id in failing {
            if logged.insert(id.clone()) {
                tracing::warn!(entity = %id, "template failure");
            }
        }
    }

    /// Warns once per ignored discovery topic and reason, leaving out the components the hub
    /// ignores on purpose; a topic that recovers and breaks again is warned again.
    fn log_new_ignored(&self, ignored: &[Ignored]) {
        let mut logged = self
            .logged_ignored
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        logged.retain(|(topic, reason)| {
            ignored
                .iter()
                .any(|i| i.topic == *topic && i.reason == *reason)
        });
        for entry in ignored {
            if entry.reason.starts_with("ignored component") {
                continue;
            }
            if logged.insert((entry.topic.clone(), entry.reason.clone())) {
                tracing::warn!(topic = %entry.topic, reason = %entry.reason, "discovery payload ignored");
            }
        }
    }

    /// The flow store, taken as it is when a thread panicked while holding it.
    /// The flow label catalog, taken as it is when a thread panicked while holding it.
    pub(crate) fn flow_label_slot(&self) -> MutexGuard<'_, nidoo_core::flow::FlowLabelCatalog> {
        self.flow_labels
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn flow_store(&self) -> MutexGuard<'_, FlowStore> {
        self.flow_store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The group store, taken as it is when a thread panicked while holding it.
    pub(crate) fn group_store(&self) -> MutexGuard<'_, GroupStore> {
        self.group_store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Read access to the group store, for tests and diagnostics.
    pub fn read_group_store(&self) -> MutexGuard<'_, GroupStore> {
        self.group_store()
    }

    /// Feeds a group definition or the migration marker to the store; false when `topic` is
    /// neither, so state and availability topics fall through to the registry.
    fn ingest_group(&self, topic: &str, payload: &[u8]) -> bool {
        let id = if topic == groups::topics::migration_marker_topic(&self.discovery_prefix) {
            "_migrated"
        } else {
            let Some(id) = topic
                .strip_prefix(self.discovery_prefix.as_str())
                .and_then(|rest| rest.strip_prefix("/group/"))
                .and_then(|rest| rest.strip_suffix("/definition"))
            else {
                return false;
            };
            if id.is_empty() || id.contains('/') {
                return false;
            }
            id
        };
        let outcome = self.group_store().apply(id, payload);
        match &outcome {
            ApplyOutcome::Rejected(reason) => {
                tracing::warn!(%id, %reason, "group definition rejected");
            }
            ApplyOutcome::Created | ApplyOutcome::Replaced | ApplyOutcome::Deleted => {
                tracing::info!(%id, ?outcome, "group definition applied");
            }
            ApplyOutcome::Marker | ApplyOutcome::Unchanged => {}
        }
        true
    }

    /// Feeds a network watch definition to the store; false when `topic` is not one, so the
    /// config, state and availability topics fall through to the registry.
    fn ingest_network(&self, topic: &str, payload: &[u8]) -> bool {
        let Some(id) = topic
            .strip_prefix(self.discovery_prefix.as_str())
            .and_then(|rest| rest.strip_prefix("/network/"))
            .and_then(|rest| rest.strip_suffix("/definition"))
        else {
            return false;
        };
        if id.is_empty() || id.contains('/') {
            return false;
        }
        let outcome = self.network_store().apply(id, payload);
        match &outcome {
            ApplyOutcome::Rejected(reason) => {
                tracing::warn!(%id, %reason, "network definition rejected");
            }
            ApplyOutcome::Created | ApplyOutcome::Replaced | ApplyOutcome::Deleted => {
                tracing::info!(%id, ?outcome, "network definition applied");
                let rev = self.network_store().rev();
                self.network_rev.send_replace(rev);
            }
            ApplyOutcome::Marker | ApplyOutcome::Unchanged => {}
        }
        true
    }

    /// The network store, taken as it is when a thread panicked while holding it.
    pub(crate) fn network_store(&self) -> MutexGuard<'_, NetworkStore> {
        self.network_store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A receiver of the network store revision.
    pub(crate) fn subscribe_network_rev(&self) -> watch::Receiver<u64> {
        self.network_rev.subscribe()
    }

    /// Records the last verdict of watch `id`.
    pub(crate) fn set_network_status(&self, id: &str, status: WatchStatus) {
        self.network_status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id.to_owned(), status);
    }

    /// Asks the flow engine to do something; nobody listening is fine (no engine started).
    pub(crate) fn send_engine(&self, request: EngineRequest) {
        let _ = self.engine_tx.send(request);
    }

    /// The topic of flow `id`.
    pub(crate) fn flow_topic(&self, id: &str) -> String {
        format!("{}/{id}", self.automation_prefix)
    }

    /// The flow id when `topic` is exactly `<automation_prefix>/<id>`; deeper topics are not flows.
    fn flow_id_of<'a>(&self, topic: &'a str) -> Option<&'a str> {
        let id = topic
            .strip_prefix(self.automation_prefix.as_str())?
            .strip_prefix('/')?;
        (!id.is_empty() && !id.contains('/')).then_some(id)
    }

    pub(crate) fn dashboard_store(&self) -> MutexGuard<'_, DashboardStore> {
        self.dashboard_store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The topic of dashboard `id`.
    pub fn dashboard_topic(&self, id: &str) -> String {
        format!("{}/dashboards/{id}", self.ui_prefix)
    }

    /// The dashboard id when `topic` is exactly `<ui_prefix>/dashboards/<id>`.
    fn dashboard_id_of<'a>(&self, topic: &'a str) -> Option<&'a str> {
        let id = topic
            .strip_prefix(self.ui_prefix.as_str())?
            .strip_prefix("/dashboards/")?;
        (!id.is_empty() && !id.contains('/')).then_some(id)
    }

    /// Applies a retained dashboard payload to the store.
    pub(crate) fn ingest_dashboard(&self, id: &str, payload: &[u8]) {
        self.dashboard_store().apply(id, payload);
    }

    pub(crate) fn ambiance_store(&self) -> MutexGuard<'_, AmbianceStore> {
        self.ambiance_store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The topic of ambiance `id`.
    pub fn ambiance_topic(&self, id: &str) -> String {
        format!("{}/ambiances/{id}", self.ui_prefix)
    }

    /// The ambiance id when `topic` is exactly `<ui_prefix>/ambiances/<id>`.
    fn ambiance_id_of<'a>(&self, topic: &'a str) -> Option<&'a str> {
        let id = topic
            .strip_prefix(self.ui_prefix.as_str())?
            .strip_prefix("/ambiances/")?;
        (!id.is_empty() && !id.contains('/')).then_some(id)
    }

    /// The receiving end of the engine requests, handed out once.
    pub fn take_engine_requests(&self) -> Option<mpsc::UnboundedReceiver<EngineRequest>> {
        self.engine_rx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    /// A poisoned lock means a thread panicked while holding it; the registry never panics on
    /// a hostile payload, so the state is taken as it is and the fact is logged.
    pub(crate) fn read(&self) -> RwLockReadGuard<'_, Registry> {
        self.registry.read().unwrap_or_else(|poisoned| {
            tracing::error!("registry lock was poisoned; continuing with its last state");
            poisoned.into_inner()
        })
    }

    fn write(&self) -> RwLockWriteGuard<'_, Registry> {
        self.registry.write().unwrap_or_else(|poisoned| {
            tracing::error!("registry lock was poisoned; continuing with its last state");
            poisoned.into_inner()
        })
    }
}

/// The number history plots for a state: the type table's mapping for a generic entity,
/// the kind's own for the others.
fn history_num_of(entity: &Entity, value: &StateValue) -> Option<f64> {
    match value {
        StateValue::Generic(gs) => nidoo_core::types::type_def(&entity.component)
            .and_then(|def| nidoo_core::types::history_num(def, gs)),
        other => nidoo_core::history::num_of(other),
    }
}

/// The WebSocket events for `changes`, built from the registry right after they were applied.
/// `IgnoredChanged` and `SubscriptionsChanged` are internal and produce none.
fn events_for(reg: &Registry, changes: &[Change]) -> Vec<WsEvent> {
    changes
        .iter()
        .filter_map(|change| match change {
            Change::EntityUpserted(id) => reg.entity(id).map(|entity| {
                let state = reg.state(id).cloned().unwrap_or_default();
                WsEvent::EntityUpserted(EntityView::from(entity, &state))
            }),
            Change::EntityRemoved(id) => Some(WsEvent::EntityRemoved { id: id.clone() }),
            Change::StateChanged(id) => reg.state(id).map(|state| {
                let mut state = state.clone();
                if let Some(entity) = reg.entity(id) {
                    mask_secret(entity, &mut state);
                }
                WsEvent::StateChanged {
                    id: id.clone(),
                    state,
                }
            }),
            Change::AreasChanged => Some(WsEvent::AreasChanged {
                areas: area_views(reg),
            }),
            Change::DevicesChanged => Some(WsEvent::DevicesChanged {
                devices: device_views(reg),
            }),
            Change::BridgeChanged => Some(WsEvent::BridgeChanged(reg.bridge().into())),
            // A report that changed nothing is not pushed to clients; it only confirms commands.
            Change::StateReported(_)
            | Change::StateTouched(_)
            | Change::IgnoredChanged
            | Change::SubscriptionsChanged => None,
        })
        .collect()
}

/// The entity id a flow node refers to, when its type carries one.
fn node_entity(kind: &nidoo_core::flow::NodeKind) -> Option<&str> {
    use nidoo_core::flow::NodeKind;
    match kind {
        NodeKind::TriggerState(params) => Some(params.entity.as_str()),
        NodeKind::Command(params) => Some(params.entity.as_str()),
        NodeKind::If(params) => params.entity.as_deref(),
        NodeKind::Switch(params) => params.entity.as_deref(),
        _ => None,
    }
}
