//! Database-backed tests for the history layer.
//!
//! These tests need a reachable TimescaleDB. Set
//! `TEST_DATABASE_URL=postgres://nidoo:change-me@localhost:5432/nidoo` to run them against
//! the compose file in `deploy/timescaledb/`. Without the variable each test prints
//! `skipping: TEST_DATABASE_URL unset` and returns, so the suite stays green on machines
//! without a database.

mod common;

use common::db::{TestDb, fresh_db};
use nidoo::history::climate::ClimateDecisionRow;
use nidoo::history::db::Db;
use nidoo::history::rollup::rollup_once;
use nidoo::history::solar::SolarForecastRow;
use nidoo_core::history::buckets::Bucket;
use nidoo_core::history::{AvailabilityRow, Incoming, Rows, ValueRow};
use nidoo_core::model::{Availability, ClimateState, EntityState, StateValue};

fn value_row(id: &str, at_micros: i64, num: f64) -> ValueRow {
    ValueRow {
        at: at_micros,
        id: id.into(),
        value: StateValue::Number(num),
        num: Some(num),
    }
}

fn availability_row(id: &str, at_micros: i64, availability: Availability) -> AvailabilityRow {
    AvailabilityRow {
        at: at_micros,
        id: id.into(),
        availability,
    }
}

fn make_bucket(start: i64, last: f64, max: f64, increase: f64) -> Bucket {
    Bucket {
        start,
        covered: 3_600_000_000,
        integral: last,
        min: last,
        max,
        last,
        increase,
    }
}

#[tokio::test]
async fn insert_then_load_latest_round_trips() {
    let Some(tdb) = fresh_db().await else { return };
    let rows = Rows {
        values: vec![
            value_row("a", 1_000_000, 21.0),
            value_row("a", 1_000_001, 22.0),
        ],
        availability: vec![availability_row("a", 1_000_000, Availability::Online)],
    };
    tdb.db.insert(&rows).await.unwrap();

    let mut lw = tdb.db.load_latest().await.unwrap();
    let mut out = Rows::default();
    lw.diff(
        &Incoming {
            at: 2_000_000,
            id: "a".into(),
            state: EntityState {
                value: StateValue::Number(22.0),
                attributes: serde_json::Map::new(),
                availability: Availability::Online,
                last_update: Some(2),
            },
            num: Some(22.0),
        },
        &mut out,
    );
    assert!(out.is_empty(), "{out:?}");

    tdb.drop_db().await;
}

#[tokio::test]
async fn same_microsecond_keeps_insert_order() {
    let Some(tdb) = fresh_db().await else { return };
    let rows = Rows {
        values: vec![
            value_row("a", 1_000_000, 1.0),
            value_row("a", 1_000_000, 2.0),
        ],
        availability: vec![],
    };
    tdb.db.insert(&rows).await.unwrap();
    let got = tdb.db.samples(0, 2_000_000, Some("a")).await.unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].1.num, 1.0);
    assert_eq!(got[1].1.num, 2.0);

    tdb.drop_db().await;
}

#[tokio::test]
async fn last_changes_and_recorded() {
    let Some(tdb) = fresh_db().await else { return };
    tdb.db
        .insert(&Rows {
            values: vec![
                value_row("a", 1_000_000, 1.0),
                value_row("a", 2_000_000, 2.0),
            ],
            availability: vec![],
        })
        .await
        .unwrap();

    assert!(tdb.db.is_recorded("a").await.unwrap());
    assert!(!tdb.db.is_recorded("ghost").await.unwrap());

    let changes = tdb
        .db
        .last_changes(&["a".into(), "ghost".into()])
        .await
        .unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes.get("a"), Some(&2_000_000));

    tdb.drop_db().await;
}

#[tokio::test]
async fn hourly_upsert_replaces() {
    let Some(tdb) = fresh_db().await else { return };
    let first = make_bucket(0, 1.0, 1.0, 0.0);
    let second = Bucket {
        last: 5.0,
        max: 5.0,
        increase: 2.0,
        ..first.clone()
    };
    tdb.db.upsert_hourly(&[("a".into(), first)]).await.unwrap();
    tdb.db
        .upsert_hourly(&[("a".into(), second.clone())])
        .await
        .unwrap();
    let hours = tdb.db.hourly("a", 0, 3_600_000_000).await.unwrap();
    assert_eq!(hours.len(), 1);
    assert_eq!(hours[0].last, 5.0);
    assert_eq!(hours[0].max, 5.0);
    assert_eq!(hours[0].increase, 2.0);

    tdb.drop_db().await;
}

