//! Layered configuration: defaults, then `oikos.toml`, then `OIKOS_*` env vars.

use std::fmt;
use std::path::{Path, PathBuf};

use figment::Figment;
use figment::providers::{Env, Format, Serialized, Toml};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub mqtt: MqttConfig,
    pub topics: TopicsConfig,
    pub http: HttpConfig,
    pub commands: CommandsConfig,
    pub log: LogConfig,
    pub push: PushConfig,
    pub frigate: FrigateConfig,
    pub watchdog: WatchdogConfig,
    pub automation: AutomationConfig,
    pub flows: FlowsConfig,
    pub groups: GroupsConfig,
    pub network: NetworkConfig,
    pub climate: ClimateConfig,
    pub alarm: AlarmConfig,
    pub history: HistoryConfig,
    pub weather: WeatherConfig,
    pub solcast: SolcastConfig,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub password: Option<String>,
    pub client_id: String,
    pub send_ha_birth: bool,
    pub ha_status_topic: String,
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            host: "localhost".into(),
            port: 1883,
            user: None,
            password: None,
            client_id: "oikos-os".into(),
            send_ha_birth: false,
            ha_status_topic: "homeassistant/status".into(),
        }
    }
}

impl fmt::Debug for MqttConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let password = self.password.as_ref().map(|_| "***");
        f.debug_struct("MqttConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &password)
            .field("client_id", &self.client_id)
            .field("send_ha_birth", &self.send_ha_birth)
            .field("ha_status_topic", &self.ha_status_topic)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TopicsConfig {
    pub discovery_prefixes: Vec<String>,
    pub ui_prefix: String,
    pub z2m_base: String,
    /// Where flow definitions live: `<prefix>/<flow id>`, retained, one JSON document each.
    pub automation_prefix: String,
}

impl Default for TopicsConfig {
    fn default() -> Self {
        Self {
            discovery_prefixes: vec!["oikos".into()],
            ui_prefix: "oikos/ui".into(),
            z2m_base: "zigbee2mqtt".into(),
            automation_prefix: "oikos/automation".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HttpConfig {
    pub bind: String,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:8081".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CommandsConfig {
    pub confirm_timeout_ms: u64,
    pub subscription_debounce_ms: u64,
}

impl Default for CommandsConfig {
    fn default() -> Self {
        Self {
            confirm_timeout_ms: 5000,
            subscription_debounce_ms: 500,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LogConfig {
    pub level: String,
    pub json: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
            json: false,
        }
    }
}

/// Push notifications, owned by the hub: producers publish on `<prefix>/notification/<source>`,
/// the app registers on `<prefix>/tokens/<installId>` and replies on `<prefix>/responses`, the
/// hub keeps the retained `<prefix>/history`. The legacy OikosPush topics under
/// `legacy_prefix` are read too (empty disables them). Without `credentials` (a Firebase
/// service-account key file) the hub keeps the history and sends nothing.
///
/// Unknown keys are refused, so a config still setting the old `history_topic` /
/// `responses_topic` fails at start naming the keys it accepts.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct PushConfig {
    pub prefix: String,
    pub legacy_prefix: String,
    /// Path of the service-account JSON key; only the path is ever logged. When unset, `load`
    /// uses [`PUSH_KEY_FILE`] next to the config file if it exists.
    pub credentials: Option<PathBuf>,
    /// The service-account key JSON itself (set from the browser); wins over `credentials`.
    pub service_account: Option<String>,
    /// Seconds between two passes of the retry queue.
    pub retry_interval: u64,
    /// Attempts before a queued alert is given up.
    pub max_retries: u32,
    /// Alerts kept in the history.
    pub history_size: usize,
    /// Days an alert stays in the history.
    pub history_days: u64,
}

impl Default for PushConfig {
    fn default() -> Self {
        Self {
            prefix: "oikos/push".into(),
            legacy_prefix: "oikos-push".into(),
            credentials: None,
            service_account: None,
            retry_interval: 60,
            max_retries: 10,
            history_size: 100,
            history_days: 7,
        }
    }
}

impl fmt::Debug for PushConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let service_account = self.service_account.as_ref().map(|_| "***");
        f.debug_struct("PushConfig")
            .field("prefix", &self.prefix)
            .field("legacy_prefix", &self.legacy_prefix)
            .field("credentials", &self.credentials)
            .field("service_account", &service_account)
            .field("retry_interval", &self.retry_interval)
            .field("max_retries", &self.max_retries)
            .field("history_size", &self.history_size)
            .field("history_days", &self.history_days)
            .finish()
    }
}

impl PushConfig {
    /// The service settings (everything but the credentials).
    pub fn settings(&self) -> nidoo_push::PushSettings {
        nidoo_push::PushSettings {
            prefix: self.prefix.clone(),
            legacy_prefix: self.legacy_prefix.clone(),
            retry_interval: self.retry_interval,
            max_retries: self.max_retries,
            history_size: self.history_size,
            history_days: self.history_days,
        }
    }
}

/// Frigate, for the alert snapshots and clips the browser cannot fetch itself: with `url` set,
/// media on that origin is served through the hub, which logs in with `user` / `password`
/// (Frigate's login port, 8971) or fetches anonymously when they are unset.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FrigateConfig {
    pub url: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    /// How long a connection to Frigate may take before the media request fails.
    pub connect_timeout_ms: u64,
    /// The base URL the phone uses to fetch media in the alerts the bridge raises; `url` when
    /// unset.
    pub media_url: Option<String>,
    /// Turns Frigate events (`<topic_prefix>/events`) into push alerts. Needs a URL.
    pub bridge: bool,
    /// Labels to alert on; empty means all.
    pub labels: Vec<String>,
    /// Cameras to alert on; empty means all.
    pub cameras: Vec<String>,
    /// Alert only once the object enters one of these zones; empty means any.
    pub zones: Vec<String>,
    /// Seconds to ignore new events on a camera after an alert.
    pub cooldown: u64,
    /// The v1 `severity` of the alerts.
    pub severity: String,
    /// The v1 `ttl` of the alerts, in seconds.
    pub ttl: u32,
    /// Frigate's MQTT `topic_prefix`.
    pub topic_prefix: String,
    /// Serves live camera frames through the hub (`/api/v1/cameras/{id}/…`). Off by default:
    /// anyone on the LAN can read them once it is on.
    pub live: bool,
}

impl Default for FrigateConfig {
    fn default() -> Self {
        Self {
            url: None,
            user: None,
            password: None,
            connect_timeout_ms: 5000,
            media_url: None,
            bridge: false,
            labels: Vec::new(),
            cameras: Vec::new(),
            zones: Vec::new(),
            cooldown: 0,
            severity: "warning".to_owned(),
            ttl: 43200,
            topic_prefix: "frigate".to_owned(),
            live: false,
        }
    }
}

impl FrigateConfig {
    /// The bridge's settings, or `None` when it is off or no URL is set.
    pub fn bridge_settings(&self) -> Option<nidoo_frigate::FrigateSettings> {
        let present = |s: &&String| !s.trim().is_empty();
        let url = self
            .media_url
            .as_ref()
            .filter(present)
            .or(self.url.as_ref().filter(present))?;
        if !self.bridge {
            return None;
        }
        let set = |list: &[String]| {
            list.iter()
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
                .collect()
        };
        let mut settings = nidoo_frigate::FrigateSettings::new(url.trim());
        settings.labels = set(&self.labels);
        settings.cameras = set(&self.cameras);
        settings.zones = set(&self.zones);
        settings.cooldown = self.cooldown;
        settings.severity = self.severity.clone();
        settings.ttl = self.ttl;
        Some(settings)
    }

    /// The MQTT topic Frigate publishes its events on.
    pub fn events_topic(&self) -> String {
        format!("{}/events", self.topic_prefix.trim_matches('/'))
    }
}

/// Watchdogs that turn infrastructure trouble into `system` push alerts.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WatchdogConfig {
    pub gateway: GatewayWatchdogConfig,
    pub acl: AclWatchdogConfig,
}

