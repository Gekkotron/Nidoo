//! Wire types for the onboarding wizard: where the house is and how far setup got.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

/// The house's town, geocoded once at setup and stored as plain numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Location {
    pub city: String,
    pub country: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    /// IANA timezone name.
    pub timezone: String,
}

/// The wizard step the browser resumes at.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SetupStep {
    #[default]
    Welcome,
    Mqtt,
    Database,
    Devices,
    Services,
    Done,
}

/// What a read says about a secret: only whether one is stored, never its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretView {
    pub set: bool,
}

/// A secret in a `PUT` or a test: left out (or an empty string) keeps the stored one, `null`
/// clears it, any other string sets it.
#[derive(Clone, Default, PartialEq, Eq)]
pub enum Secret {
    #[default]
    Keep,
    Clear,
    Set(String),
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Secret::Keep => f.write_str("Keep"),
            Secret::Clear => f.write_str("Clear"),
            Secret::Set(_) => f.write_str("Set(***)"),
        }
    }
}

/// Absent fields are `Keep` through `#[serde(default)]` on the field; `null` is `Clear`; a
/// string is `Set`, except `""`, which is `Keep`: a form posting its empty password field
/// never clears the stored secret by accident. The `{"set": bool}` object a read returns is
/// `Keep`, so a read can be edited and sent back as it is.
impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Secret, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Text(String),
            View {
                #[allow(dead_code)]
                set: bool,
            },
        }
        Ok(match Option::<Raw>::deserialize(d)? {
            None => Secret::Clear,
            Some(Raw::Text(s)) if s.is_empty() => Secret::Keep,
            Some(Raw::Text(s)) => Secret::Set(s),
            Some(Raw::View { .. }) => Secret::Keep,
        })
    }
}

/// The outcome of a connection test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TestResult {
    pub ok: bool,
    pub message: String,
    pub detail: Option<serde_json::Value>,
}

/// A geocoding candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GeoPlace {
    pub name: String,
    pub admin: Option<String>,
    pub country: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MqttInput {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub password: Secret,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub z2m_base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MqttView {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub password: SecretView,
    pub client_id: Option<String>,
    pub z2m_base: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseInput {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    #[serde(default)]
    pub password: Secret,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DatabaseView {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: SecretView,
}

/// The Solcast section as a read shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SolcastView {
    pub enabled: bool,
    pub sites: Vec<String>,
    pub daily_limit: u32,
    pub api_key: SecretView,
}

/// The Solcast section in a `PUT` or a test. A `PUT` replaces the whole section: a left-out
/// field takes its default (a secret keeps, clears or sets as `Secret` says).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SolcastInput {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub sites: Vec<String>,
    #[serde(default)]
    pub daily_limit: Option<u32>,
    #[serde(default)]
    pub api_key: Secret,
}

/// The Frigate section as a read shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrigateView {
    pub url: Option<String>,
    pub user: Option<String>,
    pub password: SecretView,
    pub media_url: Option<String>,
    pub bridge: bool,
    pub labels: Vec<String>,
    pub cameras: Vec<String>,
    pub zones: Vec<String>,
    pub cooldown: u64,
    pub severity: String,
    pub ttl: u32,
    pub topic_prefix: String,
    pub live: bool,
}

/// The Frigate section in a `PUT` or a test. A `PUT` replaces the whole section: a left-out
/// field takes its default (unset, false, empty, or the default cooldown, severity, ttl and
/// topic prefix). Only the password follows `Secret`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FrigateInput {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub password: Secret,
    #[serde(default)]
    pub media_url: Option<String>,
    #[serde(default)]
    pub bridge: bool,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub cameras: Vec<String>,
    #[serde(default)]
    pub zones: Vec<String>,
    #[serde(default)]
    pub cooldown: Option<u64>,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub ttl: Option<u32>,
    #[serde(default)]
    pub topic_prefix: Option<String>,
    #[serde(default)]
    pub live: bool,
}

/// The push section as a read shows it: the service-account key itself is never returned,
/// only the project id it names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PushView {
    pub service_account: SecretView,
    pub project_id: Option<String>,
    pub prefix: String,
    pub legacy_prefix: String,
    pub retry_interval: u64,
    pub max_retries: u32,
    pub history_size: usize,
    pub history_days: u64,
}

/// The push section in a `PUT`. A `PUT` replaces the whole section: a left-out field takes its
/// default. Only the service-account key follows `Secret`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PushInput {
    #[serde(default)]
    pub service_account: Secret,
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub legacy_prefix: Option<String>,
    #[serde(default)]
    pub retry_interval: Option<u64>,
    #[serde(default)]
    pub max_retries: Option<u32>,
    #[serde(default)]
    pub history_size: Option<usize>,
    #[serde(default)]
    pub history_days: Option<u64>,
}

/// Where the setup stands, as `GET /api/v1/setup` reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SetupStatus {
    /// `"setup"` or `"running"`.
    pub mode: String,
    pub step: SetupStep,
    pub finished: bool,
    /// Whether `bundled.json` exists (the compose file's own broker and database).
    pub bundled: bool,
    pub location: Option<Location>,
    pub mqtt: Option<MqttView>,
    pub database: Option<DatabaseView>,
    pub restarting: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn secret_in_a_body() {
        let absent: MqttInput = serde_json::from_value(json!({"host":"h","port":1})).unwrap();
        assert_eq!(absent.password, Secret::Keep);
        let null: MqttInput =
            serde_json::from_value(json!({"host":"h","port":1,"password":null})).unwrap();
        assert_eq!(null.password, Secret::Clear);
        let set: MqttInput =
            serde_json::from_value(json!({"host":"h","port":1,"password":"pw-1"})).unwrap();
        assert_eq!(set.password, Secret::Set("pw-1".into()));
        assert!(!format!("{set:?}").contains("pw-1"));
        let view: MqttInput =
            serde_json::from_value(json!({"host":"h","port":1,"password":{"set":true}})).unwrap();
        assert_eq!(view.password, Secret::Keep);
    }

    #[test]
    fn an_empty_string_keeps_the_secret() {
        // A form posting its empty password field must never clear the stored one.
        let empty: MqttInput =
            serde_json::from_value(json!({"host":"h","port":1,"password":""})).unwrap();
        assert_eq!(empty.password, Secret::Keep);
        let spaces: MqttInput =
            serde_json::from_value(json!({"host":"h","port":1,"password":" "})).unwrap();
        assert_eq!(spaces.password, Secret::Set(" ".into()));
    }
}