#[tokio::test]
async fn hourly_carry_takes_latest_before() {
    let Some(tdb) = fresh_db().await else { return };
    let hour = 3_600_000_000i64;
    tdb.db
        .upsert_hourly(&[
            ("a".into(), make_bucket(0, 1.0, 1.0, 0.0)),
            ("a".into(), make_bucket(2 * hour, 3.0, 3.0, 0.0)),
        ])
        .await
        .unwrap();
    let carry = tdb.db.hourly_carry(2 * hour, &["a".into()]).await.unwrap();
    assert_eq!(carry.get("a"), Some(&1.0));

    tdb.drop_db().await;
}

const HOUR: i64 = 3_600_000_000;
const MINUTE: i64 = 60_000_000;

#[tokio::test]
async fn replay_after_restart_writes_nothing() {
    let Some(tdb) = fresh_db().await else { return };
    tdb.db
        .insert(&Rows {
            values: vec![value_row("a", 1_000_000, 20.0)],
            availability: vec![availability_row("a", 1_000_000, Availability::Online)],
        })
        .await
        .unwrap();
    let mut lw = tdb.db.load_latest().await.unwrap();
    let mut rows = Rows::default();
    lw.diff(
        &Incoming {
            at: 2_000_000,
            id: "a".into(),
            state: EntityState {
                value: StateValue::Number(20.0),
                attributes: serde_json::Map::new(),
                availability: Availability::Online,
                last_update: Some(2),
            },
            num: Some(20.0),
        },
        &mut rows,
    );
    assert!(rows.is_empty(), "same state after restart writes nothing");

    let mut rows = Rows::default();
    lw.diff(
        &Incoming {
            at: 3_000_000,
            id: "a".into(),
            state: EntityState {
                value: StateValue::Number(21.0),
                attributes: serde_json::Map::new(),
                availability: Availability::Online,
                last_update: Some(3),
            },
            num: Some(21.0),
        },
        &mut rows,
    );
    assert_eq!(rows.values.len(), 1);
    assert_eq!(rows.values[0].num, Some(21.0));

    tdb.drop_db().await;
}

#[tokio::test]
async fn rollup_fills_hourly_with_carry() {
    let Some(tdb) = fresh_db().await else { return };
    tdb.db
        .insert(&Rows {
            values: vec![
                value_row("a", 15 * MINUTE, 10.0),
                value_row("a", HOUR + 30 * MINUTE, 20.0),
            ],
            availability: vec![],
        })
        .await
        .unwrap();

    rollup_once(&tdb.db, 3 * HOUR, i64::MAX).await.unwrap();
    let hours = tdb.db.hourly("a", 0, 3 * HOUR).await.unwrap();
    assert_eq!(hours.len(), 2, "{hours:?}");
    assert_eq!(hours[1].min, 10.0);
    assert_eq!(hours[1].max, 20.0);
    assert_eq!(hours[1].integral, 15.0);

    tdb.drop_db().await;
}

#[tokio::test]
async fn late_rows_recompute_their_hour() {
    let Some(tdb) = fresh_db().await else { return };
    tdb.db
        .insert(&Rows {
            values: vec![
                value_row("a", 15 * MINUTE, 10.0),
                value_row("a", HOUR + 30 * MINUTE, 20.0),
            ],
            availability: vec![],
        })
        .await
        .unwrap();
    rollup_once(&tdb.db, 3 * HOUR, i64::MAX).await.unwrap();

    tdb.db
        .insert(&Rows {
            values: vec![value_row("a", HOUR + 45 * MINUTE, 30.0)],
            availability: vec![],
        })
        .await
        .unwrap();
    rollup_once(&tdb.db, 3 * HOUR, HOUR).await.unwrap();

    let hours = tdb.db.hourly("a", HOUR, 2 * HOUR).await.unwrap();
    assert_eq!(hours.len(), 1);
    assert_eq!(hours[0].max, 30.0);
    assert_eq!(hours[0].last, 30.0);

    tdb.drop_db().await;
}

#[tokio::test]
async fn series_24h_carries_into_quiet_windows() {
    let Some(tdb) = fresh_db().await else { return };
    let now = 48 * HOUR; // arbitrary "present"
    tdb.db
        .insert(&Rows {
            values: vec![value_row("a", now - 2 * HOUR, 5.0)],
            availability: vec![],
        })
        .await
        .unwrap();
    rollup_once(&tdb.db, now, i64::MAX).await.unwrap();

    let history = nidoo::history::History::with_db(tdb.db.clone(), chrono_tz::UTC);
    let series = nidoo::history::query::series(&history, "a", nidoo_api::HistoryRange::Day, now)
        .await
        .expect("series");

    // Points are (t, mean, min, max) with t in unix seconds.
    assert!(!series.points.is_empty(), "{series:?}");
    // The first point is at or after now - 2h (no point for the earlier quiet windows).
    let first_t = series.points[0].0;
    let two_hours_ago_s = (now - 2 * HOUR) / 1_000_000;
    assert!(
        first_t >= two_hours_ago_s - 600,
        "{first_t} vs {two_hours_ago_s}"
    );
    // Every point carries 5.0.
    for p in &series.points {
        assert_eq!(p.1, 5.0, "mean");
        assert_eq!(p.2, 5.0, "min");
        assert_eq!(p.3, 5.0, "max");
    }

    tdb.drop_db().await;
}