impl WatchdogConfig {
    /// Rejects a watchdog that is on but has nothing to read.
    pub fn validate(&self) -> Result<(), String> {
        let blank = |s: &str| s.trim().is_empty();
        if self.gateway.enabled && blank(self.gateway.prefix.trim_matches('/')) {
            return Err("[watchdog.gateway] is enabled but prefix is empty".to_owned());
        }
        let acl = &self.acl;
        if acl.enabled {
            match acl.source {
                AclSource::File
                    if acl
                        .log_file
                        .as_ref()
                        .is_none_or(|p| p.as_os_str().is_empty()) =>
                {
                    return Err(
                        "[watchdog.acl] source = \"file\" needs log_file (the Mosquitto log path)"
                            .to_owned(),
                    );
                }
                AclSource::Topic if blank(&acl.log_topic) => {
                    return Err("[watchdog.acl] source = \"topic\" needs log_topic".to_owned());
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// The gateway health watchdog: alerts on state changes of `<prefix>/<component>`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GatewayWatchdogConfig {
    pub enabled: bool,
    pub prefix: String,
}

impl Default for GatewayWatchdogConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            prefix: "encryption-gateway/health".to_owned(),
        }
    }
}

/// Where the broker ACL watchdog reads Mosquitto's log lines from.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AclSource {
    /// The `log_topic` (`$SYS/broker/log/#`; needs `log_dest topic` in `mosquitto.conf`).
    #[default]
    Topic,
    /// Tail `log_file`.
    File,
}

