//! Climate additions to the history database: the decision table the climate engine
//! writes, and the reads it needs (applied commands, target history, outdoor mean).

use chrono::{DateTime, Utc};
use sqlx::Row;

use nidoo_core::history::Micros;

use super::db::{Db, dt_to_micros, micros_to_dt};

/// One device's share of a changed decision. A decision with no devices is stored as one
/// row with `entity_id = ""` and `mode = ""`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClimateDecisionRow {
    pub at: Micros,
    pub zone: String,
    pub entity_id: String,
    pub mode: String,
    pub setpoint: Option<f64>,
    pub source: String,
    pub reason: String,
    pub applied: bool,
    /// The correction applied to the command: it was `setpoint + adjust`. `None` when none.
    pub adjust: Option<f64>,
}

fn decision_from_row(row: &sqlx::postgres::PgRow) -> ClimateDecisionRow {
    let t: DateTime<Utc> = row.get(0);
    ClimateDecisionRow {
        at: dt_to_micros(t),
        zone: row.get(1),
        entity_id: row.get(2),
        mode: row.get(3),
        setpoint: row.get(4),
        source: row.get(5),
        reason: row.get(6),
        applied: row.get(7),
        adjust: row.get(8),
    }
}

impl Db {
    /// Inserts decision rows in one statement, preserving slice order in `seq`.
    pub async fn insert_climate_decisions(
        &self,
        rows: &[ClimateDecisionRow],
    ) -> Result<(), sqlx::Error> {
        if rows.is_empty() {
            return Ok(());
        }
        let times: Vec<DateTime<Utc>> = rows.iter().map(|r| micros_to_dt(r.at)).collect();
        let zones: Vec<String> = rows.iter().map(|r| r.zone.clone()).collect();
        let entities: Vec<String> = rows.iter().map(|r| r.entity_id.clone()).collect();
        let modes: Vec<String> = rows.iter().map(|r| r.mode.clone()).collect();
        let setpoints: Vec<Option<f64>> = rows.iter().map(|r| r.setpoint).collect();
        let sources: Vec<String> = rows.iter().map(|r| r.source.clone()).collect();
        let reasons: Vec<String> = rows.iter().map(|r| r.reason.clone()).collect();
        let applied: Vec<bool> = rows.iter().map(|r| r.applied).collect();
        let adjusts: Vec<Option<f64>> = rows.iter().map(|r| r.adjust).collect();
        sqlx::query(
            "INSERT INTO climate_decision
               (time, zone, entity_id, mode, setpoint, source, reason, applied, adjust)
             SELECT * FROM UNNEST(
                 $1::timestamptz[], $2::text[], $3::text[], $4::text[],
                 $5::float8[], $6::text[], $7::text[], $8::bool[], $9::float8[]
             ) AS t(time, zone, entity_id, mode, setpoint, source, reason, applied, adjust)",
        )
        .bind(&times)
        .bind(&zones)
        .bind(&entities)
        .bind(&modes)
        .bind(&setpoints)
        .bind(&sources)
        .bind(&reasons)
        .bind(&applied)
        .bind(&adjusts)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The latest `limit` decision rows of a zone, newest first; with `before`, only rows
    /// strictly earlier than it.
    pub async fn climate_decisions(
        &self,
        zone: &str,
        before: Option<Micros>,
        limit: i64,
    ) -> Result<Vec<ClimateDecisionRow>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT time, zone, entity_id, mode, setpoint, source, reason, applied, adjust
             FROM climate_decision
             WHERE zone = $1 AND ($3::timestamptz IS NULL OR time < $3)
             ORDER BY time DESC, seq DESC
             LIMIT $2",
        )
        .bind(zone)
        .bind(limit)
        .bind(before.map(micros_to_dt))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(decision_from_row).collect())
    }

    /// Applied decision rows for `entities` with `time` in `[from, to)`, oldest first.
    pub async fn climate_commands(
        &self,
        entities: &[String],
        from: Micros,
        to: Micros,
    ) -> Result<Vec<ClimateDecisionRow>, sqlx::Error> {
        if entities.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT time, zone, entity_id, mode, setpoint, source, reason, applied, adjust
             FROM climate_decision
             WHERE applied AND entity_id = ANY($1::text[]) AND time >= $2 AND time < $3
             ORDER BY time, seq",
        )
        .bind(entities)
        .bind(micros_to_dt(from))
        .bind(micros_to_dt(to))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(decision_from_row).collect())
    }

    /// Target and mode history of a climate entity over `[from, to)`, oldest first. The
    /// last row before `from` is included with its time clamped to `from`, so the window
    /// starts with the value in force.
    pub async fn climate_targets(
        &self,
        entity: &str,
        from: Micros,
        to: Micros,
    ) -> Result<Vec<(Micros, Option<f64>, Option<String>)>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT GREATEST(time, $2::timestamptz) AS t, target, mode FROM (
               (SELECT time, seq,
                       (value->'value'->>'target')::float8 AS target,
                       value->'value'->>'mode' AS mode
                FROM entity_state
                WHERE entity_id = $1 AND value->>'type' = 'climate'
                  AND time < $2::timestamptz
                ORDER BY time DESC, seq DESC
                LIMIT 1)
               UNION ALL
               (SELECT time, seq,
                       (value->'value'->>'target')::float8 AS target,
                       value->'value'->>'mode' AS mode
                FROM entity_state
                WHERE entity_id = $1 AND value->>'type' = 'climate'
                  AND time >= $2::timestamptz AND time < $3::timestamptz)
             ) r
             ORDER BY time, seq",
        )
        .bind(entity)
        .bind(micros_to_dt(from))
        .bind(micros_to_dt(to))
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let t: DateTime<Utc> = row.get(0);
            out.push((dt_to_micros(t), row.get(1), row.get(2)));
        }
        Ok(out)
    }

    /// Mean of the numeric samples of `entity` in `[from, to)`; `None` when there are none.
    pub async fn mean_since(
        &self,
        entity: &str,
        from: Micros,
        to: Micros,
    ) -> Result<Option<f64>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT AVG(num) FROM entity_state
             WHERE entity_id = $1 AND num IS NOT NULL AND time >= $2 AND time < $3",
        )
        .bind(entity)
        .bind(micros_to_dt(from))
        .bind(micros_to_dt(to))
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get(0))
    }
}