#[tokio::test]
async fn series_short_ranges_use_raw_samples() {
    let Some(tdb) = fresh_db().await else { return };
    let now = 48 * HOUR;
    tdb.db
        .insert(&Rows {
            values: vec![value_row("a", now - HOUR / 2, 7.0)],
            availability: vec![],
        })
        .await
        .unwrap();

    let history = nidoo::history::History::with_db(tdb.db.clone(), chrono_tz::UTC);
    for (range, window) in [
        (nidoo_api::HistoryRange::Hour, 60),
        (nidoo_api::HistoryRange::SixHours, 300),
    ] {
        let series = nidoo::history::query::series(&history, "a", range, now)
            .await
            .expect("series");
        assert_eq!(series.window, window);
        assert!(!series.points.is_empty(), "{series:?}");
        assert!(series.points.iter().all(|p| p.1 == 7.0), "{series:?}");
    }

    tdb.drop_db().await;
}

#[tokio::test]
async fn today_power_and_unrecorded() {
    let Some(tdb) = fresh_db().await else { return };
    // Reference "now" chosen so UTC midnight is a round micros count. HOUR = 1h.
    let midnight = 100 * 24 * HOUR;
    let now = midnight + 3 * HOUR;
    tdb.db
        .insert(&Rows {
            values: vec![value_row("a", midnight + HOUR, 1000.0)],
            availability: vec![],
        })
        .await
        .unwrap();

    let history = nidoo::history::History::with_db(tdb.db.clone(), chrono_tz::UTC);
    let today = nidoo::history::query::today(&history, "a", nidoo_api::EnergyKind::Power, now)
        .await
        .expect("today");
    assert_eq!(today.since, midnight / 1_000_000);
    assert_eq!(today.kwh, Some(2.0));

    let err = nidoo::history::query::series(&history, "ghost", nidoo_api::HistoryRange::Day, now)
        .await
        .expect_err("not recorded");
    assert_eq!(err.code(), "not_recorded");

    let changes = nidoo::history::query::last_change(&history, &["a".into(), "ghost".into()])
        .await
        .expect("last_change");
    assert_eq!(changes.items.len(), 2);
    assert!(changes.items[0].recorded);
    assert!(!changes.items[1].recorded);
    assert_eq!(changes.items[1].changed_at, None);

    tdb.drop_db().await;
}

#[tokio::test]
async fn writer_survives_unreachable_database_at_boot() {
    use std::time::Duration;

    let cfg = nidoo::config::HistoryConfig {
        url: "postgres://nouser:nopass@127.0.0.1:1/none".into(),
        buffer_rows: 100,
    };
    let (history, handle) =
        nidoo::history::History::spawn_writer_for_tests(&cfg, chrono_tz::UTC).expect("lazy pool");

    history.offer(Incoming {
        at: 1_000_000,
        id: "a".into(),
        state: EntityState {
            value: StateValue::OnOff(true),
            attributes: serde_json::Map::new(),
            availability: Availability::Online,
            last_update: Some(1),
        },
        num: None,
    });

    // Wait long enough for sqlx's acquire_timeout (5 s) to fire plus a retry tick: the
    // regressed writer would have called migrate(), timed out, and returned by now.
    tokio::time::sleep(Duration::from_secs(8)).await;
    assert!(
        !handle.is_finished(),
        "writer must not exit when the database is unreachable at boot"
    );

    handle.abort();
}

fn decision(at: i64, zone: &str, entity: &str, applied: bool, reason: &str) -> ClimateDecisionRow {
    ClimateDecisionRow {
        at,
        zone: zone.into(),
        entity_id: entity.into(),
        mode: "heat".into(),
        setpoint: Some(19.5),
        source: "schedule".into(),
        reason: reason.into(),
        applied,
        adjust: None,
    }
}

fn climate_row(id: &str, at: i64, target: Option<f64>, mode: &str) -> ValueRow {
    ValueRow {
        at,
        id: id.into(),
        value: StateValue::Climate(ClimateState {
            current: Some(20.0),
            target,
            mode: Some(mode.into()),
            fan_mode: None,
            action: None,
        }),
        num: None,
    }
}