/// The broker ACL watchdog: alerts on `Denied PUBLISH/SUBSCRIBE/UNSUBSCRIBE` log lines.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AclWatchdogConfig {
    pub enabled: bool,
    pub source: AclSource,
    pub log_topic: String,
    /// The Mosquitto log to tail when `source = "file"`.
    pub log_file: Option<PathBuf>,
    /// Seconds during which repeats of one denial are held back.
    pub cooldown: u64,
}

impl Default for AclWatchdogConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            source: AclSource::Topic,
            log_topic: "$SYS/broker/log/#".to_owned(),
            log_file: None,
            cooldown: 300,
        }
    }
}

impl fmt::Debug for FrigateConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let password = self.password.as_ref().map(|_| "***");
        f.debug_struct("FrigateConfig")
            .field("url", &self.url)
            .field("user", &self.user)
            .field("password", &password)
            .field("connect_timeout_ms", &self.connect_timeout_ms)
            .field("media_url", &self.media_url)
            .field("bridge", &self.bridge)
            .field("labels", &self.labels)
            .field("cameras", &self.cameras)
            .field("zones", &self.zones)
            .field("cooldown", &self.cooldown)
            .field("severity", &self.severity)
            .field("ttl", &self.ttl)
            .field("topic_prefix", &self.topic_prefix)
            .finish()
    }
}

/// The flow engine's knobs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AutomationConfig {
    /// How many recent runs the hub keeps per flow (newest first); older ones are dropped.
    pub runs_kept: usize,
}

impl Default for AutomationConfig {
    fn default() -> Self {
        Self { runs_kept: 20 }
    }
}

/// Outdoor weather from Open-Meteo. Nothing leaves the LAN while `enabled` is false.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WeatherConfig {
    pub enabled: bool,
    /// The city to resolve; required when enabled.
    pub city: String,
    /// An ISO 3166-1 alpha-2 filter for the geocoding, such as `FR`.
    pub country: Option<String>,
    /// Minutes between two fetches.
    pub refresh_minutes: u64,
    /// The house's coordinates when already known (from the setup); skips the geocoding.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

impl Default for WeatherConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            city: String::new(),
            country: None,
            refresh_minutes: 15,
            latitude: None,
            longitude: None,
        }
    }
}

impl WeatherConfig {
    /// While enabled: a city, a two-letter country when given, and at least 10 minutes between
    /// fetches (Open-Meteo's free tier is for light use).
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        if self.city.trim().is_empty() {
            return Err("[weather] city is required when weather is enabled".to_owned());
        }
        if let Some(country) = &self.country
            && !(country.len() == 2 && country.chars().all(|c| c.is_ascii_alphabetic()))
        {
            return Err(format!(
                "[weather] country `{country}` must be two letters, such as `FR`"
            ));
        }
        if self.refresh_minutes < 10 {
            return Err(format!(
                "[weather] refresh_minutes is {}: it must be at least 10",
                self.refresh_minutes
            ));
        }
        if self.refresh_minutes > 1440 {
            return Err(format!(
                "[weather] refresh_minutes is {}: it must be at most 1440 (one day)",
                self.refresh_minutes
            ));
        }
        Ok(())
    }
}

