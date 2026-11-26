use std::time::Duration;

use nidoo_climate::doc::{Window, ZoneSettings};
use nidoo_core::command::Publish;
use tokio::sync::mpsc;

use super::*;
use crate::config::Config;

fn hub() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let mut cfg = Config::default();
    cfg.climate.enabled = true;
    let (tx, rx) = mpsc::channel(64);
    (Hub::new(&cfg, tx), rx)
}

fn proposal(id: &str, kind: ProposalKind, changes: &[&str]) -> Proposal {
    Proposal {
        id: id.to_owned(),
        zone: "living".to_owned(),
        kind,
        created: 0,
        window: Window {
            from: 0,
            to: 1,
            days: 28,
        },
        rules: Rules::default(),
        changes: changes.iter().map(|c| (*c).to_owned()).collect(),
        fingerprint: id.to_owned(),
    }
}

/// Lets the push service accept alerts and returns the alerts as the hub sees them after
/// the broker echoes the history back.
async fn settle(hub: &Hub) {
    hub.on_connected();
    hub.on_suback();
    tokio::time::sleep(Duration::from_secs(5)).await;
}

fn echo(hub: &Hub, rx: &mut mpsc::Receiver<Publish>) {
    while let Ok(p) = rx.try_recv() {
        if p.topic == hub.push().history_topic() {
            hub.ingest_message(&p.topic, p.payload.as_bytes(), false);
        }
    }
}

fn alerts_of(hub: &Hub) -> Vec<nidoo_api::AlertView> {
    hub.alerts()
        .into_iter()
        .filter(|a| a.id.starts_with(ALERT_PREFIX))
        .collect()
}

#[tokio::test(start_paused = true)]
async fn proposal_push_has_buttons_and_daily_limit() {
    let (hub, mut rx) = hub();
    settle(&hub).await;
    let t = 1_000_000;

    let first = proposal(
        "p-1",
        ProposalKind::Initial,
        &["Base: 17.0 °C", "New: a", "New: b"],
    );
    publish(&hub, "living", first.clone(), t);
    echo(&hub, &mut rx);
    let alerts = alerts_of(&hub);
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    let alert = &alerts[0];
    assert_eq!(alert.id, "climate-proposal-p-1");
    assert_eq!(alert.title, "Climate: new rules for living");
    assert_eq!(
        alert.body, "Base: 17.0 °C\nNew: a\nReview on the Climate screen.",
        "the first two changes, then the pointer to the screen"
    );
    let buttons: Vec<(&str, &str)> = alert
        .actions
        .iter()
        .map(|a| (a.id.as_str(), a.label.as_str()))
        .collect();
    assert_eq!(buttons, [("approve", "Approve"), ("reject", "Reject")]);
    assert_eq!(serde_json::to_value(alert.severity).unwrap(), "info");
    assert!(!alert.resolved);

    // The same proposal again, and then a newer one, within a day: the document is
    // replaced, nobody is pinged, and the old alert is resolved by the replacement.
    publish(&hub, "living", first, t + 600);
    let second = proposal(
        "p-2",
        ProposalKind::Change,
        &["Changed: weekdays start 07:00 → 07:30"],
    );
    publish(&hub, "living", second.clone(), t + 3_600);
    echo(&hub, &mut rx);
    let alerts = alerts_of(&hub);
    assert_eq!(alerts.len(), 1, "one push per zone per day: {alerts:?}");
    assert!(
        alerts[0].resolved,
        "the replaced proposal's alert is resolved"
    );
    assert_eq!(
        hub.climate_store().zones["living"]
            .proposal
            .as_ref()
            .map(|p| p.id.as_str()),
        Some("p-2")
    );

    // A day after the first push, a new proposal pings again.
    let third = proposal(
        "p-3",
        ProposalKind::Change,
        &["Changed: weekdays end 22:00 → 22:30"],
    );
    publish(&hub, "living", third, t + PUSH_EVERY_S);
    echo(&hub, &mut rx);
    let alerts = alerts_of(&hub);
    let fresh = alerts
        .iter()
        .find(|a| a.id == "climate-proposal-p-3")
        .unwrap();
    assert_eq!(fresh.title, "Climate: rule changes for living");
    assert!(!fresh.resolved);
}

