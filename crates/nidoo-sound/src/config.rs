//! Configuration types.

use serde::{Deserialize, Serialize};

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
