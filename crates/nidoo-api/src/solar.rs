//! The solar forecast as the hub serves it: `GET /api/v1/solar`. Energies are kWh, powers kW,
//! times Unix seconds and dates `YYYY-MM-DD` in the house's local time zone.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The whole solar forecast: the figures, the refresh state and the budget.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SolarView {
    /// When the forecast was last fetched successfully.
    pub fetched_at: u64,
    /// The forecast is overdue: refreshes keep failing or the last success is old.
    pub stale: bool,
    /// When the next call is planned; `None` when none is (budget spent, key refused).
    pub next_refresh_at: Option<u64>,
    /// Requests made so far in the current UTC day.
    pub calls_today: u32,
    /// The hub's daily request budget.
    pub daily_limit: u32,
    pub now: SolarNow,
    pub today: SolarToday,
    /// The next days, starting tomorrow (today is in `today`).
    pub days: Vec<SolarDay>,
    /// Today's share per configured site.
    pub sites: Vec<SolarSite>,
    /// The merged forecast periods, for charts.
    pub periods: Vec<SolarPeriod>,
    /// Past forecasts, to compare with what was produced.
    #[serde(default)]
    pub history: SolarHistory,
}

/// The forecast for this moment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SolarNow {
    /// Expected power now, kW.
    pub power: Option<f64>,
    /// Expected energy for the rest of today, kWh.
    pub remaining_today: Option<f64>,
}

/// Today's figures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SolarToday {
    pub date: String,
    /// Expected energy for the whole day, kWh.
    pub estimate: Option<f64>,
    /// The pessimistic (10th percentile) energy, kWh.
    pub low: Option<f64>,
    /// The optimistic (90th percentile) energy, kWh.
    pub high: Option<f64>,
    pub peak: Option<SolarPeak>,
    /// The span of hours around the peak that holds most of the production.
    pub best_hours: Option<SolarSpan>,
}

/// The highest expected power of a day.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SolarPeak {
    /// kW.
    pub power: f64,
    /// When, Unix seconds.
    pub at: u64,
}

/// A span of time, Unix seconds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SolarSpan {
    pub from: u64,
    pub to: u64,
}

/// One forecast day, kWh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SolarDay {
    pub date: String,
    pub estimate: f64,
    pub low: f64,
    pub high: f64,
}

/// One configured site's share of today.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SolarSite {
    /// The site's position in `[solcast] sites`; the resource id is never served.
    pub index: u32,
    /// Expected energy today, kWh.
    pub today: Option<f64>,
}

/// One forecast period; the values are mean kW over the period.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SolarPeriod {
    /// When the period ends, Unix seconds.
    pub end: u64,
    /// The period's length.
    pub minutes: u32,
    pub estimate: f64,
    pub low: f64,
    pub high: f64,
}

/// Past day forecasts.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SolarHistory {
    pub forecasts: Vec<SolarForecastDay>,
}

/// What was forecast for a day, and when.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SolarForecastDay {
    pub date: String,
    pub estimate: f64,
    pub low: f64,
    pub high: f64,
    /// When this forecast was made, Unix seconds.
    pub made_at: u64,
}
