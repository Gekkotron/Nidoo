//! The assistant's turn log in the database.
//!
//! Needs `TEST_DATABASE_URL` (see `history_db.rs`); without it each test prints
//! `skipping: TEST_DATABASE_URL unset` and returns.

use crate::common;

use common::db::fresh_db;
use nidoo_assist::view::{ActionTrace, AssistTurn, OutcomeKind, Question, QuestionKind, Trace};

fn turn(id: &str, at: u64) -> AssistTurn {
    AssistTurn {
        id: id.into(),
        at,
        client: "page-1".into(),
        text: "mets le salon à 25".into(),
        simulate: false,
        outcome: OutcomeKind::Confirming,
        reply: "Passer le Salon à 25 °C ?".into(),
        question: Some(Question {
            pending: "p1".into(),
            kind: QuestionKind::Confirm,
            choices: vec![],
        }),
        trace: Trace {
            words: vec!["mets".into(), "le".into(), "salon".into()],
            intent: Some("set_temp".into()),
            room: Some("living".into()),
            gate: Some("hors de 16–23 °C : demander".into()),
            actions: vec![ActionTrace {
                target: "living".into(),
                what: "consigne 25 °C".into(),
                result: "ok".into(),
            }],
            elapsed_us: 42,
            ..Trace::default()
        },
        wrong: false,
    }
}

#[tokio::test]
async fn assist_turn_roundtrip() {
    let Some(tdb) = fresh_db().await else { return };
    let t = turn("a", 1_800_000_000);
    tdb.db.insert_assist_turn(&t).await.unwrap();
    assert_eq!(tdb.db.assist_turns(10).await.unwrap(), vec![t]);
    tdb.drop_db().await;
}

#[tokio::test]
async fn assist_turns_newest_first() {
    let Some(tdb) = fresh_db().await else { return };
    // The same second: insertion order breaks the tie.
    for (id, at) in [("a", 100), ("b", 200), ("c", 200)] {
        tdb.db.insert_assist_turn(&turn(id, at)).await.unwrap();
    }
    let ids: Vec<String> = tdb
        .db
        .assist_turns(10)
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(ids, ["c", "b", "a"]);
    assert_eq!(tdb.db.assist_turns(2).await.unwrap().len(), 2);
    tdb.drop_db().await;
}

#[tokio::test]
async fn mark_wrong_persists() {
    let Some(tdb) = fresh_db().await else { return };
    tdb.db.insert_assist_turn(&turn("a", 100)).await.unwrap();
    assert!(tdb.db.mark_assist_wrong("a").await.unwrap());
    assert!(!tdb.db.mark_assist_wrong("nope").await.unwrap());
    assert!(tdb.db.assist_turns(10).await.unwrap()[0].wrong);
    tdb.drop_db().await;
}

#[tokio::test]
async fn purge_drops_older_than_90_days() {
    let Some(tdb) = fresh_db().await else { return };
    let now = 1_800_000_000;
    let keep = nidoo::system::restarts::KEEP_SECS;
    tdb.db
        .insert_assist_turn(&turn("old", now - keep - 1))
        .await
        .unwrap();
    tdb.db
        .insert_assist_turn(&turn("new", now - keep + 1))
        .await
        .unwrap();
    assert_eq!(tdb.db.purge_assist(now - keep).await.unwrap(), 1);
    let ids: Vec<String> = tdb
        .db
        .assist_turns(10)
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(ids, ["new"]);
    tdb.drop_db().await;
}

/// The turn row is written in the background: a "Faux" tapped right after the reply, before
/// the row exists, must still end up in the database.
#[tokio::test]
async fn wrong_tapped_before_the_row_is_written_is_kept() {
    let Some(tdb) = fresh_db().await else { return };
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let hub = nidoo::hub::Hub::new(&nidoo::config::Config::default(), tx);
    hub.set_history(nidoo::history::History::with_db(
        tdb.db.clone(),
        chrono_tz::UTC,
    ));
    let turn = hub
        .assist(nidoo_assist::view::AssistRequest {
            client: "page-1".into(),
            text: Some("bonjour".into()),
            reply: None,
            simulate: false,
        })
        .await
        .unwrap();
    hub.assist_mark_wrong(&turn.id).await.unwrap();
    let mut kept = false;
    for _ in 0..50 {
        let rows = tdb.db.assist_turns(10).await.unwrap();
        if rows.iter().any(|t| t.id == turn.id && t.wrong) {
            kept = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(kept, "the wrong flag never reached the database");
    tdb.drop_db().await;
}