/// The Solcast rooftop solar forecast. Nothing leaves the LAN while `enabled` is false.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SolcastConfig {
    pub enabled: bool,
    /// Rooftop resource ids from the Solcast Toolkit; one request per site per refresh.
    pub sites: Vec<String>,
    /// This hub's share of the account's daily requests.
    pub daily_limit: u32,
    /// The file holding the API key; `load` defaults it to [`SOLCAST_KEY_FILE`] next to the config.
    pub api_key_file: Option<PathBuf>,
    /// The API key itself (set from the browser); wins over `api_key_file`.
    pub api_key: Option<String>,
}

impl fmt::Debug for SolcastConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let api_key = self.api_key.as_ref().map(|_| "***");
        f.debug_struct("SolcastConfig")
            .field("enabled", &self.enabled)
            .field("sites", &self.sites)
            .field("daily_limit", &self.daily_limit)
            .field("api_key_file", &self.api_key_file)
            .field("api_key", &api_key)
            .finish()
    }
}

impl Default for SolcastConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            sites: Vec::new(),
            daily_limit: 10,
            api_key_file: None,
            api_key: None,
        }
    }
}

/// Reads an API key file: its trimmed content. Errors never include that content.
pub(crate) fn read_key_file(path: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    Ok(text.trim().to_owned())
}

impl SolcastConfig {
    /// While enabled: at least one site, every id non-empty and made of letters, digits, `-` and
    /// `_` (it goes into a URL path), a daily limit covering one call per site, and a readable,
    /// non-empty key file.
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        if self.sites.is_empty() {
            return Err("[solcast] sites needs at least one rooftop resource id".to_owned());
        }
        if self.sites.iter().any(|s| s.trim().is_empty()) {
            return Err("[solcast] sites has an empty id".to_owned());
        }
        if self.sites.iter().any(|s| {
            !s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        }) {
            return Err(
                "[solcast] sites has an id with characters other than letters, digits, - and _"
                    .to_owned(),
            );
        }
        if (self.daily_limit as usize) < self.sites.len() {
            return Err(format!(
                "[solcast] daily_limit is {}: it must be at least the number of sites ({})",
                self.daily_limit,
                self.sites.len()
            ));
        }
        self.read_key().map(|_| ())
    }

    /// The API key: `api_key` when set, otherwise the trimmed content of the key file.
    pub fn read_key(&self) -> Result<String, String> {
        if let Some(key) = self.api_key.as_deref() {
            let key = key.trim();
            if key.is_empty() {
                return Err("[solcast] the API key is empty".to_owned());
            }
            return Ok(key.to_owned());
        }
        let path = self
            .api_key_file
            .as_deref()
            .unwrap_or(Path::new(SOLCAST_KEY_FILE));
        let key = read_key_file(path).map_err(|e| {
            format!(
                "[solcast] cannot read the API key file {}: {e}",
                path.display()
            )
        })?;
        if key.is_empty() {
            return Err(format!(
                "[solcast] the API key file {} is empty",
                path.display()
            ));
        }
        Ok(key)
    }
}

/// Flow scheduling.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FlowsConfig {
    /// IANA time zone the schedules run in (`Europe/Paris`). Absent: the system's zone;
    /// unparseable: UTC, with a warning.
    pub timezone: Option<String>,
    /// House latitude in degrees, north positive. Set together with `longitude`; absent
    /// (both): Paris.
    pub latitude: Option<f64>,
    /// House longitude in degrees, east positive. Set together with `latitude`.
    pub longitude: Option<f64>,
}