#[tokio::test]
async fn climate_decisions_round_trip() {
    let Some(tdb) = fresh_db().await else { return };
    let mut empty = decision(3_000_000, "z1", "", false, "no devices");
    empty.mode = String::new();
    empty.setpoint = None;
    let rows = vec![
        decision(1_000_000, "z1", "climate.a", true, "first"),
        decision(2_000_000, "z1", "climate.a", false, "second"),
        empty.clone(),
        decision(2_500_000, "z2", "climate.b", true, "other zone"),
    ];
    tdb.db.insert_climate_decisions(&rows).await.unwrap();

    let got = tdb.db.climate_decisions("z1", None, 10).await.unwrap();
    assert_eq!(got.len(), 3);
    assert_eq!(got[0], empty);
    assert_eq!(got[1], rows[1]);
    assert_eq!(got[2], rows[0]);
    let earlier = tdb
        .db
        .climate_decisions("z1", Some(2_000_000), 10)
        .await
        .unwrap();
    assert_eq!(earlier.len(), 1, "strictly before the bound");
    assert_eq!(earlier[0].at, 1_000_000);
    let limited = tdb.db.climate_decisions("z1", None, 1).await.unwrap();
    assert_eq!(limited, vec![empty]);

    tdb.drop_db().await;
}

#[tokio::test]
async fn climate_decision_adjust_round_trips() {
    let Some(tdb) = fresh_db().await else { return };
    let mut corrected = decision(1_000_000, "z1", "climate.a", true, "corrected");
    corrected.adjust = Some(-0.5);
    let plain = decision(2_000_000, "z1", "climate.b", true, "plain");
    tdb.db
        .insert_climate_decisions(&[corrected.clone(), plain.clone()])
        .await
        .unwrap();

    let got = tdb.db.climate_decisions("z1", None, 10).await.unwrap();
    assert_eq!(got, vec![plain.clone(), corrected.clone()]);
    let applied = tdb
        .db
        .climate_commands(
            &["climate.a".to_string(), "climate.b".to_string()],
            0,
            5_000_000,
        )
        .await
        .unwrap();
    assert_eq!(applied, vec![corrected, plain]);

    tdb.drop_db().await;
}

#[tokio::test]
async fn climate_commands_filters_applied_and_entities() {
    let Some(tdb) = fresh_db().await else { return };
    let rows = vec![
        decision(1_000_000, "z1", "climate.a", true, "in"),
        decision(2_000_000, "z1", "climate.a", false, "not applied"),
        decision(3_000_000, "z1", "climate.b", true, "other entity"),
        decision(4_000_000, "z1", "climate.a", true, "also in"),
        decision(9_000_000, "z1", "climate.a", true, "after window"),
    ];
    tdb.db.insert_climate_decisions(&rows).await.unwrap();

    let got = tdb
        .db
        .climate_commands(&["climate.a".to_string()], 0, 5_000_000)
        .await
        .unwrap();
    let reasons: Vec<&str> = got.iter().map(|r| r.reason.as_str()).collect();
    assert_eq!(reasons, ["in", "also in"]);
    assert!(
        tdb.db
            .climate_commands(&[], 0, 5_000_000)
            .await
            .unwrap()
            .is_empty()
    );

    tdb.drop_db().await;
}

#[tokio::test]
async fn climate_targets_reads_jsonb_and_carries_prior_value() {
    let Some(tdb) = fresh_db().await else { return };
    let t: i64 = 1_800_000_000_000_000;
    let h = 3_600_000_000;
    let m = 60_000_000;
    let rows = Rows {
        values: vec![
            climate_row("climate.a", t - 2 * h, Some(18.0), "heat"),
            climate_row("climate.a", t - 30 * m, Some(20.0), "heat"),
            climate_row("climate.a", t + 10 * m, None, "off"),
            climate_row("climate.b", t - 15 * m, Some(25.0), "cool"),
            value_row("climate.a", t - 5 * m, 7.0),
        ],
        availability: vec![],
    };
    tdb.db.insert(&rows).await.unwrap();

    let got = tdb
        .db
        .climate_targets("climate.a", t - h, t + h)
        .await
        .unwrap();
    assert_eq!(
        got,
        vec![
            (t - h, Some(18.0), Some("heat".to_string())),
            (t - 30 * m, Some(20.0), Some("heat".to_string())),
            (t + 10 * m, None, Some("off".to_string())),
        ]
    );

    tdb.drop_db().await;
}

