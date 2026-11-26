//! `hub.json`: the few values the hub needs before it can reach its database.

use std::fmt;
use std::path::{Path, PathBuf};

use figment::Figment;
use figment::providers::Serialized;
use nidoo_api::{Location, SetupStep};
use serde::{Deserialize, Serialize};

use crate::config::{
    BOOTSTRAP_ENV_PREFIXES, ENV_PREFIX, HttpConfig, LogConfig, MqttConfig, mask_url_password,
    merge_env,
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
    /// A city (and country) to geocode into `location`; set by the setup.
    pub pending_city: Option<(String, Option<String>)>,
    /// A `[flows] timezone`, kept while `location` is `None`.
    pub timezone: Option<String>,
    pub http: HttpConfig,
    pub log: LogConfig,
    pub step: SetupStep,
    pub finished: bool,
    /// Set while the hub is to restore a backup before it runs; absent in files written before
    /// the restore existed.
    pub restore: Option<RestoreState>,
}

/// Where a pending restore stands.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RestoreState {
    /// Whether a restore has been started on this hub.
    pub attempted: bool,
    /// The database and broker the first attempt found empty. Absent in files written before it
    /// was recorded.
    #[serde(default)]
    pub target: Option<RestoreTarget>,
}

impl RestoreState {
    /// Whether a retry may clear what `file` points at (the target after the env overrides):
    /// only the database and broker an attempt checked for data. Anything else, such as a
    /// target an override or a hand edit changed since, is checked again first.
    pub fn retry_clears(&self, file: &HubFile) -> bool {
        self.attempted && self.target.is_some() && self.target == RestoreTarget::of(file)
    }
}

/// The PostgreSQL port a URL without one uses.
const PG_PORT: u16 = 5432;

/// Which database a URL reaches, as which user: decoded (a re-saved URL may encode them
/// differently), the host in lower case. No password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseTarget {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
}

impl DatabaseTarget {
    pub fn parse(url: &str) -> Option<DatabaseTarget> {
        let u = url::Url::parse(url).ok()?;
        Some(DatabaseTarget {
            host: u.host_str()?.to_ascii_lowercase(),
            port: u.port().unwrap_or(PG_PORT),
            database: decode(u.path().trim_start_matches('/')),
            user: decode(u.username()),
        })
    }
}

/// What a restore checked for data: the database and the broker address. No secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreTarget {
    pub database: DatabaseTarget,
    pub broker_host: String,
    pub broker_port: u16,
}

impl RestoreTarget {
    /// The target `file` names; `None` without a usable database URL or a broker.
    pub fn of(file: &HubFile) -> Option<RestoreTarget> {
        let mqtt = file.mqtt.as_ref()?;
        Some(RestoreTarget {
            database: DatabaseTarget::parse(file.database_url.as_deref()?)?,
            broker_host: mqtt.host.trim().to_ascii_lowercase(),
            broker_port: mqtt.port,
        })
    }
}

pub(crate) fn decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .into_owned()
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
            .field("restore", &self.restore)
            .finish()
    }
}

/// The `hub.json` values a backup carries: the house's place and the hub's own HTTP and log
/// settings. What ties the file to one machine (`mqtt`, `database_url`) and the setup's progress
/// stay out.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HubFields {
    pub location: Option<Location>,
    pub timezone: Option<String>,
    pub http: HttpConfig,
    pub log: LogConfig,
    pub z2m_base: Option<String>,
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

    /// The values a backup carries.
    pub fn fields(&self) -> HubFields {
        HubFields {
            location: self.location.clone(),
            timezone: self.timezone.clone(),
            http: self.http.clone(),
            log: self.log.clone(),
            z2m_base: self.z2m_base.clone(),
        }
    }

    /// Takes a backup's values. `mqtt`, `database_url`, `step` and `finished` are left as they
    /// are. A restored `location` also drops a city still waiting to be geocoded, which would
    /// otherwise replace it.
    pub fn apply_fields(&mut self, fields: HubFields) {
        if fields.location.is_some() {
            self.pending_city = None;
        }
        self.location = fields.location;
        self.timezone = fields.timezone;
        self.http = fields.http;
        self.log = fields.log;
        self.z2m_base = fields.z2m_base;
    }

    pub fn is_complete(&self) -> bool {
        self.mqtt.is_some() && self.database_url.is_some()
    }

    /// Applies the env overrides (`NIDOO_*`) to the bootstrap keys
    /// only. `mqtt` is filled only when at least one `*_MQTT__*` variable is set.
    #[allow(clippy::result_large_err)]
    pub fn with_bootstrap_env(mut self) -> Result<HubFile, figment::Error> {
        let mqtt_env = std::env::vars().any(|(k, _)| {
            let k = k.to_uppercase();
            k.starts_with(&format!("{ENV_PREFIX}MQTT__"))
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
mod tests;