#[tokio::test]
async fn learn_status_from_the_run_reaches_the_view() {
    let (hub, _rx) = hub();
    let (history, _in) = crate::history::History::detached(8, chrono_tz::UTC);
    hub.set_history(history);
    hub.ingest_message(
        "nidoo/climate/zone/attic/settings",
        serde_json::to_string(&ZoneSettings {
            state: nidoo_climate::doc::ZoneState::Active,
            ..ZoneSettings::default()
        })
        .unwrap()
        .as_bytes(),
        true,
    );
    let learn = |hub: &Hub| hub.climate_view().unwrap().zones[0].learn.clone();
    assert_eq!(learn(&hub), LearnStatus::UpToDate);
    for status in [
        LearnStatus::NotEnough {
            days: 6,
            needed: 10,
        },
        LearnStatus::Rejected,
    ] {
        hub.climate_runtime()
            .learn_status
            .insert("attic".to_owned(), status.clone());
        assert_eq!(learn(&hub), status);
    }
    // A pending proposal wins over whatever the last run said.
    hub.ingest_message(
        "nidoo/climate/zone/attic/proposal",
        serde_json::to_string(&proposal("p-1", ProposalKind::Initial, &[]))
            .unwrap()
            .as_bytes(),
        true,
    );
    assert_eq!(learn(&hub), LearnStatus::Proposed);
}

#[tokio::test]
async fn a_zone_in_eco_is_not_learned() {
    use nidoo_climate::doc::{RoomEco, ZoneState};
    let (hub, _rx) = hub();
    let (history, _in) = crate::history::History::detached(8, chrono_tz::UTC);
    hub.set_history(history);
    hub.ingest_message(
        "nidoo/climate/zone/attic/settings",
        settings_doc(ZoneState::Active).as_bytes(),
        true,
    );
    let now = clock::now();
    let eco = |until| {
        serde_json::to_string(&RoomEco {
            set_at: now - 60,
            until,
        })
        .unwrap()
    };
    let topic = "nidoo/climate/zone/attic/eco";
    hub.ingest_message(topic, eco(now + 3_600).as_bytes(), true);
    let learn = |hub: &Hub| hub.climate_view().unwrap().zones[0].learn.clone();
    assert_eq!(learn(&hub), LearnStatus::Eco);
    assert_eq!(
        learning::learn_zone(&hub, "attic", now).await,
        LearnStatus::Eco
    );
    assert_eq!(
        hub.climate_runtime().learn_status["attic"],
        LearnStatus::Eco
    );
    // An eco that has ended no longer holds learning back.
    hub.ingest_message(topic, eco(now - 1).as_bytes(), true);
    assert_eq!(learn(&hub), LearnStatus::UpToDate);
    assert_ne!(
        learning::learn_zone(&hub, "attic", now).await,
        LearnStatus::Eco
    );
}

#[tokio::test(start_paused = true)]
async fn proposal_reply_follows_push_prefix() {
    let mut cfg = Config::default();
    cfg.climate.enabled = true;
    cfg.push.prefix = "custom/push".to_owned();
    let (tx, mut rx) = mpsc::channel(64);
    let hub = Hub::new(&cfg, tx);
    settle(&hub).await;
    publish(
        &hub,
        "living",
        proposal("p-1", ProposalKind::Initial, &["Base: 17.0 °C"]),
        1_000_000,
    );
    echo(&hub, &mut rx);
    assert_eq!(
        alerts_of(&hub).len(),
        1,
        "the history on custom/push came back"
    );
}

#[tokio::test(start_paused = true)]
async fn a_new_fingerprint_under_the_same_id_replaces_the_alert() {
    let (hub, mut rx) = hub();
    settle(&hub).await;
    let first = proposal("p-1", ProposalKind::Initial, &["Base: 17.0 °C"]);
    publish(&hub, "living", first.clone(), 1_000_000);
    let mut again = first;
    again.fingerprint = "other".to_owned();
    publish(&hub, "living", again, 1_000_010);
    echo(&hub, &mut rx);
    let alerts = alerts_of(&hub);
    assert_eq!(alerts.len(), 1);
    assert!(alerts[0].resolved, "the old rules' alert was cancelled");
}

/// The retained push history of a hub that pushed `alert_id` at `sent_at`, delivered as
/// the broker does after a restart.
fn ingest_pushed(hub: &Hub, alert_id: &str, sent_at: u64) {
    let history = json!([{
        "v": 1, "id": alert_id, "channel": "climate", "severity": "info",
        "title": "Climate: new rules for living", "body": "", "actions": [],
        "silent": false, "source": "climate", "sent_at": sent_at,
    }]);
    hub.ingest_message("nidoo/push/history", history.to_string().as_bytes(), true);
}

/// A pending proposal, as the broker delivers it after a restart.
fn ingest_pending(hub: &Hub, id: &str, created: u64) {
    let mut old = proposal(id, ProposalKind::Initial, &[]);
    old.created = created;
    hub.ingest_message(
        "nidoo/climate/zone/living/proposal",
        serde_json::to_string(&old).unwrap().as_bytes(),
        true,
    );
}