#[tokio::test]
async fn equal_times_keep_insertion_order() {
    let Some(tdb) = fresh_db().await else { return };
    let rows = vec![
        decision(1_000_000, "z1", "climate.a", true, "first"),
        decision(1_000_000, "z1", "climate.a", true, "second"),
        decision(1_000_000, "z1", "climate.a", true, "third"),
    ];
    tdb.db.insert_climate_decisions(&rows).await.unwrap();

    let newest_first: Vec<String> = tdb
        .db
        .climate_decisions("z1", None, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.reason)
        .collect();
    assert_eq!(newest_first, ["third", "second", "first"]);
    let oldest_first: Vec<String> = tdb
        .db
        .climate_commands(&["climate.a".to_string()], 0, 5_000_000)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.reason)
        .collect();
    assert_eq!(oldest_first, ["first", "second", "third"]);

    let t: i64 = 1_800_000_000_000_000;
    let m = 60_000_000;
    let states = Rows {
        values: vec![
            climate_row("climate.a", t - 10 * m, Some(18.0), "heat"),
            climate_row("climate.a", t - 10 * m, Some(21.0), "heat"),
            climate_row("climate.a", t + m, Some(19.0), "heat"),
            climate_row("climate.a", t + m, None, "off"),
        ],
        availability: vec![],
    };
    tdb.db.insert(&states).await.unwrap();
    let got = tdb
        .db
        .climate_targets("climate.a", t, t + 10 * m)
        .await
        .unwrap();
    assert_eq!(
        got,
        vec![
            (t, Some(21.0), Some("heat".to_string())),
            (t + m, Some(19.0), Some("heat".to_string())),
            (t + m, None, Some("off".to_string())),
        ],
        "the later write wins the prior value, and same-time rows stay in order"
    );

    tdb.drop_db().await;
}

#[tokio::test]
async fn mean_since_averages_num() {
    let Some(tdb) = fresh_db().await else { return };
    let rows = Rows {
        values: vec![
            value_row("out", 1_000_000, 10.0),
            value_row("out", 2_000_000, 20.0),
            value_row("out", 9_000_000, 99.0),
            value_row("other", 1_500_000, 50.0),
        ],
        availability: vec![],
    };
    tdb.db.insert(&rows).await.unwrap();

    assert_eq!(
        tdb.db.mean_since("out", 0, 5_000_000).await.unwrap(),
        Some(15.0)
    );
    assert_eq!(tdb.db.mean_since("nope", 0, 5_000_000).await.unwrap(), None);

    tdb.drop_db().await;
}

// ----- climate learning runs -----

mod solar {
    use super::*;
    use chrono::{NaiveDate, TimeZone, Utc};

    fn day(d: u32, estimate: f64) -> SolarForecastRow {
        SolarForecastRow {
            date: NaiveDate::from_ymd_opt(2026, 10, d).unwrap(),
            estimate,
            low: estimate - 1.0,
            high: estimate + 1.0,
            made_at: Utc.with_ymd_and_hms(2026, 10, d, 6, 0, 0).unwrap(),
        }
    }

    #[tokio::test]
    async fn solar_forecast_first_row_of_day_wins() {
        let Some(tdb) = fresh_db().await else { return };
        assert!(
            tdb.db
                .insert_solar_forecast_day(&day(20, 10.0))
                .await
                .unwrap()
        );
        assert!(
            !tdb.db
                .insert_solar_forecast_day(&day(20, 99.0))
                .await
                .unwrap()
        );
        let rows = tdb
            .db
            .solar_forecast_days(day(20, 0.0).date, day(21, 0.0).date)
            .await
            .unwrap();
        assert_eq!(rows, vec![day(20, 10.0)]);
        tdb.drop_db().await;
    }

    #[tokio::test]
    async fn solar_forecast_days_range_is_half_open_and_sorted() {
        let Some(tdb) = fresh_db().await else { return };
        for d in [22, 20, 21, 23] {
            tdb.db
                .insert_solar_forecast_day(&day(d, f64::from(d)))
                .await
                .unwrap();
        }
        let rows = tdb
            .db
            .solar_forecast_days(day(21, 0.0).date, day(23, 0.0).date)
            .await
            .unwrap();
        assert_eq!(rows, vec![day(21, 21.0), day(22, 22.0)]);
        tdb.drop_db().await;
    }
}

mod learning {
    use std::sync::Arc;

    use chrono::{Datelike, TimeZone, Weekday};
    use nidoo::config::{Config, HistoryConfig};
    use nidoo::hub::Hub;
    use nidoo_api::ConnectionStatus;
    use nidoo_climate::doc::{Day, ProposalKind, Setpoint, ZoneState};
    use nidoo_climate::view::LearnStatus;
    use nidoo_core::command::Publish;
    use tokio::sync::mpsc;

    use super::*;

    const DAY_MICROS: i64 = 86_400_000_000;