impl FlowsConfig {
    /// Rejects a lone coordinate and coordinates outside the globe.
    pub fn validate(&self) -> Result<(), String> {
        match (self.latitude, self.longitude) {
            (None, None) => Ok(()),
            (Some(_), None) => Err(
                "[flows] latitude is set but longitude is missing: set both or neither".to_owned(),
            ),
            (None, Some(_)) => Err(
                "[flows] longitude is set but latitude is missing: set both or neither".to_owned(),
            ),
            (Some(lat), Some(lon)) => {
                if !(-90.0..=90.0).contains(&lat) {
                    return Err(format!("[flows] latitude {lat} is outside -90..=90"));
                }
                if !(-180.0..=180.0).contains(&lon) {
                    return Err(format!("[flows] longitude {lon} is outside -180..=180"));
                }
                Ok(())
            }
        }
    }

    /// The house (latitude, longitude), Paris when neither is configured.
    pub fn house_point(&self) -> (f64, f64) {
        match (self.latitude, self.longitude) {
            (Some(lat), Some(lon)) => (lat, lon),
            _ => (48.8566, 2.3522),
        }
    }

    /// True when the house point is the Paris fallback.
    pub fn fallback_active(&self) -> bool {
        !(self.latitude.is_some() && self.longitude.is_some())
    }
}

/// Hub-level kill switch for group publishing and migration. Off by default.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GroupsConfig {
    pub enabled: bool,
}

/// Hub-level kill switch for network presence watches. On by default.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NetworkConfig {
    pub enabled: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// The climate engine's switch and the topic prefix of its retained documents. Off by default;
/// everything else is edited through MCP and the web Climate screen.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ClimateConfig {
    pub enabled: bool,
    pub prefix: String,
}

impl Default for ClimateConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            prefix: "oikos/climate".to_owned(),
        }
    }
}

impl ClimateConfig {
    /// An enabled engine needs a prefix to keep its documents under: not empty, not only `/`,
    /// and free of the MQTT wildcards `+` and `#`.
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        let prefix = self.prefix.trim();
        if prefix.chars().all(|c| c == '/') {
            return Err("[climate] is enabled but prefix is empty".to_owned());
        }
        if prefix.contains(['+', '#']) {
            return Err("[climate] prefix must not contain the MQTT wildcards + or #".to_owned());
        }
        Ok(())
    }
}

/// The topic prefix of the alarm's retained documents. The alarm itself is switched on in its
/// settings document (web or MCP), so there is no `enabled` here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AlarmConfig {
    pub prefix: String,
}

impl Default for AlarmConfig {
    fn default() -> Self {
        Self {
            prefix: "oikos/alarm".to_owned(),
        }
    }
}

impl AlarmConfig {
    /// The engine always listens on its prefix, so it is always checked: not empty, not only `/`,
    /// and free of the MQTT wildcards `+` and `#`.
    pub fn validate(&self) -> Result<(), String> {
        let prefix = self.prefix.trim();
        if prefix.chars().all(|c| c == '/') {
            return Err("[alarm] prefix is empty".to_owned());
        }
        if prefix.contains(['+', '#']) {
            return Err("[alarm] prefix must not contain the MQTT wildcards + or #".to_owned());
        }
        Ok(())
    }
}

/// TimescaleDB connection and the writer's queue size. An empty `url` turns history off
/// (the house keeps running, the `/api/v1/history/*` routes answer 503).
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HistoryConfig {
    pub url: String,
    pub buffer_rows: usize,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            buffer_rows: 100_000,
        }
    }
}

impl HistoryConfig {
    pub fn enabled(&self) -> bool {
        !self.url.is_empty()
    }
}

/// `url` with its password replaced by `***`, for `Debug` output.
pub(crate) fn mask_url_password(url: &str) -> String {
    if url.is_empty() {
        return String::new();
    }
    match url::Url::parse(url) {
        Ok(mut u) => {
            if u.password().is_some() {
                let _ = u.set_password(Some("***"));
            }
            u.to_string()
        }
        Err(_) => "<unparsable>".to_string(),
    }
}

impl fmt::Debug for HistoryConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let masked = mask_url_password(&self.url);
        f.debug_struct("HistoryConfig")
            .field("url", &masked)
            .field("buffer_rows", &self.buffer_rows)
            .finish()
    }
}

