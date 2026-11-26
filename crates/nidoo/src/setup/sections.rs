//! The settings the wizard edits, one JSON row per section in the `settings` table, and how
//! they combine with `hub.json` into the [`Config`] the hub runs on.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::backup::settings::BackupSection;
use crate::config::{
    AlarmConfig, AutomationConfig, ClimateConfig, CommandsConfig, Config, FrigateConfig,
    GroupsConfig, HistoryConfig, SolcastConfig, SystemConfig, TopicsConfig, WatchdogConfig,
    WeatherConfig,
};
use crate::setup::hub_file::HubFile;
use crate::setup::mobile::MobileSection;

/// The row keys, in the order the sections are listed.
pub const SECTION_KEYS: [&str; 9] = [
    "weather", "solcast", "frigate", "push", "climate", "advanced", "system", "mobile", "backup",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WeatherSection {
    pub enabled: bool,
    pub refresh_minutes: u64,
    pub notify_unreachable: bool,
}

impl Default for WeatherSection {
    fn default() -> Self {
        Self {
            enabled: false,
            refresh_minutes: 15,
            notify_unreachable: false,
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SolcastSection {
    pub enabled: bool,
    pub sites: Vec<String>,
    pub daily_limit: u32,
    pub api_key: Option<String>,
    pub notify_unreachable: bool,
}

impl Default for SolcastSection {
    fn default() -> Self {
        let d = SolcastConfig::default();
        Self {
            enabled: d.enabled,
            sites: d.sites,
            daily_limit: d.daily_limit,
            api_key: None,
            notify_unreachable: d.notify_unreachable,
        }
    }
}

impl fmt::Debug for SolcastSection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let api_key = self.api_key.as_ref().map(|_| "***");
        f.debug_struct("SolcastSection")
            .field("enabled", &self.enabled)
            .field("sites", &self.sites)
            .field("daily_limit", &self.daily_limit)
            .field("api_key", &api_key)
            .field("notify_unreachable", &self.notify_unreachable)
            .finish()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PushSection {
    pub service_account: Option<String>,
    pub prefix: String,
    pub retry_interval: u64,
    pub max_retries: u32,
    pub history_size: usize,
    pub history_days: u64,
}

impl Default for PushSection {
    fn default() -> Self {
        let d = crate::config::PushConfig::default();
        Self {
            service_account: None,
            prefix: d.prefix,
            retry_interval: d.retry_interval,
            max_retries: d.max_retries,
            history_size: d.history_size,
            history_days: d.history_days,
        }
    }
}

impl fmt::Debug for PushSection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let service_account = self.service_account.as_ref().map(|_| "***");
        f.debug_struct("PushSection")
            .field("service_account", &service_account)
            .field("prefix", &self.prefix)
            .field("retry_interval", &self.retry_interval)
            .field("max_retries", &self.max_retries)
            .field("history_size", &self.history_size)
            .field("history_days", &self.history_days)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AdvancedSection {
    pub topics: TopicsConfig,
    pub commands: CommandsConfig,
    pub automation: AutomationConfig,
    pub watchdog: WatchdogConfig,
    pub groups: GroupsConfig,
    pub alarm: AlarmConfig,
    pub history_buffer_rows: usize,
}

impl Default for AdvancedSection {
    fn default() -> Self {
        Self {
            topics: TopicsConfig::default(),
            commands: CommandsConfig::default(),
            automation: AutomationConfig::default(),
            watchdog: WatchdogConfig::default(),
            groups: GroupsConfig::default(),
            alarm: AlarmConfig::default(),
            history_buffer_rows: HistoryConfig::default().buffer_rows,
        }
    }
}

/// Stored without a restart (see `Hub::set_system_notify`), so it is not in the generic
/// settings routes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemSection {
    pub notify: bool,
    /// The sensor supervisor, saved live like `notify` (see `Hub::set_supervisor`).
    pub supervisor: nidoo_api::SupervisorSettings,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sections {
    pub weather: WeatherSection,
    pub solcast: SolcastSection,
    pub frigate: FrigateConfig,
    pub push: PushSection,
    pub climate: ClimateConfig,
    pub advanced: AdvancedSection,
    pub system: SystemSection,
    pub mobile: MobileSection,
    pub backup: BackupSection,
}

impl Sections {
    /// One `(key, json)` row per section, keys from [`SECTION_KEYS`].
    pub fn to_rows(&self) -> Vec<(&'static str, Value)> {
        fn row<T: Serialize>(v: &T) -> Value {
            serde_json::to_value(v).expect("a section serializes")
        }
        vec![
            ("weather", row(&self.weather)),
            ("solcast", row(&self.solcast)),
            ("frigate", row(&self.frigate)),
            ("push", row(&self.push)),
            ("climate", row(&self.climate)),
            ("advanced", row(&self.advanced)),
            ("system", row(&self.system)),
            ("mobile", row(&self.mobile)),
            ("backup", row(&self.backup)),
        ]
    }

    /// Rebuilds the sections from stored rows. An unknown key is ignored; a row that does not
    /// parse falls back to that section's default, with a warning naming the key.
    pub fn from_rows(rows: impl IntoIterator<Item = (String, Value)>) -> Sections {
        fn parse<T: serde::de::DeserializeOwned + Default>(key: &str, value: Value) -> T {
            match serde_json::from_value(value) {
                Ok(v) => v,
                Err(error) => {
                    tracing::warn!(key, %error, "settings row is not valid; using the defaults for it");
                    T::default()
                }
            }
        }
        let mut out = Sections::default();
        for (key, value) in rows {
            match key.as_str() {
                "weather" => out.weather = parse(&key, value),
                "solcast" => out.solcast = parse(&key, value),
                "frigate" => out.frigate = parse(&key, value),
                "push" => out.push = parse(&key, value),
                "climate" => out.climate = parse(&key, value),
                "advanced" => out.advanced = parse(&key, value),
                "system" => out.system = parse(&key, value),
                "mobile" => out.mobile = parse(&key, value),
                "backup" => out.backup = parse(&key, value),
                _ => {}
            }
        }
        out
    }
}

/// The hub's [`Config`]: bootstrap values from `file`, everything else from `s`. The key files
/// (`credentials`, `api_key_file`) stay unset: secrets live in the settings.
pub fn assemble(file: &HubFile, s: &Sections) -> Config {
    let mut topics = s.advanced.topics.clone();
    if let Some(base) = &file.z2m_base {
        topics.z2m_base = base.clone();
    }
    let mut cfg = Config {
        mqtt: file.mqtt.clone().unwrap_or_default(),
        topics,
        http: file.http.clone(),
        commands: s.advanced.commands.clone(),
        log: file.log.clone(),
        push: crate::config::PushConfig {
            prefix: s.push.prefix.clone(),
            credentials: None,
            service_account: s.push.service_account.clone(),
            retry_interval: s.push.retry_interval,
            max_retries: s.push.max_retries,
            history_size: s.push.history_size,
            history_days: s.push.history_days,
        },
        frigate: s.frigate.clone(),
        watchdog: s.advanced.watchdog.clone(),
        automation: s.advanced.automation.clone(),
        groups: s.advanced.groups.clone(),
        climate: s.climate.clone(),
        alarm: s.advanced.alarm.clone(),
        history: HistoryConfig {
            url: file.database_url.clone().unwrap_or_default(),
            buffer_rows: s.advanced.history_buffer_rows,
        },
        weather: WeatherConfig {
            enabled: s.weather.enabled,
            refresh_minutes: s.weather.refresh_minutes,
            notify_unreachable: s.weather.notify_unreachable,
            ..WeatherConfig::default()
        },
        solcast: SolcastConfig {
            enabled: s.solcast.enabled,
            sites: s.solcast.sites.clone(),
            daily_limit: s.solcast.daily_limit,
            api_key_file: None,
            api_key: s.solcast.api_key.clone(),
            notify_unreachable: s.solcast.notify_unreachable,
        },
        system: SystemConfig {
            notify: s.system.notify,
            supervisor: s.system.supervisor.clone(),
        },
        ..Config::default()
    };
    cfg.flows.timezone = file.timezone.clone();
    if let Some(loc) = &file.location {
        cfg.flows.timezone = Some(loc.timezone.clone());
        cfg.flows.latitude = Some(loc.latitude);
        cfg.flows.longitude = Some(loc.longitude);
        cfg.weather.city = loc.city.clone();
        cfg.weather.country = loc.country.clone();
        cfg.weather.latitude = Some(loc.latitude);
        cfg.weather.longitude = Some(loc.longitude);
    } else if let Some((city, country)) = &file.pending_city {
        // An imported city still waiting for its lookup: the weather poller geocodes it
        // itself, as the hub did before the import.
        cfg.weather.city = city.clone();
        cfg.weather.country = country.clone();
    }
    cfg
}

#[cfg(test)]
mod tests;