    /// A hub with one Daikin in `living`, reading its history from `tdb`. The house clock is UTC
    /// so the rows below land on the days the learner reads.
    async fn hub_on(
        tdb: &TestDb,
    ) -> (
        Arc<Hub>,
        String,
        mpsc::Receiver<Publish>,
        tokio::task::JoinHandle<()>,
    ) {
        let mut cfg = Config::default();
        cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
        cfg.climate.enabled = true;
        cfg.flows.timezone = Some("UTC".to_owned());
        let (tx, rx) = mpsc::channel(256);
        let hub = Hub::new(&cfg, tx);
        hub.set_connection(ConnectionStatus::Online);
        let fixture = std::fs::read(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../nidoo-core/tests/fixtures/discovery/")
                .join("climate__daikin_192_168_1_150__climate.json"),
        )
        .expect("fixture");
        hub.ingest(
            "nidoo/climate/daikin_192_168_1_150/climate/config",
            &fixture,
        );
        let id = hub
            .snapshot()
            .entities
            .into_iter()
            .find(|e| e.id.starts_with("daikin_192_168_1_150"))
            .expect("daikin entity")
            .id;
        let mut patch = serde_json::Map::new();
        patch.insert("area".to_owned(), serde_json::Value::from("living"));
        hub.set_override(&id, &patch).await.expect("area");
        let history_cfg = HistoryConfig {
            url: tdb.url.clone(),
            buffer_rows: 100,
        };
        let (history, writer) =
            nidoo::history::History::spawn_writer_for_tests(&history_cfg, chrono_tz::UTC)
                .expect("lazy pool");
        hub.set_history(history);
        (hub, id, rx, writer)
    }

    /// 28 whole days before today of a heater held at 17 degrees, raised to 20.5 between 07:00
    /// and 22:00 on weekdays.
    async fn seed_weekday_routine(tdb: &TestDb, id: &str) {
        let now = nidoo::clock::now();
        let today = i64::try_from(now - now % 86_400).unwrap() * 1_000_000;
        let mut values = Vec::new();
        for back in 1..=28 {
            let start = today - back * DAY_MICROS;
            let date = chrono::Utc
                .timestamp_micros(start)
                .single()
                .expect("a valid day");
            values.push(climate_row(id, start, Some(17.0), "heat"));
            if !matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
                values.push(climate_row(
                    id,
                    start + 7 * 3_600_000_000,
                    Some(20.5),
                    "heat",
                ));
                values.push(climate_row(
                    id,
                    start + 22 * 3_600_000_000,
                    Some(17.0),
                    "heat",
                ));
            }
        }
        tdb.db
            .insert(&Rows {
                values,
                availability: vec![],
            })
            .await
            .unwrap();
    }

    fn pending(hub: &Hub) -> Option<nidoo_climate::doc::Proposal> {
        hub.climate_view().unwrap().zones[0].proposal.clone()
    }

    #[tokio::test]
    async fn learn_zone_proposes_initial_rules_from_history() {
        let Some(tdb) = fresh_db().await else { return };
        let (hub, id, _rx, writer) = hub_on(&tdb).await;
        seed_weekday_routine(&tdb, &id).await;

        let status = hub.learn_climate_zone("living").await.unwrap();
        assert_eq!(status, LearnStatus::Proposed);
        let proposal = pending(&hub).expect("a proposal is pending");
        assert_eq!(proposal.kind, ProposalKind::Initial);
        assert!(proposal.id.ends_with("-living"), "{}", proposal.id);
        assert_eq!(proposal.window.days, 28);
        let heat = proposal.rules.heat.expect("a heat schedule");
        assert_eq!(heat.base, Setpoint::Temp(17.0));
        let weekday_block = heat
            .blocks
            .iter()
            .find(|b| b.days.contains(&Day::Mon) && !b.days.contains(&Day::Sat))
            .unwrap_or_else(|| panic!("a weekday block in {heat:?}"));
        assert_eq!(weekday_block.setpoint, Setpoint::Temp(20.5));
        assert_eq!((weekday_block.from.0, weekday_block.to.0), (420, 1320));
        assert_eq!(
            hub.climate_view().unwrap().zones[0].settings.state,
            ZoneState::Pending,
            "learning never activates a zone"
        );

        writer.abort();
        tdb.drop_db().await;
    }

    #[tokio::test]
    async fn learn_zone_skips_rejected_fingerprint() {
        let Some(tdb) = fresh_db().await else { return };
        let (hub, id, _rx, writer) = hub_on(&tdb).await;
        seed_weekday_routine(&tdb, &id).await;

        assert_eq!(
            hub.learn_climate_zone("living").await.unwrap(),
            LearnStatus::Proposed
        );
        hub.reject_climate_proposal("living").await.unwrap();
        assert!(pending(&hub).is_none());

        // The same history makes the same proposal, which is remembered as rejected.
        assert_eq!(
            hub.learn_climate_zone("living").await.unwrap(),
            LearnStatus::Rejected
        );
        assert!(pending(&hub).is_none());

        writer.abort();
        tdb.drop_db().await;
    }

    #[tokio::test]
    async fn learn_zone_reports_days_still_needed() {
        let Some(tdb) = fresh_db().await else { return };
        let (hub, id, _rx, writer) = hub_on(&tdb).await;
        // Two days of data, ten needed.
        let now = nidoo::clock::now();
        let today = i64::try_from(now - now % 86_400).unwrap() * 1_000_000;
        tdb.db
            .insert(&Rows {
                values: vec![
                    climate_row(&id, today - 2 * DAY_MICROS, Some(17.0), "heat"),
                    climate_row(&id, today - DAY_MICROS, Some(17.0), "heat"),
                ],
                availability: vec![],
            })
            .await
            .unwrap();
        let status = hub.learn_climate_zone("living").await.unwrap();
        assert_eq!(
            status,
            LearnStatus::NotEnough {
                days: 2,
                needed: 10
            }
        );
        assert_eq!(
            hub.climate_view().unwrap().zones[0].learn,
            LearnStatus::NotEnough {
                days: 2,
                needed: 10
            }
        );
        assert!(pending(&hub).is_none());

        writer.abort();
        tdb.drop_db().await;
    }
}