/// Dotted, lowercase keys of string-typed fields. Single source of truth for
/// both the Env filter (figment's `ignore` sees the key after `split`, i.e.
/// `mqtt.password`) and the raw-string merge.
pub const STRING_ENV_KEYS: &[&str] = &[
    "mqtt.host",
    "mqtt.user",
    "mqtt.password",
    "mqtt.client_id",
    "mqtt.ha_status_topic",
    "topics.ui_prefix",
    "topics.z2m_base",
    "topics.automation_prefix",
    "http.bind",
    "log.level",
    "push.prefix",
    "push.legacy_prefix",
    "push.credentials",
    "flows.timezone",
    "frigate.url",
    "frigate.user",
    "frigate.password",
    "frigate.media_url",
    "frigate.severity",
    "frigate.topic_prefix",
    "watchdog.gateway.prefix",
    "watchdog.acl.log_topic",
    "watchdog.acl.log_file",
    "history.url",
    "climate.prefix",
    "alarm.prefix",
    "weather.city",
    "weather.country",
    "solcast.api_key_file",
];

/// The only keys `OIKOS_*` env vars may override once the hub boots from `hub.json`.
pub const BOOTSTRAP_ENV_PREFIXES: &[&str] = &["mqtt.", "history.url", "http.", "log."];

/// The push key `load` falls back to, relative to the config file's folder (git-ignored).
pub const PUSH_KEY_FILE: &str = "secrets/firebase.json";

/// The Solcast key `load` falls back to, relative to the config file's folder (git-ignored).
pub const SOLCAST_KEY_FILE: &str = "secrets/solcast.key";

impl Config {
    /// Runs every section validator, first error wins.
    pub fn validate_all(&self) -> Result<(), String> {
        self.flows.validate()?;
        self.watchdog.validate()?;
        self.climate.validate()?;
        self.alarm.validate()?;
        self.weather.validate()?;
        self.solcast.validate()?;
        Ok(())
    }

    /// The `OIKOS_*` env layer for non-string fields; string fields are
    /// excluded because figment would guess `123456` to be a number.
    pub fn env_provider() -> Env {
        Env::prefixed("OIKOS_").split("__").ignore(STRING_ENV_KEYS)
    }

    /// Loads defaults, merged with the TOML file (`oikos.toml` when `path` is
    /// `None`; a missing file is fine), merged with `OIKOS_` env vars
    /// (`__` separates nesting levels). With no `push.credentials` from either,
    /// [`PUSH_KEY_FILE`] next to the TOML file is used when it exists.
    #[allow(clippy::result_large_err)] // signature fixed by the spec
    pub fn load(path: Option<&Path>) -> Result<Config, figment::Error> {
        let file = path.unwrap_or_else(|| Path::new("oikos.toml"));
        let mut figment = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::file(file))
            .merge(Self::env_provider());
        // String fields keep the raw env text: figment would otherwise guess
        // `123456` or `007` to be numbers.
        for key in STRING_ENV_KEYS {
            let var = format!("OIKOS_{}", key.replace('.', "__").to_uppercase());
            if let Ok(value) = std::env::var(&var) {
                figment = figment.merge(Serialized::default(key, value));
            }
        }
        let mut cfg: Config = figment.extract()?;
        if cfg.push.credentials.is_none() {
            let key = file.parent().unwrap_or(Path::new("")).join(PUSH_KEY_FILE);
            cfg.push.credentials = key.is_file().then_some(key);
        }
        if cfg.solcast.api_key_file.is_none() {
            // Always set, so a missing key's error names the path that was looked at.
            cfg.solcast.api_key_file = Some(
                file.parent()
                    .unwrap_or(Path::new(""))
                    .join(SOLCAST_KEY_FILE),
            );
        }
        Ok(cfg)
    }
}

#[cfg(test)]
mod groups_config_tests {
    use super::*;

    #[test]
    fn groups_default_is_disabled() {
        assert!(!Config::default().groups.enabled);
    }

    #[test]
    fn groups_enabled_true_from_toml() {
        let cfg: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string("[groups]\nenabled = true\n"))
            .extract()
            .unwrap();
        assert!(cfg.groups.enabled);
    }
}

#[cfg(test)]
mod climate_config_tests {
    use super::*;

    #[test]
    fn climate_off_by_default() {
        let cfg = Config::default();
        assert!(!cfg.climate.enabled);
        assert_eq!(cfg.climate.prefix, "oikos/climate");
    }

