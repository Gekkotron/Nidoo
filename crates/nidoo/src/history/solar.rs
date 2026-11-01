//! Solar additions to the history database: the forecast as it stood at each local day's
//! first successful refresh, kept to compare with what the panels produced.

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::Row;

use super::db::Db;

/// One local day's morning forecast, kWh.
#[derive(Clone, Debug, PartialEq)]
pub struct SolarForecastRow {
    pub date: NaiveDate,
    pub estimate: f64,
    pub low: f64,
    pub high: f64,
    /// When the forecast was made.
    pub made_at: DateTime<Utc>,
}

impl Db {
    /// Stores a day's forecast unless the day already has one. True when the row was inserted.
    pub async fn insert_solar_forecast_day(
        &self,
        row: &SolarForecastRow,
    ) -> Result<bool, sqlx::Error> {
        let done = sqlx::query(
            "INSERT INTO solar_forecast_days (date, estimate_kwh, low_kwh, high_kwh, made_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (date) DO NOTHING",
        )
        .bind(row.date)
        .bind(row.estimate)
        .bind(row.low)
        .bind(row.high)
        .bind(row.made_at)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// The stored forecasts with `from <= date < to`, oldest first.
    pub async fn solar_forecast_days(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<SolarForecastRow>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT date, estimate_kwh, low_kwh, high_kwh, made_at
             FROM solar_forecast_days
             WHERE date >= $1 AND date < $2
             ORDER BY date",
        )
        .bind(from)
        .bind(to)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| SolarForecastRow {
                date: r.get(0),
                estimate: r.get(1),
                low: r.get(2),
                high: r.get(3),
                made_at: r.get(4),
            })
            .collect())
    }
}
