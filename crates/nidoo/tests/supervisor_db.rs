//! The sensor supervisor's problem table and its history queries.
//!
//! Needs `TEST_DATABASE_URL` (see `history_db.rs`); without it each test prints
//! `skipping: TEST_DATABASE_URL unset` and returns.

use crate::common;

use common::db::fresh_db;
use nidoo::supervisor::store;
use nidoo_watchdog::supervisor::{Check, Evidence, Problem};

const NOW: u64 = 1_800_000_000;
const H: u64 = 3600;

fn problem(id: &str, opened_at: u64) -> Problem {
    Problem {
        entity_id: id.into(),
        name: "Tank temperature".into(),
        check: Check::Stuck,
        opened_at,
        evidence: Evidence {
            value: Some(15.0),
            unit: Some("°C".into()),
            for_secs: 8 * H,
            usual_secs: Some(3 * H),
        },
        alert_id: Some(format!("supervisor-stuck-{id}-{opened_at}")),
    }
}

#[tokio::test]
async fn supervisor_problem_insert_load_close() {
    let Some(tdb) = fresh_db().await else { return };
    let p = problem("sensor.tank", NOW - 10 * H);
    store::insert(&tdb.db, &p).await.unwrap();
    // A second insert of the same problem is a no-op.
    store::insert(&tdb.db, &p).await.unwrap();
    assert_eq!(store::load_open(&tdb.db).await.unwrap(), vec![p.clone()]);

    store::close(&tdb.db, &p, NOW - H).await.unwrap();
    assert!(store::load_open(&tdb.db).await.unwrap().is_empty());
    let intervals = store::intervals(&tdb.db, NOW - 14 * 24 * H).await.unwrap();
    assert_eq!(intervals["sensor.tank"], vec![(NOW - 10 * H, NOW - H)]);

    store::purge(&tdb.db, NOW).await.unwrap();
    assert!(store::intervals(&tdb.db, 0).await.unwrap().is_empty());
    tdb.drop_db().await;
}

#[tokio::test]
async fn supervisor_problem_delete() {
    let Some(tdb) = fresh_db().await else { return };
    store::insert(&tdb.db, &problem("sensor.a", NOW - 5 * H))
        .await
        .unwrap();
    store::insert(&tdb.db, &problem("sensor.b", NOW - 5 * H))
        .await
        .unwrap();
    store::delete(&tdb.db, "sensor.a").await.unwrap();
    let open = store::load_open(&tdb.db).await.unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].entity_id, "sensor.b");
    store::delete_open(&tdb.db).await.unwrap();
    assert!(store::load_open(&tdb.db).await.unwrap().is_empty());
    tdb.drop_db().await;
}

#[tokio::test]
async fn supervisor_gap_stats_and_last_changes() {
    let Some(tdb) = fresh_db().await else { return };
    // 30 changes, 10 min apart, except one 5 h gap.
    let start = NOW - 10 * 24 * H;
    let mut t = start;
    for i in 0..30u64 {
        sqlx::query(
            "INSERT INTO entity_state (time, entity_id, num, value)
             VALUES (to_timestamp($1::double precision), 'sensor.tank', $2, to_jsonb($2))",
        )
        .bind(t as i64)
        .bind(i as f64)
        .execute(tdb.db.pool())
        .await
        .unwrap();
        t += if i == 10 { 5 * H } else { 600 };
    }
    sqlx::query(
        "INSERT INTO entity_latest (entity_id, time, value)
         VALUES ('sensor.tank', to_timestamp($1::double precision), '29')",
    )
    .bind((t - 600) as i64)
    .execute(tdb.db.pool())
    .await
    .unwrap();

    let ids = vec!["sensor.tank".to_string(), "sensor.none".to_string()];
    let stats = store::gap_stats(&tdb.db, &ids, NOW - 14 * 24 * H)
        .await
        .unwrap();
    assert_eq!(stats.len(), 1);
    let s = &stats[0];
    assert_eq!(s.entity_id, "sensor.tank");
    assert_eq!(s.changes, 30);
    assert_eq!(s.first, start);
    assert_eq!(s.top_gaps.len(), 5);
    let big = start + 10 * 600;
    assert_eq!(s.top_gaps[0], (big, big + 5 * H));

    let last = store::last_changes(&tdb.db, &ids).await.unwrap();
    assert_eq!(last.get("sensor.tank"), Some(&(t - 600)));
    assert!(!last.contains_key("sensor.none"));
    tdb.drop_db().await;
}
