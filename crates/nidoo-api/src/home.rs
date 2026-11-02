//! The home settings document: `GET`/`PUT /api/v1/home/settings`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use nidoo_core::energy::{OffPeakDevice, OffPeakEntry, OffPeakWindow};

/// What the home header needs from the user. Every field is optional.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct HomeSettings {
    /// The threshold in °C (5 to 30) at or below which a thermostat, or an off one, counts as eco.
    pub eco_setpoint: Option<f64>,
    /// The entity id of the select that holds the house mode.
    pub mode_entity: Option<String>,
    /// The entity id of the sensor shown as the indoor temperature.
    pub indoor_entity: Option<String>,
    /// °C, below this the house reads cold; unset = 15.
    pub cold_below: Option<f64>,
    /// °C, above this it reads warm; unset = 26.
    pub hot_above: Option<f64>,
    /// Off-peak windows in force from today, `HH:MM` local wall clock; `end < start` crosses
    /// midnight. Empty means a single tariff.
    pub off_peak: Vec<OffPeakWindow>,
    /// The dated windows of past days, kept by the hub: a client's value is ignored.
    pub off_peak_history: Vec<OffPeakEntry>,
    /// The entity whose state says off-peak, and the values that mean it; the windows are the
    /// backup when it has no reading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off_peak_device: Option<OffPeakDevice>,
    /// How the web Energy page draws the live power, the same for every browser.
    pub energy_view: EnergyView,
    /// °C, the lowest setpoint the assistant applies without asking (5 to 35); unset = 16.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assist_min_c: Option<f64>,
    /// °C, the highest setpoint the assistant applies without asking (5 to 35); unset = 23.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assist_max_c: Option<f64>,
}

/// The assistant's range when the home settings leave it unset, in °C.
pub const ASSIST_DEFAULT_MIN_C: f64 = 16.0;
pub const ASSIST_DEFAULT_MAX_C: f64 = 23.0;

impl HomeSettings {
    /// The setpoints the assistant applies without asking: `(min, max)` in °C.
    pub fn assist_range(&self) -> (f64, f64) {
        (
            self.assist_min_c.unwrap_or(ASSIST_DEFAULT_MIN_C),
            self.assist_max_c.unwrap_or(ASSIST_DEFAULT_MAX_C),
        )
    }
}

/// The web Energy page's live diagram.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EnergyView {
    /// The circle diagram.
    #[default]
    Classic,
    /// The Sankey diagram.
    Flux,
    /// The double ring.
    Orbite,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn home_settings_default_to_null_and_accept_partial_json() {
        let s: HomeSettings = serde_json::from_value(json!({"eco_setpoint": 15.0})).unwrap();
        assert_eq!(
            s,
            HomeSettings {
                eco_setpoint: Some(15.0),
                ..HomeSettings::default()
            }
        );
        assert_eq!((s.cold_below, s.hot_above), (None, None));
        assert!(s.off_peak.is_empty() && s.off_peak_history.is_empty());
        assert_eq!(s.energy_view, EnergyView::Classic);
        assert_eq!(
            serde_json::to_value(HomeSettings::default()).unwrap(),
            json!({
                "eco_setpoint": null,
                "mode_entity": null,
                "indoor_entity": null,
                "cold_below": null,
                "hot_above": null,
                "off_peak": [],
                "off_peak_history": [],
                "energy_view": "classic"
            })
        );
    }

    #[test]
    fn energy_view_is_lowercase_and_refuses_unknown_values() {
        let s: HomeSettings = serde_json::from_value(json!({"energy_view": "flux"})).unwrap();
        assert_eq!(s.energy_view, EnergyView::Flux);
        let s: HomeSettings = serde_json::from_value(json!({"energy_view": "orbite"})).unwrap();
        assert_eq!(s.energy_view, EnergyView::Orbite);
        assert_eq!(
            serde_json::to_value(EnergyView::Orbite).unwrap(),
            json!("orbite")
        );
        assert!(serde_json::from_value::<HomeSettings>(json!({"energy_view": "orbit"})).is_err());
    }

    #[test]
    fn off_peak_device_round_trips_and_is_omitted_when_none() {
        let s: HomeSettings = serde_json::from_value(json!({"eco_setpoint": 15.0})).unwrap();
        assert_eq!(s.off_peak_device, None);
        let wire = serde_json::to_value(&s).unwrap();
        assert!(wire.get("off_peak_device").is_none(), "{wire}");
        let device = json!({"entity": "sensor.tariff", "values": ["OFF"]});
        let s: HomeSettings =
            serde_json::from_value(json!({"off_peak_device": device.clone()})).unwrap();
        assert_eq!(
            s.off_peak_device,
            Some(OffPeakDevice {
                entity: "sensor.tariff".to_owned(),
                values: vec!["OFF".to_owned()],
            })
        );
        assert_eq!(serde_json::to_value(&s).unwrap()["off_peak_device"], device);
    }
}
