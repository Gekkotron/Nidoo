//! Postgres (TimescaleDB) layer: lazy pool, embedded migrations, inserts and reads.
//!
//! All I/O lives here. The compute types (`Rows`, `ValueRow`, `LastWritten`, `Bucket`,
//! `Sample`) come from `nidoo_core::history`; this module only converts, binds and runs
//! queries. The pool is created with [`Db::connect_lazy`] so the hub starts even when the
//! database is temporarily down; the first query times out via `acquire_timeout` instead
//! of hanging the writer.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value as Json;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};

use nidoo_core::history::buckets::{Bucket, Sample};
use nidoo_core::history::{AvailabilityRow, LastWritten, Micros, Rows, ValueRow};
use nidoo_core::model::{Availability, StateValue};

/// Handle on the history Postgres pool.
#[derive(Clone)]
pub struct Db {
    pub(super) pool: PgPool,
}

impl Db {
    /// Builds a lazy pool — no connection is opened until the first query, so the hub boots
    /// without a running database.
    pub fn connect_lazy(url: &str) -> Result<Db, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(5))
            .connect_lazy(url)?;
        Ok(Db { pool })
    }

    /// Runs the embedded migrations in `./migrations` at startup.
    /// The underlying pool, for modules outside `history` that keep their own tables.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn migrate(&self) -> Result<(), sqlx::migrate::MigrateError> {
        sqlx::migrate!("./migrations").run(&self.pool).await
    }

    /// Inserts a batch of value and availability rows in one transaction, then upserts the
    /// `*_latest` lookup tables for the rows whose time is newer than what's stored.
    pub async fn insert(&self, rows: &Rows) -> Result<(), sqlx::Error> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;

        if !rows.values.is_empty() {
            insert_values(&mut tx, &rows.values).await?;
            upsert_entity_latest(&mut tx, &rows.values).await?;
        }

        if !rows.availability.is_empty() {
            insert_availability(&mut tx, &rows.availability).await?;
            upsert_availability_latest(&mut tx, &rows.availability).await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Reads the latest value and availability per entity into a seeded [`LastWritten`].
    pub async fn load_latest(&self) -> Result<LastWritten, sqlx::Error> {
        let mut lw = LastWritten::default();

        let rows = sqlx::query("SELECT entity_id, value FROM entity_latest")
            .fetch_all(&self.pool)
            .await?;
        for row in rows {
            let id: String = row.get(0);
            let value: Json = row.get(1);
            if let Ok(sv) = serde_json::from_value::<StateValue>(value) {
                lw.seed_value(id, sv);
            }
        }

        let rows = sqlx::query("SELECT entity_id, availability FROM availability_latest")
            .fetch_all(&self.pool)
            .await?;
        for row in rows {
            let id: String = row.get(0);
            let a: String = row.get(1);
            if let Some(av) = parse_availability(&a) {
                lw.seed_availability(id, av);
            }
        }

        Ok(lw)
    }

    /// The latest recorded value per entity with the time it was recorded; a row whose value
    /// no longer deserializes is skipped.
    pub async fn latest_states(&self) -> Result<Vec<(String, Micros, StateValue)>, sqlx::Error> {
        let rows = sqlx::query("SELECT entity_id, time, value FROM entity_latest")
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.get(0);
            let t: DateTime<Utc> = row.get(1);
            let value: Json = row.get(2);
            if let Ok(sv) = serde_json::from_value::<StateValue>(value) {
                out.push((id, dt_to_micros(t), sv));
            }
        }
        Ok(out)
    }

    /// Raw numeric samples in `[from, to)`, in `(entity_id, time, seq)` order so writes at
    /// the same microsecond keep their insert order.
    pub async fn samples(
        &self,
        from: Micros,
        to: Micros,
        entity: Option<&str>,
    ) -> Result<Vec<(String, Sample)>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT entity_id, time, num FROM entity_state
             WHERE num IS NOT NULL AND time >= $1 AND time < $2
               AND ($3::text IS NULL OR entity_id = $3)
             ORDER BY entity_id, time, seq",
        )
        .bind(micros_to_dt(from))
        .bind(micros_to_dt(to))
        .bind(entity)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.get(0);
            let t: DateTime<Utc> = row.get(1);
            let n: f64 = row.get(2);
            out.push((
                id,
                Sample {
                    at: dt_to_micros(t),
                    num: n,
                },
            ));
        }
        Ok(out)
    }

    /// For each entity, the `last` value of its latest hourly bucket strictly before
    /// `before`.
    pub async fn hourly_carry(
        &self,
        before: Micros,
        entities: &[String],
    ) -> Result<HashMap<String, f64>, sqlx::Error> {
        if entities.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = sqlx::query(
            "SELECT DISTINCT ON (entity_id) entity_id, last
             FROM history_hourly
             WHERE bucket < $1 AND entity_id = ANY($2::text[])
             ORDER BY entity_id, bucket DESC",
        )
        .bind(micros_to_dt(before))
        .bind(entities)
        .fetch_all(&self.pool)
        .await?;
        let mut out = HashMap::new();
        for row in rows {
            let id: String = row.get(0);
            let last: f64 = row.get(1);
            out.insert(id, last);
        }
        Ok(out)
    }

    /// Upserts a batch of hourly buckets, replacing an existing (entity, bucket) row.
    pub async fn upsert_hourly(&self, rows: &[(String, Bucket)]) -> Result<(), sqlx::Error> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        for (id, b) in rows {
            sqlx::query(
                "INSERT INTO history_hourly
                   (entity_id, bucket, covered_us, integral, min, max, last, increase)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT (entity_id, bucket) DO UPDATE SET
                   covered_us = EXCLUDED.covered_us,
                   integral   = EXCLUDED.integral,
                   min        = EXCLUDED.min,
                   max        = EXCLUDED.max,
                   last       = EXCLUDED.last,
                   increase   = EXCLUDED.increase",
            )
            .bind(id)
            .bind(micros_to_dt(b.start))
            .bind(b.covered)
            .bind(b.integral)
            .bind(b.min)
            .bind(b.max)
            .bind(b.last)
            .bind(b.increase)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Stored hourly buckets for `entity` with `bucket` in `[from, to)`, sorted.
    pub async fn hourly(
        &self,
        entity: &str,
        from: Micros,
        to: Micros,
    ) -> Result<Vec<Bucket>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT bucket, covered_us, integral, min, max, last, increase
             FROM history_hourly
             WHERE entity_id = $1 AND bucket >= $2 AND bucket < $3
             ORDER BY bucket",
        )
        .bind(entity)
        .bind(micros_to_dt(from))
        .bind(micros_to_dt(to))
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let bucket: DateTime<Utc> = row.get(0);
            out.push(Bucket {
                start: dt_to_micros(bucket),
                covered: row.get(1),
                integral: row.get(2),
                min: row.get(3),
                max: row.get(4),
                last: row.get(5),
                increase: row.get(6),
            });
        }
        Ok(out)
    }

    /// How far the rollup has filled, as microseconds. `None` on the very first run.
    pub async fn done_until(&self) -> Result<Option<Micros>, sqlx::Error> {
        let row = sqlx::query("SELECT done_until FROM rollup_state WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| dt_to_micros(r.get(0))))
    }

    /// Writes the rollup watermark.
    pub async fn set_done_until(&self, at: Micros) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO rollup_state (id, done_until) VALUES (1, $1)
             ON CONFLICT (id) DO UPDATE SET done_until = EXCLUDED.done_until",
        )
        .bind(micros_to_dt(at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Oldest `entity_state.time`, if any — used to seed the rollup watermark.
    pub async fn first_time(&self) -> Result<Option<Micros>, sqlx::Error> {
        let row = sqlx::query("SELECT MIN(time) AS t FROM entity_state")
            .fetch_one(&self.pool)
            .await?;
        let t: Option<DateTime<Utc>> = row.get(0);
        Ok(t.map(dt_to_micros))
    }

    /// The latest raw value of `entity` strictly before `at`.
    pub async fn last_before(&self, entity: &str, at: Micros) -> Result<Option<f64>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT num FROM entity_state
             WHERE entity_id = $1 AND num IS NOT NULL AND time < $2
             ORDER BY time DESC, seq DESC LIMIT 1",
        )
        .bind(entity)
        .bind(micros_to_dt(at))
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.get(0)))
    }

    /// `entity`'s last value before `from` and its values in `[from, to)`, oldest first, text
    /// included; rows that no longer decode are skipped.
    pub async fn states_between(
        &self,
        entity: &str,
        from: Micros,
        to: Micros,
    ) -> Result<(Option<StateValue>, Vec<(Micros, StateValue)>), sqlx::Error> {
        let carry = sqlx::query(
            "SELECT value FROM entity_state
             WHERE entity_id = $1 AND time < $2
             ORDER BY time DESC, seq DESC LIMIT 1",
        )
        .bind(entity)
        .bind(micros_to_dt(from))
        .fetch_optional(&self.pool)
        .await?
        .and_then(|row| serde_json::from_value::<StateValue>(row.get::<Json, _>(0)).ok());
        let rows = sqlx::query(
            "SELECT time, value FROM entity_state
             WHERE entity_id = $1 AND time >= $2 AND time < $3
             ORDER BY time, seq",
        )
        .bind(entity)
        .bind(micros_to_dt(from))
        .bind(micros_to_dt(to))
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .filter_map(|row| {
            let t: DateTime<Utc> = row.get(0);
            let value = serde_json::from_value::<StateValue>(row.get::<Json, _>(1)).ok()?;
            Some((dt_to_micros(t), value))
        })
        .collect();
        Ok((carry, rows))
    }

    /// `entity`'s last availability before `from` and its changes in `[from, to)`, oldest first.
    pub async fn availability_between(
        &self,
        entity: &str,
        from: Micros,
        to: Micros,
    ) -> Result<(Option<Availability>, Vec<(Micros, Availability)>), sqlx::Error> {
        let carry = sqlx::query(
            "SELECT availability FROM entity_availability
             WHERE entity_id = $1 AND time < $2
             ORDER BY time DESC, seq DESC LIMIT 1",
        )
        .bind(entity)
        .bind(micros_to_dt(from))
        .fetch_optional(&self.pool)
        .await?
        .and_then(|row| parse_availability(&row.get::<String, _>(0)));
        let rows = sqlx::query(
            "SELECT time, availability FROM entity_availability
             WHERE entity_id = $1 AND time >= $2 AND time < $3
             ORDER BY time, seq",
        )
        .bind(entity)
        .bind(micros_to_dt(from))
        .bind(micros_to_dt(to))
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .filter_map(|row| {
            let t: DateTime<Utc> = row.get(0);
            Some((
                dt_to_micros(t),
                parse_availability(&row.get::<String, _>(1))?,
            ))
        })
        .collect();
        Ok((carry, rows))
    }

    /// Oldest value time over `entities`, if any: one indexed `ORDER BY time LIMIT 1` per id,
    /// so the answer never scans an entity's history.
    pub async fn first_time_of(&self, entities: &[String]) -> Result<Option<Micros>, sqlx::Error> {
        let mut first: Option<Micros> = None;
        for id in entities {
            let row = sqlx::query(
                "SELECT time FROM entity_state
                 WHERE entity_id = $1 AND num IS NOT NULL
                 ORDER BY time ASC LIMIT 1",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
            if let Some(row) = row {
                let t = dt_to_micros(row.get::<DateTime<Utc>, _>(0));
                first = Some(first.map_or(t, |f| f.min(t)));
            }
        }
        Ok(first)
    }

    /// Latest value time per requested id, missing ids absent from the map.
    pub async fn last_changes(
        &self,
        ids: &[String],
    ) -> Result<HashMap<String, Micros>, sqlx::Error> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = sqlx::query(
            "SELECT entity_id, time FROM entity_latest WHERE entity_id = ANY($1::text[])",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await?;
        let mut out = HashMap::new();
        for row in rows {
            let id: String = row.get(0);
            let t: DateTime<Utc> = row.get(1);
            out.insert(id, dt_to_micros(t));
        }
        Ok(out)
    }

    /// True iff the entity has at least one value row.
    pub async fn is_recorded(&self, id: &str) -> Result<bool, sqlx::Error> {
        let row = sqlx::query("SELECT 1 FROM entity_latest WHERE entity_id = $1 LIMIT 1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    /// Closes the pool. Used by tests to drop their per-test database cleanly.
    pub async fn close(self) {
        self.pool.close().await;
    }
}

async fn insert_values(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    values: &[ValueRow],
) -> Result<(), sqlx::Error> {
    let times: Vec<DateTime<Utc>> = values.iter().map(|r| micros_to_dt(r.at)).collect();
    let ids: Vec<String> = values.iter().map(|r| r.id.clone()).collect();
    let nums: Vec<Option<f64>> = values.iter().map(|r| r.num).collect();
    let payload: Vec<Json> = values
        .iter()
        .map(|r| serde_json::to_value(&r.value).unwrap_or(Json::Null))
        .collect();
    sqlx::query(
        "INSERT INTO entity_state (time, entity_id, num, value)
         SELECT * FROM UNNEST(
             $1::timestamptz[], $2::text[], $3::float8[], $4::jsonb[]
         ) AS t(time, entity_id, num, value)",
    )
    .bind(&times)
    .bind(&ids)
    .bind(&nums)
    .bind(&payload)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn upsert_entity_latest(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    values: &[ValueRow],
) -> Result<(), sqlx::Error> {
    for row in values {
        let v = serde_json::to_value(&row.value).unwrap_or(Json::Null);
        sqlx::query(
            "INSERT INTO entity_latest (entity_id, time, value)
             VALUES ($1, $2, $3)
             ON CONFLICT (entity_id) DO UPDATE SET
               time  = EXCLUDED.time,
               value = EXCLUDED.value
             WHERE EXCLUDED.time >= entity_latest.time",
        )
        .bind(&row.id)
        .bind(micros_to_dt(row.at))
        .bind(v)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn insert_availability(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    rows: &[AvailabilityRow],
) -> Result<(), sqlx::Error> {
    let times: Vec<DateTime<Utc>> = rows.iter().map(|r| micros_to_dt(r.at)).collect();
    let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    let avails: Vec<String> = rows
        .iter()
        .map(|r| availability_to_str(r.availability).to_owned())
        .collect();
    sqlx::query(
        "INSERT INTO entity_availability (time, entity_id, availability)
         SELECT * FROM UNNEST(
             $1::timestamptz[], $2::text[], $3::text[]
         ) AS t(time, entity_id, availability)",
    )
    .bind(&times)
    .bind(&ids)
    .bind(&avails)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn upsert_availability_latest(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    rows: &[AvailabilityRow],
) -> Result<(), sqlx::Error> {
    for row in rows {
        sqlx::query(
            "INSERT INTO availability_latest (entity_id, time, availability)
             VALUES ($1, $2, $3)
             ON CONFLICT (entity_id) DO UPDATE SET
               time         = EXCLUDED.time,
               availability = EXCLUDED.availability
             WHERE EXCLUDED.time >= availability_latest.time",
        )
        .bind(&row.id)
        .bind(micros_to_dt(row.at))
        .bind(availability_to_str(row.availability))
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub(super) fn micros_to_dt(m: Micros) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_micros(m).unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
}

pub(super) fn dt_to_micros(dt: DateTime<Utc>) -> Micros {
    dt.timestamp_micros()
}

fn availability_to_str(a: Availability) -> &'static str {
    match a {
        Availability::Online => "online",
        Availability::Offline => "offline",
        Availability::Unknown => "unknown",
    }
}

fn parse_availability(s: &str) -> Option<Availability> {
    match s {
        "online" => Some(Availability::Online),
        "offline" => Some(Availability::Offline),
        "unknown" => Some(Availability::Unknown),
        _ => None,
    }
}
