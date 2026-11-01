//! The outdoor weather as the hub serves it: `GET /api/v1/weather`. Units are metric.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A coarse sky condition, grouped from the WMO weather codes Open-Meteo reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WeatherCondition {
    Clear,
    PartlyCloudy,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    FreezingRain,
    Snow,
    Showers,
    Thunderstorm,
}

impl WeatherCondition {
    /// Every condition, in declaration order.
    pub const ALL: [WeatherCondition; 10] = [
        WeatherCondition::Clear,
        WeatherCondition::PartlyCloudy,
        WeatherCondition::Cloudy,
        WeatherCondition::Fog,
        WeatherCondition::Drizzle,
        WeatherCondition::Rain,
        WeatherCondition::FreezingRain,
        WeatherCondition::Snow,
        WeatherCondition::Showers,
        WeatherCondition::Thunderstorm,
    ];

    /// The condition for a WMO code, or `None` for a code the table does not know.
    pub fn try_from_wmo(code: u16) -> Option<WeatherCondition> {
        use WeatherCondition::*;
        Some(match code {
            0 => Clear,
            1 | 2 => PartlyCloudy,
            3 => Cloudy,
            45 | 48 => Fog,
            51 | 53 | 55 => Drizzle,
            56 | 57 | 66 | 67 => FreezingRain,
            61 | 63 | 65 => Rain,
            71 | 73 | 75 | 77 | 85 | 86 => Snow,
            80..=82 => Showers,
            95 | 96 | 99 => Thunderstorm,
            _ => return None,
        })
    }

    /// Like [`try_from_wmo`](Self::try_from_wmo), with `Cloudy` for an unknown code.
    pub fn from_wmo(code: u16) -> WeatherCondition {
        Self::try_from_wmo(code).unwrap_or(WeatherCondition::Cloudy)
    }

    /// The snake_case name, as serialized.
    pub fn as_str(self) -> &'static str {
        match self {
            WeatherCondition::Clear => "clear",
            WeatherCondition::PartlyCloudy => "partly_cloudy",
            WeatherCondition::Cloudy => "cloudy",
            WeatherCondition::Fog => "fog",
            WeatherCondition::Drizzle => "drizzle",
            WeatherCondition::Rain => "rain",
            WeatherCondition::FreezingRain => "freezing_rain",
            WeatherCondition::Snow => "snow",
            WeatherCondition::Showers => "showers",
            WeatherCondition::Thunderstorm => "thunderstorm",
        }
    }
}

/// The place the weather is for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WeatherPlace {
    pub name: String,
    pub admin: Option<String>,
    pub country: Option<String>,
}

/// Conditions right now. A value Open-Meteo did not report is `None`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WeatherCurrent {
    /// °C.
    pub temperature: Option<f64>,
    /// °C.
    pub apparent_temperature: Option<f64>,
    /// %.
    pub humidity: Option<f64>,
    /// %.
    pub cloud_cover: Option<f64>,
    /// mm.
    pub precipitation: Option<f64>,
    /// km/h.
    pub wind_speed: Option<f64>,
    /// km/h.
    pub wind_gusts: Option<f64>,
    /// Degrees the wind comes from.
    pub wind_direction: Option<f64>,
    /// hPa.
    pub pressure: Option<f64>,
    pub uv_index: Option<f64>,
    pub condition: Option<WeatherCondition>,
    /// Whether the sun is up.
    pub daylight: Option<bool>,
}

/// Today's totals and extremes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WeatherToday {
    /// °C.
    pub min: Option<f64>,
    /// °C.
    pub max: Option<f64>,
    /// mm.
    pub precipitation: Option<f64>,
    /// %.
    pub precipitation_probability: Option<f64>,
    /// km/h.
    pub wind_gusts_max: Option<f64>,
}

/// One day of the forecast.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WeatherDay {
    /// `YYYY-MM-DD`, in the place's local time.
    pub date: String,
    pub condition: Option<WeatherCondition>,
    /// °C.
    pub min: Option<f64>,
    /// °C.
    pub max: Option<f64>,
    /// %.
    pub precipitation_probability: Option<f64>,
    /// °C, the highest "feels like" temperature.
    pub apparent_max: Option<f64>,
    /// mm, the day's total.
    pub precipitation: Option<f64>,
    /// km/h, the strongest sustained wind.
    pub wind_speed_max: Option<f64>,
    /// km/h, the strongest gust.
    pub wind_gusts_max: Option<f64>,
    /// Degrees the wind mostly comes from over the day.
    pub wind_direction: Option<f64>,
    /// %, the mean of the day's hours.
    pub humidity: Option<f64>,
    /// hPa, the mean of the day's hours.
    pub pressure: Option<f64>,
    /// %, the mean of the day's hours.
    pub cloud_cover: Option<f64>,
    /// The highest UV index of the day.
    pub uv_index_max: Option<f64>,
}