fn paris(s: &str) -> i64 {
    use chrono::TimeZone;
    let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap();
    chrono_tz::Europe::Paris
        .from_local_datetime(&naive)
        .single()
        .unwrap()
        .timestamp_micros()
}

fn off_peak_history() -> Vec<nidoo_core::energy::OffPeakEntry> {
    use nidoo_core::energy::{OffPeakEntry, OffPeakWindow};
    let w = |a: &str, b: &str| OffPeakWindow {
        start: a.into(),
        end: b.into(),
    };
    vec![OffPeakEntry {
        since: "2026-01-01".into(),
        windows: vec![w("01:34", "07:04"), w("12:05", "14:34")],
    }]
}

#[tokio::test]
async fn grid_day_totals_match_counter_increase_db() {
    use nidoo::history::{History, query};
    use nidoo_core::energy::GridCounters;
    let Some(tdb) = fresh_db().await else { return };
    tdb.db
        .insert(&Rows {
            values: vec![
                value_row("bought", paris("2026-10-22 23:50:00"), 100.0),
                value_row("bought", paris("2026-10-23 01:40:00"), 100.4),
                value_row("bought", paris("2026-10-23 08:00:00"), 101.0),
                value_row("bought", paris("2026-10-23 13:00:00"), 101.5),
                value_row("sold", paris("2026-10-22 23:50:00"), 5.0),
                value_row("sold", paris("2026-10-23 13:10:00"), 5.6),
            ],
            availability: vec![],
        })
        .await
        .unwrap();
    let h = History::with_db(
        Db::connect_lazy(&tdb.url).unwrap(),
        chrono_tz::Europe::Paris,
    );
    let counters = GridCounters {
        bought: vec!["bought".into()],
        sold: vec!["sold".into()],
        ..GridCounters::default()
    };
    let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 23);
    let now = paris("2026-10-24 09:00:00");
    let day = query::grid_day(&h, &counters, &off_peak_history(), None, date, now)
        .await
        .unwrap();
    assert_eq!(day.slots.len(), 48);
    assert_eq!(
        (day.totals.off_peak, day.totals.peak, day.totals.export),
        (0.9, 0.6, 0.6)
    );
    assert_eq!(day.slots[3].off_peak, 0.4);
    assert_eq!(day.first_date.as_deref(), Some("2026-10-22"));

    let future = chrono::NaiveDate::from_ymd_opt(2026, 10, 25);
    let err = query::grid_day(&h, &counters, &off_peak_history(), None, future, now)
        .await
        .unwrap_err();
    assert_eq!(err.code(), "bad_request");
    let err = query::grid_day(&h, &GridCounters::default(), &[], None, None, now)
        .await
        .unwrap_err();
    assert_eq!(err.code(), "no_grid_meter");

    tdb.drop_db().await;
}

