//! Configuration: defaults ← the hub's broker (`<data_dir>/hub.json`) ← TOML
//! file ← `NIDOO_SOUND_*` environment variables (`__` nests, e.g.
//! `NIDOO_SOUND_MQTT__HOST`).
//!
//! With the Docker Compose install the service shares the hub's `data/` folder
//! and the host network, so the broker address and login the hub saved in
//! `hub.json` work here as they are: no `.env` needed.

use std::path::{Path, PathBuf};

use anyhow::Context;
use figment::Figment;
use figment::providers::{Env, Format, Serialized, Toml};
use serde::{Deserialize, Serialize};

/// Read when no `--config` is given (optional).
pub const DEFAULT_CONFIG_FILE: &str = "nidoo-sound.toml";
pub const ENV_PREFIX: &str = "NIDOO_SOUND_";
/// The hub's own settings file (`crates/nidoo/src/setup/hub_file.rs`).
pub const HUB_FILE: &str = "hub.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundConfig {
    pub mqtt: MqttSettings,
    /// TCP port of the raw PCM music input (s16le, 48 kHz, stereo).
    pub input_port: u16,
    /// TCP port where players that connect to us (e.g. a browser player)
    /// open `ws://<host>:<port>/sendspin`; 0 turns this endpoint off.
    pub listen_port: u16,
    /// Holds `sound/identity.key` and `sound/players.json`.
    pub data_dir: PathBuf,
    /// How far ahead of playback chunks are timestamped (raised to cover the
    /// players' own minimum buffer).
    pub lead_ms: u32,
    /// Phone PCM gathered before playback starts, to ride out Wi-Fi stalls
    /// (adds this much delay; 0 = none).
    pub input_buffer_ms: u32,
    /// Silence on a connected input after which playback stops.
    pub idle_stop_ms: u32,
    pub discovery_prefix: String,
    /// The hub's web address, where its speech clips are fetched
    /// (`NIDOO_SOUND_HUB_URL`); by default the one in `hub.json`.
    pub hub_url: Option<String>,
    /// The broker login came from the hub's `hub.json` (for the startup log).
    #[serde(skip)]
    pub broker_from_hub: bool,
    /// The hub's web address derived from `hub.json`'s `http.bind`.
    #[serde(skip)]
    pub hub_http: Option<String>,
}

impl Default for SoundConfig {
    fn default() -> Self {
        Self {
            mqtt: MqttSettings::default(),
            input_port: 4953,
            listen_port: crate::listen::DEFAULT_PORT,
            data_dir: PathBuf::from("./data"),
            lead_ms: 500,
            input_buffer_ms: 250,
            idle_stop_ms: 5000,
            discovery_prefix: "homeassistant".into(),
            hub_url: None,
            broker_from_hub: false,
            hub_http: None,
        }
    }
}

impl SoundConfig {
    /// Loads `path` (must exist) or the optional `nidoo-sound.toml`, then the
    /// environment, over the hub's broker from `<data_dir>/hub.json` when the
    /// file has one.
    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        if let Some(p) = path {
            anyhow::ensure!(p.exists(), "config file {} not found", p.display());
        }
        let own = |base: Figment| -> anyhow::Result<SoundConfig> {
            let file = match path {
                Some(p) => Toml::file_exact(p),
                None => Toml::file(DEFAULT_CONFIG_FILE),
            };
            base.merge(file)
                .merge(Env::prefixed(ENV_PREFIX).split("__"))
                .extract()
                .context("reading the configuration")
        };
        let defaults = || Figment::from(Serialized::defaults(SoundConfig::default()));
        // data_dir itself may come from the file or the environment.
        let first = own(defaults())?;
        let hub = read_hub_file(&first.data_dir.join(HUB_FILE)).unwrap_or_default();
        let mut config = match &hub.mqtt {
            None => first,
            Some(broker) => {
                let mut config = own(defaults().merge(Serialized::default("mqtt", broker)))?;
                config.broker_from_hub =
                    config.mqtt.user == broker.user && config.mqtt.password == broker.password;
                config
            }
        };
        config.hub_http = hub.http.map(|h| http_url(&h.bind));
        Ok(config)
    }

    /// Where the hub answers: `hub_url`, else the address in `hub.json`, else
    /// the hub's default port on this machine.
    pub fn hub_base_url(&self) -> String {
        self.hub_url
            .clone()
            .or_else(|| self.hub_http.clone())
            .unwrap_or_else(|| "http://127.0.0.1:8081".into())
    }
}