/// The whole weather document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WeatherView {
    pub place: WeatherPlace,
    /// Unix seconds of the last successful fetch.
    pub fetched_at: u64,
    /// The data is older than it should be (the last refreshes failed).
    pub stale: bool,
    pub current: WeatherCurrent,
    pub today: WeatherToday,
    /// mm expected in the next hour.
    pub rain_next_hour: Option<f64>,
    /// °C, the lowest expected tonight.
    pub tonight_min: Option<f64>,
    pub days: Vec<WeatherDay>,
}

/// The weather device's name, `[en, fr]`. The hub publishes the English one.
pub const WEATHER_DEVICE_NAME: [&str; 2] = ["Weather", "Météo"];

/// The name of the `weather_<key>` entity, `[en, fr]`, or `None` for a key the hub does not
/// publish. The hub publishes the English one; a client shows its language's while the entity
/// still carries that default.
pub fn weather_entity_name(key: &str) -> Option<[&'static str; 2]> {
    Some(match key {
        "temperature" => ["Outdoor temperature", "Température extérieure"],
        "apparent_temperature" => ["Feels like", "Ressenti"],
        "humidity" => ["Outdoor humidity", "Humidité extérieure"],
        "condition" => ["Condition", "Temps"],
        "daylight" => ["Daylight", "Jour"],
        "cloud_cover" => ["Cloud cover", "Couverture nuageuse"],
        "precipitation" => ["Precipitation", "Précipitations"],
        "wind_speed" => ["Wind speed", "Vitesse du vent"],
        "wind_gusts" => ["Wind gusts", "Rafales"],
        "wind_direction" => ["Wind direction", "Direction du vent"],
        "pressure" => ["Pressure", "Pression"],
        "uv_index" => ["UV index", "Indice UV"],
        "today_min" => ["Today's minimum", "Minimum du jour"],
        "today_max" => ["Today's maximum", "Maximum du jour"],
        "today_precipitation" => ["Today's precipitation", "Précipitations du jour"],
        "today_precipitation_probability" => [
            "Today's precipitation probability",
            "Probabilité de pluie du jour",
        ],
        "today_wind_gusts_max" => ["Today's strongest gust", "Rafale max du jour"],
        "rain_next_hour" => ["Rain next hour", "Pluie dans l'heure"],
        "tonight_min" => ["Tonight's minimum", "Minimum cette nuit"],
        "tomorrow_condition" => ["Tomorrow's condition", "Temps demain"],
        "tomorrow_min" => ["Tomorrow's minimum", "Minimum demain"],
        "tomorrow_max" => ["Tomorrow's maximum", "Maximum demain"],
        "tomorrow_precipitation" => ["Tomorrow's precipitation", "Précipitations demain"],
        "tomorrow_precipitation_probability" => [
            "Tomorrow's precipitation probability",
            "Probabilité de pluie demain",
        ],
        "tomorrow_wind_gusts_max" => ["Tomorrow's strongest gust", "Rafale max demain"],
        "tomorrow_uv_index_max" => ["Tomorrow's UV index", "Indice UV demain"],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wmo_codes_map_to_conditions() {
        use WeatherCondition::*;
        let cases = [
            (0, Clear),
            (1, PartlyCloudy),
            (2, PartlyCloudy),
            (3, Cloudy),
            (45, Fog),
            (48, Fog),
            (51, Drizzle),
            (55, Drizzle),
            (56, FreezingRain),
            (57, FreezingRain),
            (66, FreezingRain),
            (67, FreezingRain),
            (61, Rain),
            (65, Rain),
            (71, Snow),
            (77, Snow),
            (80, Showers),
            (82, Showers),
            (85, Snow),
            (86, Snow),
            (95, Thunderstorm),
            (99, Thunderstorm),
            (42, Cloudy),
        ];
        for (code, want) in cases {
            assert_eq!(WeatherCondition::from_wmo(code), want, "code {code}");
        }
        assert_eq!(WeatherCondition::try_from_wmo(42), None);
    }

    #[test]
    fn condition_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(WeatherCondition::PartlyCloudy).unwrap(),
            json!("partly_cloudy")
        );
        assert_eq!(WeatherCondition::FreezingRain.as_str(), "freezing_rain");
        for c in WeatherCondition::ALL {
            assert_eq!(serde_json::to_value(c).unwrap(), json!(c.as_str()));
        }
    }
}