#[tokio::test]
async fn grid_day_reads_text_device_rows_db() {
    use nidoo::history::{History, query};
    use nidoo_core::energy::{GridCounters, OffPeakDevice};
    let Some(tdb) = fresh_db().await else { return };
    let text = |id: &str, at: i64, s: &str| ValueRow {
        at,
        id: id.into(),
        value: StateValue::Text(s.into()),
        num: None,
    };
    tdb.db
        .insert(&Rows {
            values: vec![
                value_row("bought", paris("2026-10-22 23:50:00"), 100.0),
                value_row("bought", paris("2026-10-23 01:30:00"), 100.4),
                value_row("bought", paris("2026-10-23 03:30:00"), 101.0),
                text("tariff", paris("2026-10-22 22:00:00"), "OFF"),
                text("tariff", paris("2026-10-23 02:00:00"), "PEAK"),
            ],
            availability: vec![availability_row(
                "tariff",
                paris("2026-10-23 03:00:00"),
                Availability::Offline,
            )],
        })
        .await
        .unwrap();
    let (from, to) = (paris("2026-10-23 00:00:00"), paris("2026-10-24 00:00:00"));
    let (carry, rows) = tdb.db.states_between("tariff", from, to).await.unwrap();
    assert_eq!(carry, Some(StateValue::Text("OFF".into())));
    assert_eq!(
        rows,
        [(
            paris("2026-10-23 02:00:00"),
            StateValue::Text("PEAK".into())
        )]
    );
    let (carry, rows) = tdb
        .db
        .availability_between("tariff", from, to)
        .await
        .unwrap();
    assert_eq!(carry, None);
    assert_eq!(
        rows,
        [(paris("2026-10-23 03:00:00"), Availability::Offline)]
    );

    let h = History::with_db(
        Db::connect_lazy(&tdb.url).unwrap(),
        chrono_tz::Europe::Paris,
    );
    let counters = GridCounters {
        bought: vec!["bought".into()],
        ..GridCounters::default()
    };
    let device = OffPeakDevice {
        entity: "tariff".into(),
        values: vec!["OFF".into()],
    };
    let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 23);
    let now = paris("2026-10-24 09:00:00");
    // No windows: 01:30 is off-peak by the device, 03:30 has no usable reading → peak.
    let day = query::grid_day(&h, &counters, &[], Some(&device), date, now)
        .await
        .unwrap();
    assert_eq!((day.totals.off_peak, day.totals.peak), (0.4, 0.6));
    assert!(day.tariffs);

    tdb.drop_db().await;
}

#[tokio::test]
async fn restart_log_round_trip() {
    use nidoo_api::system::{RestartView, StopKind};
    let Some(tdb) = fresh_db().await else { return };
    let mk = |started_at, down_secs, stop| RestartView {
        started_at,
        down_secs,
        stop,
    };
    let rows = [
        mk(1_000_000, None, StopKind::First),
        mk(2_000_000, Some(30), StopKind::Clean),
        mk(3_000_000, None, StopKind::Unexpected),
    ];
    for r in &rows {
        tdb.db.insert_restart(r).await.unwrap();
    }
    let got = tdb.db.restarts_since(1_500_000).await.unwrap();
    assert_eq!(got, vec![rows[2].clone(), rows[1].clone()]);
    let all = tdb.db.restarts_since(0).await.unwrap();
    assert_eq!(all, vec![rows[2].clone(), rows[1].clone(), rows[0].clone()]);
    assert_eq!(tdb.db.first_start().await.unwrap(), Some(1_000_000));

    tdb.drop_db().await;
}

#[tokio::test]
async fn first_start_is_none_on_an_empty_log() {
    let Some(tdb) = fresh_db().await else { return };
    assert_eq!(tdb.db.first_start().await.unwrap(), None);
    tdb.drop_db().await;
}

#[tokio::test]
async fn purge_removes_only_system_rows() {
    use nidoo_api::system::{RestartView, StopKind};
    let Some(tdb) = fresh_db().await else { return };
    let old = 1_000_000_000_000; // micros = 1_000_000 s; cutoff below is 2_000_000 s
    let rows = Rows {
        values: vec![
            value_row("system_cpu", old, 10.0),
            value_row("light_x", old, 1.0),
            value_row("system_cpu", 4_000_000_000_000, 20.0),
        ],
        availability: vec![
            availability_row("system_cpu", old, Availability::Online),
            availability_row("light_x", old, Availability::Online),
        ],
    };
    tdb.db.insert(&rows).await.unwrap();
    tdb.db
        .insert_restart(&RestartView {
            started_at: 1_000_000,
            down_secs: None,
            stop: StopKind::First,
        })
        .await
        .unwrap();

    let removed = tdb.db.purge_system_before(2_000_000).await.unwrap();
    assert_eq!(removed, 3, "old system state + availability + restart");

    let left: Vec<String> = sqlx::query_scalar(
        "SELECT entity_id || ':' || coalesce(num::text, '') FROM entity_state ORDER BY time",
    )
    .fetch_all(tdb.db.pool())
    .await
    .unwrap();
    assert_eq!(left, vec!["light_x:1", "system_cpu:20"]);
    let avail: Vec<String> = sqlx::query_scalar("SELECT entity_id FROM entity_availability")
        .fetch_all(tdb.db.pool())
        .await
        .unwrap();
    assert_eq!(avail, vec!["light_x"]);
    assert_eq!(tdb.db.first_start().await.unwrap(), None);

    tdb.drop_db().await;
}
