//! The settings the wizard edits, one JSON row per section in the `settings` table, and how
//! they combine with `hub.json` into the [`Config`] the hub runs on.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{
    AlarmConfig, AutomationConfig, ClimateConfig, CommandsConfig, Config, FrigateConfig,
    GroupsConfig, HistoryConfig, SolcastConfig, TopicsConfig, WatchdogConfig, WeatherConfig,
};
use crate::setup::hub_file::HubFile;

/// The row keys, in the order the sections are listed.
pub const SECTION_KEYS: [&str; 6] = [
    "weather", "solcast", "frigate", "push", "climate", "advanced",
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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sections {
    pub weather: WeatherSection,
    pub solcast: SolcastSection,
    pub frigate: FrigateConfig,
    pub push: PushSection,
    pub climate: ClimateConfig,
    pub advanced: AdvancedSection,
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
mod tests {
    use super::*;
    use nidoo_api::Location;
    use serde_json::json;

    fn owned(rows: Vec<(&'static str, Value)>) -> Vec<(String, Value)> {
        rows.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
    }

    fn sample() -> Sections {
        let mut s = Sections::default();
        s.weather.enabled = true;
        s.weather.refresh_minutes = 30;
        s.solcast.enabled = true;
        s.solcast.sites = vec!["site-1".into()];
        s.solcast.api_key = Some("sk-secret".into());
        s.frigate.url = Some("http://192.0.2.5:8971".into());
        s.push.service_account = Some("{\"k\":1}".into());
        s.push.max_retries = 3;
        s.climate.enabled = true;
        s.advanced.history_buffer_rows = 5;
        s.advanced.topics.z2m_base = "z2m-x".into();
        s
    }

    fn location() -> Location {
        Location {
            city: "Lyon".into(),
            country: Some("FR".into()),
            latitude: 45.76,
            longitude: 4.84,
            timezone: "Europe/Paris".into(),
        }
    }

    fn file() -> HubFile {
        HubFile {
            database_url: Some("postgres://u:pw@db.example.com/nidoo".into()),
            ..HubFile::default()
        }
    }

    #[test]
    fn timezone_without_location_reaches_flows_and_location_wins() {
        let mut f = file();
        f.timezone = Some("America/Toronto".into());
        let cfg = assemble(&f, &Sections::default());
        assert_eq!(cfg.flows.timezone.as_deref(), Some("America/Toronto"));
        assert_eq!(cfg.flows.latitude, None);
        f.location = Some(location());
        let cfg = assemble(&f, &Sections::default());
        assert_eq!(cfg.flows.timezone.as_deref(), Some("Europe/Paris"));
    }

    #[test]
    fn rows_round_trip() {
        let s = sample();
        assert_eq!(Sections::from_rows(owned(s.to_rows())), s);
    }

    #[test]
    fn row_keys_match_section_keys() {
        let keys: Vec<_> = Sections::default().to_rows().iter().map(|r| r.0).collect();
        assert_eq!(keys, SECTION_KEYS);
    }

    #[test]
    fn missing_rows_are_defaults() {
        assert_eq!(Sections::from_rows(vec![]), Sections::default());
    }

    #[test]
    fn unknown_row_is_ignored() {
        let rows = vec![("nope".to_owned(), json!({"a": 1}))];
        assert_eq!(Sections::from_rows(rows), Sections::default());
    }

    #[test]
    fn bad_row_falls_back_to_that_section_only() {
        let s = sample();
        let rows = vec![
            ("weather".to_owned(), json!(5)),
            (
                "solcast".to_owned(),
                serde_json::to_value(&s.solcast).unwrap(),
            ),
        ];
        let got = Sections::from_rows(rows);
        assert_eq!(got.weather, WeatherSection::default());
        assert_eq!(got.solcast, s.solcast);
    }

    #[test]
    fn assemble_maps_location_and_bootstrap() {
        let mut f = file();
        f.location = Some(location());
        let cfg = assemble(&f, &Sections::default());
        assert_eq!(cfg.flows.timezone.as_deref(), Some("Europe/Paris"));
        assert_eq!(
            (cfg.flows.latitude, cfg.flows.longitude),
            (Some(45.76), Some(4.84))
        );
        assert_eq!(cfg.weather.city, "Lyon");
        assert_eq!(cfg.weather.country.as_deref(), Some("FR"));
        assert_eq!(
            (cfg.weather.latitude, cfg.weather.longitude),
            (Some(45.76), Some(4.84))
        );
        assert_eq!(cfg.history.url, f.database_url.unwrap());
        assert_eq!(cfg.topics.z2m_base, "zigbee2mqtt");
        assert!(cfg.push.credentials.is_none());
        assert!(cfg.solcast.api_key_file.is_none());
    }

    #[test]
    fn assemble_maps_sections() {
        let s = sample();
        let cfg = assemble(&file(), &s);
        assert!(cfg.weather.enabled);
        assert_eq!(cfg.weather.refresh_minutes, 30);
        assert_eq!(cfg.solcast.api_key.as_deref(), Some("sk-secret"));
        assert_eq!(cfg.push.service_account.as_deref(), Some("{\"k\":1}"));
        assert_eq!(cfg.push.max_retries, 3);
        assert_eq!(cfg.history.buffer_rows, 5);
        assert_eq!(cfg.topics.z2m_base, "z2m-x");
        assert!(cfg.climate.enabled);
        assert_eq!(cfg.frigate, s.frigate);
    }

    #[test]
    fn assemble_z2m_base_from_file_wins() {
        let mut f = file();
        f.z2m_base = Some("z2m".into());
        let cfg = assemble(&f, &sample());
        assert_eq!(cfg.topics.z2m_base, "z2m");
    }

    #[test]
    fn solcast_api_key_beats_the_file() {
        let cfg = SolcastConfig {
            api_key: Some(" k ".into()),
            api_key_file: Some("/nonexistent/solcast.key".into()),
            ..SolcastConfig::default()
        };
        assert_eq!(cfg.read_key(), Ok("k".to_owned()));
    }

    #[test]
    fn debug_hides_service_account_and_api_key() {
        let mut s = sample();
        s.push.service_account = Some("PRIVATE-KEY-JSON".into());
        s.solcast.api_key = Some("SOLCAST-KEY".into());
        s.frigate.password = Some("FRIGATE-PW".into());
        let mut f = file();
        f.location = Some(location());
        let cfg = assemble(&f, &s);
        for text in [format!("{cfg:?}"), format!("{s:?}")] {
            for secret in ["PRIVATE-KEY-JSON", "SOLCAST-KEY", "FRIGATE-PW"] {
                assert!(!text.contains(secret), "{secret} leaked in {text}");
            }
        }
    }
}