/// `http://host:port` for a bind address; an "any address" bind is reached
/// on this machine.
fn http_url(bind: &str) -> String {
    let (host, port) = bind.rsplit_once(':').unwrap_or((bind, "8081"));
    let host = match host {
        "0.0.0.0" | "[::]" | "" => "127.0.0.1",
        other => other,
    };
    format!("http://{host}:{port}")
}

/// The broker part of the hub's `hub.json`. Its client id is left out on
/// purpose: two clients with one id would keep kicking each other off.
#[derive(Debug, Serialize, Deserialize)]
struct HubBroker {
    host: String,
    port: u16,
    user: Option<String>,
    password: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HubHttp {
    bind: String,
}

/// The parts of the hub's `hub.json` we use.
#[derive(Debug, Default, Deserialize)]
struct HubFile {
    #[serde(default)]
    mqtt: Option<HubBroker>,
    #[serde(default)]
    http: Option<HubHttp>,
}

/// The hub's settings file, or `None` when it is missing or unreadable. Its
/// broker is absent while the hub is still in its setup wizard.
fn read_hub_file(path: &Path) -> Option<HubFile> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<HubFile>(&text) {
        Ok(file) => Some(file),
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "ignoring the hub's settings file");
            None
        }
    }
}

/// Broker connection, same style as the hub's `[mqtt]` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MqttSettings {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub password: Option<String>,
    pub client_id: String,
}

impl Default for MqttSettings {
    fn default() -> Self {
        Self {
            host: "localhost".into(),
            port: 1883,
            user: None,
            password: None,
            client_id: "nidoo-sound".into(),
        }
    }
}

#[cfg(test)]
// figment::Jail closures return figment::Error, which clippy finds large.
#[allow(clippy::result_large_err)]
mod tests {
    use super::*;

    #[test]
    fn defaults_without_file() {
        figment::Jail::expect_with(|_jail| {
            let c = SoundConfig::load(None).unwrap();
            assert_eq!(c.input_port, 4953);
            assert_eq!(c.listen_port, 8927);
            assert_eq!(c.input_buffer_ms, 250);
            assert_eq!(c.data_dir, PathBuf::from("./data"));
            assert_eq!((c.lead_ms, c.idle_stop_ms), (500, 5000));
            assert_eq!(c.discovery_prefix, "homeassistant");
            assert_eq!(c.mqtt, MqttSettings::default());
            Ok(())
        });
    }

    #[test]
    fn env_overrides_file() {
        figment::Jail::expect_with(|jail| {
            jail.create_file("nidoo-sound.toml", "input_port = 5000\nlead_ms = 700\n")?;
            jail.set_env("NIDOO_SOUND_INPUT_PORT", 6000);
            let c = SoundConfig::load(None).unwrap();
            assert_eq!((c.input_port, c.lead_ms), (6000, 700));
            Ok(())
        });
    }