#[test]
fn a_proposal_alert_names_its_zone() {
    assert_eq!(
        alert_zone("climate-proposal-p-20261029-080000-living"),
        Some("living")
    );
    assert_eq!(
        alert_zone("climate-proposal-p-20261029-080000-x-living"),
        Some("x-living")
    );
    assert_eq!(alert_zone("climate-proposal-p-1"), None);
    assert_eq!(alert_zone("climate-safety-living-frost"), None);
}

#[tokio::test(start_paused = true)]
async fn the_daily_limit_survives_a_restart() {
    let (hub, mut rx) = hub();
    let now = clock::now();
    // A proposal was pushed an hour ago, before the restart; a later one replaced it
    // without a push. Only the push history remembers when the user was told.
    ingest_pushed(
        &hub,
        "climate-proposal-p-20261029-080000-living",
        now - 3_600,
    );
    settle(&hub).await;
    ingest_pending(&hub, "p-20261029-084000-living", now - 1_200);
    publish(
        &hub,
        "living",
        proposal("p-new", ProposalKind::Change, &[]),
        now,
    );
    echo(&hub, &mut rx);
    assert!(
        alerts_of(&hub)
            .iter()
            .all(|a| a.id != "climate-proposal-p-new"),
        "no second push within a day"
    );
}

#[tokio::test(start_paused = true)]
async fn a_pending_proposal_never_pushed_does_not_hold_the_push() {
    let (hub, mut rx) = hub();
    let now = clock::now();
    // Another zone's push and a pending proposal of this zone that was never pushed.
    ingest_pushed(
        &hub,
        "climate-proposal-p-20261029-080000-bedroom",
        now - 3_600,
    );
    settle(&hub).await;
    ingest_pending(&hub, "p-20261029-084000-living", now - 1_200);
    publish(
        &hub,
        "living",
        proposal("p-new", ProposalKind::Change, &[]),
        now,
    );
    echo(&hub, &mut rx);
    assert!(
        alerts_of(&hub)
            .iter()
            .any(|a| a.id == "climate-proposal-p-new"),
        "pushed: {:?}",
        alerts_of(&hub)
    );
}

#[tokio::test]
async fn history_off_runs_clear_a_stale_status() {
    let (hub, _rx) = hub();
    hub.climate_runtime()
        .learn_status
        .insert("living".to_owned(), LearnStatus::Rejected);
    // No history configured.
    assert_eq!(
        learning::learn_zone(&hub, "living", 1_000).await,
        LearnStatus::HistoryOff
    );
    assert_eq!(
        hub.climate_runtime().learn_status["living"],
        LearnStatus::HistoryOff
    );
    // Configured but unreachable.
    let (history, _in) = crate::history::History::detached(8, chrono_tz::UTC);
    hub.set_history(history);
    hub.climate_runtime().learn_status.insert(
        "living".to_owned(),
        LearnStatus::NotEnough {
            days: 1,
            needed: 10,
        },
    );
    assert_eq!(
        learning::learn_zone(&hub, "living", 1_000).await,
        LearnStatus::HistoryOff
    );
    assert_eq!(
        hub.climate_runtime().learn_status["living"],
        LearnStatus::HistoryOff
    );
}

fn settings_doc(state: nidoo_climate::doc::ZoneState) -> String {
    serde_json::to_string(&ZoneSettings {
        state,
        ..ZoneSettings::default()
    })
    .unwrap()
}

#[tokio::test]
async fn a_zone_leaving_off_is_learned_at_once() {
    use nidoo_climate::doc::ZoneState;
    let (hub, _rx) = hub();
    let topic = "nidoo/climate/zone/attic/settings";
    hub.ingest_message(topic, settings_doc(ZoneState::Off).as_bytes(), true);
    hub.climate_runtime()
        .learn_status
        .insert("attic".to_owned(), LearnStatus::Rejected);
    // Still off: nothing runs.
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        hub.climate_runtime().learn_status["attic"],
        LearnStatus::Rejected
    );
    hub.ingest_message(topic, settings_doc(ZoneState::Pending).as_bytes(), true);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        hub.climate_runtime().learn_status["attic"],
        LearnStatus::HistoryOff,
        "the run replaced the stale status"
    );
    assert!(hub.climate_runtime().learning_now.is_empty());
}

#[tokio::test]
async fn a_zone_already_being_learned_is_skipped() {
    let (hub, _rx) = hub();
    hub.climate_runtime()
        .learning_now
        .insert("attic".to_owned());
    learning::spawn_zone_learning(&hub, "attic");
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!hub.climate_runtime().learn_status.contains_key("attic"));
    assert!(
        hub.climate_runtime().learning_now.contains("attic"),
        "not ours to clear"
    );
    hub.climate_runtime().learning_now.clear();
    learning::spawn_zone_learning(&hub, "attic");
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(hub.climate_runtime().learn_status.contains_key("attic"));
    assert!(
        hub.climate_runtime().learning_now.is_empty(),
        "the guard clears it"
    );
}