    #[test]
    fn climate_enabled_from_toml() {
        let cfg: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string("[climate]\nenabled = true\n"))
            .extract()
            .unwrap();
        assert!(cfg.climate.enabled);
        assert_eq!(cfg.climate.prefix, "oikos/climate");
    }

    #[test]
    fn climate_empty_prefix_rejected_when_enabled() {
        let mut cfg = ClimateConfig {
            enabled: true,
            prefix: "  ".to_owned(),
        };
        assert!(cfg.validate().unwrap_err().contains("prefix"));
        cfg.enabled = false;
        assert_eq!(cfg.validate(), Ok(()));
    }

    #[test]
    fn climate_prefix_rejects_slashes_only_and_wildcards() {
        for bad in ["", "/", " // ", "oikos/+/climate", "oikos/#", "a+b"] {
            let cfg = ClimateConfig {
                enabled: true,
                prefix: bad.to_owned(),
            };
            assert!(
                cfg.validate().unwrap_err().contains("prefix"),
                "{bad:?} accepted"
            );
        }
        for good in ["oikos/climate", "climate/", "home"] {
            let cfg = ClimateConfig {
                enabled: true,
                prefix: good.to_owned(),
            };
            assert_eq!(cfg.validate(), Ok(()), "{good:?}");
        }
    }
}

#[cfg(test)]
mod alarm_config_tests {
    use super::*;

    #[test]
    fn alarm_prefix_defaults() {
        let cfg = Config::default();
        assert_eq!(cfg.alarm.prefix, "oikos/alarm");
        assert_eq!(cfg.alarm.validate(), Ok(()));
    }

    #[test]
    fn alarm_prefix_from_toml() {
        let cfg: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string("[alarm]\nprefix = \"home/alarm\"\n"))
            .extract()
            .unwrap();
        assert_eq!(cfg.alarm.prefix, "home/alarm");
    }

    #[test]
    fn alarm_prefix_rejects_slashes_only_and_wildcards() {
        for bad in ["", "/", " // ", "oikos/+/alarm", "oikos/#", "a+b"] {
            let cfg = AlarmConfig {
                prefix: bad.to_owned(),
            };
            assert!(
                cfg.validate().unwrap_err().contains("prefix"),
                "{bad:?} accepted"
            );
        }
        for good in ["oikos/alarm", "alarm/", "home"] {
            let cfg = AlarmConfig {
                prefix: good.to_owned(),
            };
            assert_eq!(cfg.validate(), Ok(()), "{good:?}");
        }
    }
}

#[cfg(test)]
mod flows_config_tests {
    use super::*;

    #[test]
    fn flows_config_both_coords_required_together() {
        let only_lat: FlowsConfig = toml::from_str("latitude = 45.75").unwrap();
        assert!(only_lat.validate().is_err());
        let only_lon: FlowsConfig = toml::from_str("longitude = 4.85").unwrap();
        assert!(only_lon.validate().is_err());
        let both: FlowsConfig = toml::from_str("latitude = 45.75\nlongitude = 4.85").unwrap();
        assert_eq!(both.validate(), Ok(()));
        let neither = FlowsConfig::default();
        assert_eq!(neither.validate(), Ok(()));
    }

    #[test]
    fn flows_config_coord_ranges_enforced() {
        let bad_lat: FlowsConfig = toml::from_str("latitude = 91.0\nlongitude = 4.85").unwrap();
        assert!(bad_lat.validate().is_err());
        let bad_lon: FlowsConfig = toml::from_str("latitude = 45.75\nlongitude = 181.0").unwrap();
        assert!(bad_lon.validate().is_err());
    }

    #[test]
    fn flows_config_house_point_falls_back_to_paris() {
        let cfg = FlowsConfig::default();
        assert_eq!(cfg.house_point(), (48.8566, 2.3522));
        assert!(cfg.fallback_active());
        let configured: FlowsConfig = toml::from_str("latitude = 45.75\nlongitude = 4.85").unwrap();
        assert_eq!(configured.house_point(), (45.75, 4.85));
        assert!(!configured.fallback_active());
    }
}