    #[test]
    fn mqtt_nested_env() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "custom.toml",
                "[mqtt]\nhost = \"file-host\"\nuser = \"hub\"\n",
            )?;
            jail.set_env("NIDOO_SOUND_MQTT__HOST", "broker");
            let c = SoundConfig::load(Some(Path::new("custom.toml"))).unwrap();
            assert_eq!(c.mqtt.host, "broker");
            assert_eq!(c.mqtt.user.as_deref(), Some("hub"));
            assert_eq!(c.mqtt.port, 1883);
            Ok(())
        });
    }

    #[test]
    fn listen_port_zero_turns_the_endpoint_off() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("NIDOO_SOUND_LISTEN_PORT", 0);
            assert_eq!(SoundConfig::load(None).unwrap().listen_port, 0);
            Ok(())
        });
    }

    const HUB_JSON: &str = r#"{"mqtt":{"host":"hub-broker","port":1884,"user":"hub","password":"secret","client_id":"nidoo","send_ha_birth":false,"ha_status_topic":"homeassistant/status"},"database_url":"postgres://x","finished":true}"#;

    #[test]
    fn the_hub_login_is_used_without_any_setting() {
        figment::Jail::expect_with(|jail| {
            jail.create_dir("data")?;
            jail.create_file("data/hub.json", HUB_JSON)?;
            let c = SoundConfig::load(None).unwrap();
            assert_eq!((c.mqtt.host.as_str(), c.mqtt.port), ("hub-broker", 1884));
            assert_eq!(c.mqtt.user.as_deref(), Some("hub"));
            assert_eq!(c.mqtt.password.as_deref(), Some("secret"));
            // Our own client id: two clients with one id kick each other off.
            assert_eq!(c.mqtt.client_id, MqttSettings::default().client_id);
            assert!(c.broker_from_hub);
            Ok(())
        });
    }

    #[test]
    fn env_beats_the_hub_login() {
        figment::Jail::expect_with(|jail| {
            jail.create_dir("data")?;
            jail.create_file("data/hub.json", HUB_JSON)?;
            jail.set_env("NIDOO_SOUND_MQTT__USER", "sound");
            jail.set_env("NIDOO_SOUND_MQTT__PASSWORD", "other");
            let c = SoundConfig::load(None).unwrap();
            assert_eq!(c.mqtt.user.as_deref(), Some("sound"));
            assert_eq!(c.mqtt.password.as_deref(), Some("other"));
            Ok(())
        });
    }

    #[test]
    fn the_hub_file_is_read_from_the_data_dir() {
        figment::Jail::expect_with(|jail| {
            jail.create_dir("elsewhere")?;
            jail.create_file("elsewhere/hub.json", HUB_JSON)?;
            jail.set_env("NIDOO_SOUND_DATA_DIR", "elsewhere");
            assert_eq!(
                SoundConfig::load(None).unwrap().mqtt.user.as_deref(),
                Some("hub")
            );
            Ok(())
        });
    }

    #[test]
    fn a_hub_file_without_a_broker_or_unreadable_is_ignored() {
        figment::Jail::expect_with(|jail| {
            jail.create_dir("data")?;
            jail.create_file("data/hub.json", r#"{"finished":false}"#)?;
            let c = SoundConfig::load(None).unwrap();
            assert_eq!(
                (c.mqtt.clone(), c.broker_from_hub),
                (MqttSettings::default(), false)
            );
            jail.create_file("data/hub.json", "{\"mqtt\":")?;
            let c = SoundConfig::load(None).unwrap();
            assert_eq!(
                (c.mqtt, c.broker_from_hub),
                (MqttSettings::default(), false)
            );
            Ok(())
        });
    }

    #[test]
    fn hub_url_comes_from_hub_json() {
        figment::Jail::expect_with(|jail| {
            jail.create_dir("data")?;
            jail.create_file("data/hub.json", r#"{"http":{"bind":"0.0.0.0:9090"}}"#)?;
            assert_eq!(
                SoundConfig::load(None).unwrap().hub_base_url(),
                "http://127.0.0.1:9090"
            );
            jail.create_file("data/hub.json", r#"{"http":{"bind":"192.0.2.5:8081"}}"#)?;
            assert_eq!(
                SoundConfig::load(None).unwrap().hub_base_url(),
                "http://192.0.2.5:8081"
            );
            Ok(())
        });
    }

    #[test]
    fn hub_url_env_wins() {
        figment::Jail::expect_with(|jail| {
            jail.create_dir("data")?;
            jail.create_file("data/hub.json", r#"{"http":{"bind":"0.0.0.0:9090"}}"#)?;
            jail.set_env("NIDOO_SOUND_HUB_URL", "http://hub:1");
            assert_eq!(
                SoundConfig::load(None).unwrap().hub_base_url(),
                "http://hub:1"
            );
            Ok(())
        });
    }

    #[test]
    fn hub_url_default() {
        figment::Jail::expect_with(|_jail| {
            assert_eq!(
                SoundConfig::load(None).unwrap().hub_base_url(),
                "http://127.0.0.1:8081"
            );
            Ok(())
        });
    }

    #[test]
    fn named_file_must_exist() {
        figment::Jail::expect_with(|_jail| {
            assert!(SoundConfig::load(Some(Path::new("missing.toml"))).is_err());
            Ok(())
        });
    }
}
