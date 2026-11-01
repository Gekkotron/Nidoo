//! `hub.json`: the few values the hub needs before it can reach its database.

use std::fmt;
use std::path::{Path, PathBuf};

use figment::Figment;
use figment::providers::Serialized;
use nidoo_api::{Location, SetupStep};
use serde::{Deserialize, Serialize};

use crate::config::{
    BOOTSTRAP_ENV_PREFIXES, ENV_PREFIX, HttpConfig, LEGACY_ENV_PREFIX, LogConfig, MqttConfig,
    mask_url_password, merge_env,
};

pub const HUB_FILE: &str = "hub.json";

#[derive(Debug, thiserror::Error)]
pub enum HubFileError {
    #[error("cannot read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{} is not valid: {source}", path.display())]
    Invalid {
        path: PathBuf,
        source: serde_json::Error,
    },
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HubFile {
    pub mqtt: Option<MqttConfig>,
    pub z2m_base: Option<String>,
    pub database_url: Option<String>,
    pub location: Option<Location>,
    /// A city (and country) to geocode into `location`; set only by the import.
    pub pending_city: Option<(String, Option<String>)>,
    /// An imported `[flows] timezone`, kept while `location` is `None`.
    pub timezone: Option<String>,
    pub http: HttpConfig,
    pub log: LogConfig,
    pub step: SetupStep,
    pub finished: bool,
    pub settings_imported: bool,
}

impl fmt::Debug for HubFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let url = self.database_url.as_deref().map(mask_url_password);
        f.debug_struct("HubFile")
            .field("mqtt", &self.mqtt)
            .field("z2m_base", &self.z2m_base)
            .field("database_url", &url)
            .field("location", &self.location)
            .field("pending_city", &self.pending_city)
            .field("timezone", &self.timezone)
            .field("http", &self.http)
            .field("log", &self.log)
            .field("step", &self.step)
            .field("finished", &self.finished)
            .field("settings_imported", &self.settings_imported)
            .finish()
    }
}

/// Just the keys env vars may override, laid out like `Config` so figment paths match.
#[derive(Serialize, Deserialize)]
struct BootstrapView {
    mqtt: MqttConfig,
    history: UrlOnly,
    http: HttpConfig,
    log: LogConfig,
}

#[derive(Serialize, Deserialize)]
struct UrlOnly {
    url: String,
}

fn is_bootstrap_key(key: &str) -> bool {
    // figment hands the filter the split but not yet lowercased key (`MQTT.PORT`).
    let key = key.to_lowercase();
    BOOTSTRAP_ENV_PREFIXES.iter().any(|p| key.starts_with(p))
}

