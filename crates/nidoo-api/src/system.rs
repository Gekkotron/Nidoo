//! System health: the hub's own figures, restart history and service states.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The system health page; times are Unix seconds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SystemView {
    /// When this hub process started.
    pub started_at: u64,
    /// Share of the last 30 days the hub was up, 0 to 100.
    pub uptime_pct: f64,
    /// Recent starts, with how long the hub was down before each.
    pub restarts: Vec<RestartView>,
    /// Processor load; absent when it cannot be measured.
    pub cpu: Option<CpuView>,
    /// Memory use; absent when it cannot be measured.
    pub ram: Option<MemView>,
    /// Disk use; absent when it cannot be measured.
    pub disk: Option<DiskView>,
    /// Machine temperature in degrees Celsius, when a sensor exists.
    pub temperature_c: Option<f64>,
    /// How many devices the hub knows.
    pub devices: u32,
    /// How many entities the hub knows.
    pub entities: u32,
    /// How many retained messages the broker holds, when known.
    pub retained: Option<u64>,
    /// Commands sent today.
    pub commands_today: u64,
    /// The services the hub depends on.
    pub services: Vec<ServiceView>,
    /// Whether a service outage sends a push.
    pub notify: bool,
}

/// One start of the hub.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RestartView {
    /// When the hub started.
    pub started_at: u64,
    /// Seconds the hub was down before this start; unknown when absent.
    pub down_secs: Option<u64>,
    /// How the previous run ended.
    pub stop: StopKind,
}

/// How a run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StopKind {
    /// No earlier run is known.
    First,
    /// The hub was stopped on purpose.
    Clean,
    /// The hub went down without saying so.
    Unexpected,
}

/// Processor load in percent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CpuView {
    /// The whole machine.
    pub server_pct: f64,
    /// The hub process.
    pub hub_pct: f64,
}

/// Memory use in bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemView {
    pub used_bytes: u64,
    pub total_bytes: u64,
    /// The hub process.
    pub hub_bytes: u64,
}

/// Disk use in bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiskView {
    pub used_bytes: u64,
    pub total_bytes: u64,
    /// The database size, when known.
    pub db_bytes: Option<u64>,
}

/// A service's state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceView {
    pub service: SystemService,
    pub state: ServiceState,
    /// When the state last changed.
    pub since: Option<u64>,
}

/// A service the hub depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SystemService {
    Broker,
    Zigbee2mqtt,
    Database,
}

/// Whether a service answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    Up,
    Down,
    /// Not measurable right now.
    Unknown,
}

/// The body that switches outage pushes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NotifyInput {
    pub notify: bool,
}

/// The ids of the system sensors.
pub mod ids {
    pub const CPU: &str = "system_cpu";
    pub const CPU_HUB: &str = "system_cpu_hub";
    pub const RAM_USED: &str = "system_ram_used";
    pub const RAM_HUB: &str = "system_ram_hub";
    pub const DISK_USED: &str = "system_disk_used";
    pub const DB_SIZE: &str = "system_db_size";
    pub const TEMPERATURE: &str = "system_temperature";
    pub const DEVICES: &str = "system_devices";
    pub const ENTITIES: &str = "system_entities";
    pub const RETAINED: &str = "system_retained";
    pub const COMMANDS: &str = "system_commands";
    pub const STARTED: &str = "system_started";
    pub const BROKER: &str = "system_broker";
    pub const ZIGBEE2MQTT: &str = "system_zigbee2mqtt";
    pub const DATABASE: &str = "system_database";
    /// When the last successful backup ended.
    pub const LAST_BACKUP: &str = "system_last_backup";
    /// Every system entity id.
    pub const ALL: [&str; 16] = [
        CPU,
        CPU_HUB,
        RAM_USED,
        RAM_HUB,
        DISK_USED,
        DB_SIZE,
        TEMPERATURE,
        DEVICES,
        ENTITIES,
        RETAINED,
        COMMANDS,
        STARTED,
        BROKER,
        ZIGBEE2MQTT,
        DATABASE,
        LAST_BACKUP,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn system_view_wire_shape() {
        let view = SystemView {
            started_at: 1,
            uptime_pct: 99.5,
            restarts: vec![RestartView {
                started_at: 10,
                down_secs: None,
                stop: StopKind::Unexpected,
            }],
            cpu: None,
            ram: None,
            disk: None,
            temperature_c: None,
            devices: 3,
            entities: 9,
            retained: None,
            commands_today: 7,
            services: vec![ServiceView {
                service: SystemService::Zigbee2mqtt,
                state: ServiceState::Down,
                since: Some(5),
            }],
            notify: false,
        };
        let value = serde_json::to_value(&view).unwrap();
        assert_eq!(value["startedAt"], 1);
        assert_eq!(value["uptimePct"], 99.5);
        assert_eq!(value["commandsToday"], 7);
        assert!(value["restarts"][0]["downSecs"].is_null());
        assert_eq!(value["restarts"][0]["stop"], "unexpected");
        assert_eq!(value["services"][0]["service"], "zigbee2mqtt");
        assert_eq!(value["services"][0]["state"], "down");
        let back: SystemView = serde_json::from_value(value).unwrap();
        assert_eq!(back, view);
    }

    #[test]
    fn ids_all_are_distinct_and_prefixed() {
        let mut seen = std::collections::HashSet::new();
        for id in ids::ALL {
            assert!(id.starts_with("system_"), "{id}");
            assert!(seen.insert(id), "{id} repeated");
        }
        let bad = serde_json::from_value::<NotifyInput>(json!({"notify": true, "x": 1}));
        assert!(bad.is_err());
        let ok: NotifyInput = serde_json::from_value(json!({"notify": true})).unwrap();
        assert!(ok.notify);
    }
}
