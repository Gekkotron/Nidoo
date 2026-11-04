//! Configuration: defaults ← TOML file ← `NIDOO_SOUND_*` environment
//! variables (`__` nests, e.g. `NIDOO_SOUND_MQTT__HOST`).

use std::path::{Path, PathBuf};

use anyhow::Context;
use figment::Figment;
use figment::providers::{Env, Format, Serialized, Toml};
use serde::{Deserialize, Serialize};

/// Read when no `--config` is given (optional).
pub const DEFAULT_CONFIG_FILE: &str = "nidoo-sound.toml";
pub const ENV_PREFIX: &str = "NIDOO_SOUND_";

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
    /// Silence on a connected input after which playback stops.
    pub idle_stop_ms: u32,
    pub discovery_prefix: String,
}

impl Default for SoundConfig {
    fn default() -> Self {
        Self {
            mqtt: MqttSettings::default(),
            input_port: 4953,
            listen_port: crate::listen::DEFAULT_PORT,
            data_dir: PathBuf::from("./data"),
            lead_ms: 500,
            idle_stop_ms: 5000,
            discovery_prefix: "homeassistant".into(),
        }
    }
}

impl SoundConfig {
    /// Loads `path` (must exist) or the optional `nidoo-sound.toml`, then the environment.
    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        let mut figment = Figment::from(Serialized::defaults(SoundConfig::default()));
        figment = match path {
            Some(p) => {
                anyhow::ensure!(p.exists(), "config file {} not found", p.display());
                figment.merge(Toml::file_exact(p))
            }
            None => figment.merge(Toml::file(DEFAULT_CONFIG_FILE)),
        };
        figment
            .merge(Env::prefixed(ENV_PREFIX).split("__"))
            .extract()
            .context("reading the configuration")
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

    #[test]
    fn named_file_must_exist() {
        figment::Jail::expect_with(|_jail| {
            assert!(SoundConfig::load(Some(Path::new("missing.toml"))).is_err());
            Ok(())
        });
    }
}