impl HubFile {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join(HUB_FILE)
    }

    /// `Ok(None)` when there is no file; an error naming the file when it can't be read or parsed.
    pub fn load(dir: &Path) -> Result<Option<HubFile>, HubFileError> {
        let path = Self::path(dir);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(HubFileError::Read { path, source }),
        };
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|source| HubFileError::Invalid { path, source })
    }

    /// Atomic (`*.tmp` then rename), mode 0600, creating `dir`.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        crate::setup::fs::write_private_atomic(&Self::path(dir), &json)
    }

    pub fn is_complete(&self) -> bool {
        self.mqtt.is_some() && self.database_url.is_some()
    }

    /// Applies the env overrides (`OIKOS_*`, then `NIDOO_*` over them) to the bootstrap keys
    /// only. `mqtt` is filled only when at least one `*_MQTT__*` variable is set.
    #[allow(clippy::result_large_err)]
    pub fn with_bootstrap_env(mut self) -> Result<HubFile, figment::Error> {
        let mqtt_env = std::env::vars().any(|(k, _)| {
            let k = k.to_uppercase();
            [LEGACY_ENV_PREFIX, ENV_PREFIX]
                .iter()
                .any(|p| k.starts_with(&format!("{p}MQTT__")))
        });
        let view = BootstrapView {
            mqtt: self.mqtt.clone().unwrap_or_default(),
            history: UrlOnly {
                url: self.database_url.clone().unwrap_or_default(),
            },
            http: self.http.clone(),
            log: self.log.clone(),
        };
        let figment = merge_env(Figment::from(Serialized::defaults(view)), is_bootstrap_key);
        let view: BootstrapView = figment.extract()?;
        if mqtt_env {
            self.mqtt = Some(view.mqtt);
        }
        if !view.history.url.is_empty() {
            self.database_url = Some(view.history.url);
        }
        self.http = view.http;
        self.log = view.log;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::result_large_err)] // figment::Jail closures return figment::Error

    use super::*;

    fn full() -> HubFile {
        HubFile {
            mqtt: Some(MqttConfig {
                host: "192.0.2.1".into(),
                user: Some("u".into()),
                password: Some("s3cret".into()),
                ..MqttConfig::default()
            }),
            z2m_base: Some("zigbee2mqtt".into()),
            database_url: Some("postgres://u:dbpass@h/db".into()),
            location: Some(Location {
                city: "Paris".into(),
                country: Some("France".into()),
                latitude: 48.85,
                longitude: 2.35,
                timezone: "Europe/Paris".into(),
            }),
            pending_city: Some(("Lyon".into(), None)),
            step: SetupStep::Devices,
            finished: true,
            settings_imported: true,
            ..HubFile::default()
        }
    }

    #[test]
    fn missing_file_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(HubFile::load(tmp.path()).unwrap().is_none());
    }

    #[test]
    fn save_then_load_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("nested");
        let f = full();
        f.save(&dir).unwrap();
        assert_eq!(HubFile::load(&dir).unwrap().unwrap(), f);
        assert!(!dir.join("hub.json.tmp").exists());
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        full().save(tmp.path()).unwrap();
        let meta = std::fs::metadata(tmp.path().join("hub.json")).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn invalid_json_is_an_error_naming_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("hub.json"), "{\"mqtt\":").unwrap();
        let err = HubFile::load(tmp.path()).unwrap_err().to_string();
        assert!(err.contains("hub.json"), "{err}");
    }

    #[test]
    fn complete_needs_mqtt_and_database() {
        let mut f = HubFile {
            mqtt: Some(MqttConfig::default()),
            ..HubFile::default()
        };
        assert!(!f.is_complete());
        f.database_url = Some("postgres://h/db".into());
        assert!(f.is_complete());
        f.mqtt = None;
        assert!(!f.is_complete());
    }

    #[test]
    fn debug_hides_passwords() {
        let s = format!("{:?}", full());
        assert!(!s.contains("s3cret") && !s.contains("dbpass"), "{s}");
    }

    #[test]
    fn env_overrides_bootstrap_keys_only() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("OIKOS_MQTT__HOST", "192.0.2.5");
            jail.set_env("OIKOS_MQTT__PASSWORD", "123456");
            jail.set_env("OIKOS_MQTT__PORT", "1884");
            jail.set_env("OIKOS_HISTORY__URL", "postgres://u:p@192.0.2.6/oikos");
            jail.set_env("OIKOS_WEATHER__ENABLED", "true");
            let f = HubFile::default().with_bootstrap_env().unwrap();
            let mqtt = f.mqtt.unwrap();
            assert_eq!(mqtt.host, "192.0.2.5");
            assert_eq!(mqtt.port, 1884);
            assert_eq!(mqtt.password.as_deref(), Some("123456"));
            assert_eq!(
                f.database_url.as_deref(),
                Some("postgres://u:p@192.0.2.6/oikos")
            );
            Ok(())
        });
    }

    #[test]
    fn no_mqtt_env_leaves_mqtt_unset() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("OIKOS_LOG__LEVEL", "debug");
            let f = HubFile::default().with_bootstrap_env().unwrap();
            assert!(f.mqtt.is_none());
            assert_eq!(f.log.level, "debug");
            Ok(())
        });
    }
}
