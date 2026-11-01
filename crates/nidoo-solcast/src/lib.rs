//! Solcast rooftop-site forecasts: parsing, day figures, the call budget and a small client.

#![forbid(unsafe_code)]

pub mod budget;
pub mod client;
pub mod forecast;
pub mod period;

pub use budget::{Decision, Ledger, RESERVE, decide, refreshes_per_day, slots};
pub use client::{ApiKey, BASE_URL, Client, SolcastError};
pub use forecast::{
    DayTotal, Forecast, best_hours, day_totals, local_midnight, merge, peak, power_at, remaining,
};
pub use period::{ParseError, Period, parse_forecasts};
