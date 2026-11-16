//! The supervisor's problem table (`supervisor_problem`) and the history reads behind the
//! baselines. Times are Unix seconds.

use std::collections::{BTreeMap, HashMap};

use nidoo_watchdog::supervisor::{Check, Evidence, GapStats, Problem};
use sqlx::Row;

use crate::history::db::Db;

/// How many of a sensor's largest gaps the baseline query returns.
const TOP_GAPS: i64 = 5;

fn secs(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn unsecs(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

fn check_from(s: &str) -> Option<Check> {
    serde_json::from_value(serde_json::Value::String(s.to_owned())).ok()
}

/// The open problems, oldest first. A row whose check or evidence no longer decodes is
/// skipped.
pub async fn load_open(db: &Db) -> Result<Vec<Problem>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT entity_id, check_kind, extract(epoch FROM opened_at)::bigint, name, alert_id,
                evidence
         FROM supervisor_problem
         WHERE closed_at IS NULL
         ORDER BY opened_at, entity_id, check_kind",
    )
    .fetch_all(db.pool())
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let evidence: Evidence = serde_json::from_value(r.get(5)).ok()?;
            Some(Problem {
                entity_id: r.get(0),
                check: check_from(r.get(1))?,
                opened_at: unsecs(r.get(2)),
                name: r.get(3),
                alert_id: r.get(4),
                evidence,
            })
        })
        .collect())
}

/// Records a newly opened problem; one already stored is kept as is.
pub async fn insert(db: &Db, p: &Problem) -> Result<(), sqlx::Error> {
    let evidence = serde_json::to_value(&p.evidence).expect("evidence serializes");
    sqlx::query(
        "INSERT INTO supervisor_problem
             (entity_id, check_kind, opened_at, name, alert_id, evidence)
         VALUES ($1, $2, to_timestamp($3::double precision), $4, $5, $6)
         ON CONFLICT (entity_id, check_kind, opened_at) DO NOTHING",
    )
    .bind(&p.entity_id)
    .bind(p.check.as_str())
    .bind(secs(p.opened_at))
    .bind(&p.name)
    .bind(&p.alert_id)
    .bind(evidence)
    .execute(db.pool())
    .await?;
    Ok(())
}

/// Marks a problem closed at `at`.
pub async fn close(db: &Db, p: &Problem, at: u64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE supervisor_problem SET closed_at = to_timestamp($4::double precision)
         WHERE entity_id = $1 AND check_kind = $2
           AND opened_at = to_timestamp($3::double precision)",
    )
    .bind(&p.entity_id)
    .bind(p.check.as_str())
    .bind(secs(p.opened_at))
    .bind(secs(at))
    .execute(db.pool())
    .await?;
    Ok(())
}

/// Drops a muted sensor's open problems (no record kept: they were never real recoveries).
pub async fn delete(db: &Db, entity_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM supervisor_problem WHERE entity_id = $1 AND closed_at IS NULL")
        .bind(entity_id)
        .execute(db.pool())
        .await?;
    Ok(())
}

/// Drops every open problem (the supervisor was turned off).
pub async fn delete_open(db: &Db) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM supervisor_problem WHERE closed_at IS NULL")
        .execute(db.pool())
        .await?;
    Ok(())
}

/// Per entity, the time spans of its problems that touch `since` or later: closed ones as
/// (opened, closed), open ones as (opened, now). The baseline leaves gaps overlapping them out.
pub async fn intervals(
    db: &Db,
    since: u64,
) -> Result<BTreeMap<String, Vec<(u64, u64)>>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT entity_id, extract(epoch FROM opened_at)::bigint,
                extract(epoch FROM coalesce(closed_at, now()))::bigint
         FROM supervisor_problem
         WHERE coalesce(closed_at, now()) >= to_timestamp($1::double precision)
         ORDER BY entity_id, opened_at",
    )
    .bind(secs(since))
    .fetch_all(db.pool())
    .await?;
    let mut out: BTreeMap<String, Vec<(u64, u64)>> = BTreeMap::new();
    for r in rows {
        out.entry(r.get(0))
            .or_default()
            .push((unsecs(r.get(1)), unsecs(r.get(2))));
    }
    Ok(out)
}

/// Deletes the problems closed before `before`.
pub async fn purge(db: &Db, before: u64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM supervisor_problem
         WHERE closed_at IS NOT NULL AND closed_at < to_timestamp($1::double precision)",
    )
    .bind(secs(before))
    .execute(db.pool())
    .await?;
    Ok(())
}

/// Per entity in `ids` with numeric history since `since`: the change count, the first
/// change, and the five largest gaps between consecutive changes (largest first). One grouped
/// query; entities without rows are absent.
pub async fn gap_stats(db: &Db, ids: &[String], since: u64) -> Result<Vec<GapStats>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        "WITH s AS (
             SELECT entity_id, time,
                    lag(time) OVER (PARTITION BY entity_id ORDER BY time, seq) AS prev
             FROM entity_state
             WHERE entity_id = ANY($1) AND num IS NOT NULL
               AND time >= to_timestamp($2::double precision)
         ), c AS (
             SELECT entity_id, count(*) AS changes, min(time) AS first
             FROM s GROUP BY entity_id
         ), g AS (
             SELECT entity_id, prev, time,
                    row_number() OVER (PARTITION BY entity_id ORDER BY time - prev DESC, time)
                        AS rk
             FROM s WHERE prev IS NOT NULL
         )
         SELECT c.entity_id, c.changes, extract(epoch FROM c.first)::bigint,
                extract(epoch FROM g.prev)::bigint, extract(epoch FROM g.time)::bigint
         FROM c LEFT JOIN g ON g.entity_id = c.entity_id AND g.rk <= $3
         ORDER BY c.entity_id, g.rk",
    )
    .bind(ids)
    .bind(secs(since))
    .bind(TOP_GAPS)
    .fetch_all(db.pool())
    .await?;
    let mut out: Vec<GapStats> = Vec::new();
    for r in rows {
        let id: String = r.get(0);
        if out.last().is_none_or(|s| s.entity_id != id) {
            out.push(GapStats {
                entity_id: id,
                changes: u32::try_from(r.get::<i64, _>(1)).unwrap_or(u32::MAX),
                first: unsecs(r.get(2)),
                top_gaps: Vec::new(),
            });
        }
        if let (Some(start), Some(end)) = (r.get::<Option<i64>, _>(3), r.get::<Option<i64>, _>(4)) {
            let stats = out.last_mut().expect("pushed above");
            stats.top_gaps.push((unsecs(start), unsecs(end)));
        }
    }
    Ok(out)
}

/// When each entity in `ids` last changed value (`entity_latest`).
pub async fn last_changes(db: &Db, ids: &[String]) -> Result<HashMap<String, u64>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query(
        "SELECT entity_id, extract(epoch FROM time)::bigint
         FROM entity_latest WHERE entity_id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(db.pool())
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.get(0), unsecs(r.get(1))))
        .collect())
}
