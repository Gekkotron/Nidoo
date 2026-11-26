use std::sync::Arc;

use nidoo_api::{ConnectionStatus, SupervisorSettings};
use nidoo_core::model::Availability;
use nidoo_watchdog::supervisor::{Baseline, Check, Ctx, Evidence, Problem, Reading, Supervisor};

use super::*;
use crate::config::Config;
use crate::history::db::Db;
use crate::hub::Hub;
use crate::setup::sections::Sections;
use crate::setup::{HubFile, Mode, RestartHook, SetupCtx};
use crate::supervisor::entity_of_alert;

const NOW: u64 = 1_800_000_000;
const H: u64 = 3600;

fn hub_with(db: Option<Db>, dir: &std::path::Path) -> Arc<Hub> {
    let ctx = Arc::new(SetupCtx::new(
        dir.to_path_buf(),
        Mode::Run,
        HubFile::default(),
        db,
        Sections::default(),
        RestartHook::Record(Arc::default()),
    ));
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let hub = Hub::new(&Config::default(), tx);
    hub.set_setup(ctx);
    hub
}

fn reading(id: &str) -> Reading {
    Reading {
        entity_id: id.into(),
        name: format!("Sensor {id}"),
        device_class: Some("temperature".into()),
        unit: Some("°C".into()),
        value: Some(15.0),
        last_change: Some(NOW - 14 * H),
        last_report: Some(NOW - 60),
        unavailable_since: None,
        baseline: Some(Baseline {
            max_gap: 3 * H,
            changes: 400,
            rest: None,
        }),
    }
}

fn silent(id: &str) -> Reading {
    Reading {
        last_change: Some(NOW - 10 * H),
        last_report: Some(NOW - 10 * H),
        ..reading(id)
    }
}

fn ctx(paused: bool) -> Ctx {
    Ctx {
        now: NOW,
        armed_at: NOW - 30 * 24 * H,
        factor: 2.0,
        silent_paused: paused,
    }
}

fn stuck_problem(id: &str, opened_at: u64) -> Problem {
    Problem {
        entity_id: id.into(),
        name: format!("Sensor {id}"),
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

fn loaded(hub: &Hub, open: Vec<Problem>) {
    let mut rt = hub.supervisor_state();
    rt.engine = Supervisor::new(open);
    rt.loaded = true;
}

#[tokio::test]
async fn restart_does_not_resend_open_problems() {
    let tmp = tempfile::tempdir().unwrap();
    let hub = hub_with(None, tmp.path());
    loaded(&hub, vec![stuck_problem("sensor.tank", NOW - 5 * H)]);
    let (step, sent) = hub.supervisor_step(&[reading("sensor.tank")], &ctx(false));
    assert!(step.opened.is_empty() && step.closed.is_empty());
    assert!(sent.is_empty());
    assert_eq!(hub.supervisor_view().problems.len(), 1);
}

#[tokio::test]
async fn recovered_while_down_pushes_once() {
    let tmp = tempfile::tempdir().unwrap();
    let hub = hub_with(None, tmp.path());
    loaded(&hub, vec![stuck_problem("sensor.tank", NOW - 5 * H)]);
    let moved = Reading {
        last_change: Some(NOW - 2 * H),
        ..reading("sensor.tank")
    };
    let (_, sent) = hub.supervisor_step(std::slice::from_ref(&moved), &ctx(false));
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["loc"]["key"], "supervisor.recovered");
    let (_, again) = hub.supervisor_step(&[moved], &ctx(false));
    assert!(again.is_empty());
}

#[tokio::test]
async fn bridge_offline_pauses_silent() {
    let tmp = tempfile::tempdir().unwrap();
    let hub = hub_with(None, tmp.path());
    let readings: Vec<Reading> = (0..30).map(|i| silent(&format!("sensor.s{i}"))).collect();
    let (_, sent) = hub.supervisor_step(&readings, &ctx(true));
    assert!(sent.is_empty());
    assert_eq!(hub.supervisor_view().problems.len(), 0);
    let (_, sent) = hub.supervisor_step(&readings, &ctx(false));
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["loc"]["key"], "supervisor.group");
    assert_eq!(hub.supervisor_view().problems.len(), 30);
}

#[test]
fn silent_is_paused_by_the_link_or_an_offline_bridge() {
    let online = ConnectionStatus::Online;
    let down = ConnectionStatus::Offline {
        reason: "gone".into(),
    };
    assert!(!silent_paused(&online, Availability::Online));
    assert!(silent_paused(&online, Availability::Offline));
    // A house without Zigbee2MQTT never hears from a bridge.
    assert!(!silent_paused(&online, Availability::Unknown));
    assert!(silent_paused(&down, Availability::Online));
}

#[tokio::test]
async fn db_down_skips_the_run() {
    let tmp = tempfile::tempdir().unwrap();
    let hub = hub_with(None, tmp.path());
    loaded(&hub, vec![stuck_problem("sensor.tank", NOW - 5 * H)]);
    hub.supervisor_tick(NOW).await;
    let view = hub.supervisor_view();
    assert!(!view.available);
    assert_eq!(view.problems.len(), 1, "nothing closes");

    // A database that does not answer: the run is skipped, the problem stays.
    let cfg = crate::config::HistoryConfig {
        url: "postgres://u:p@127.0.0.1:1/x".to_owned(),
        ..crate::config::HistoryConfig::default()
    };
    hub.set_history(crate::history::History::start(&cfg, hub.zone()).expect("a lazy pool"));
    hub.supervisor_tick(NOW).await;
    let view = hub.supervisor_view();
    assert!(view.available);
    assert_eq!(view.problems.len(), 1, "nothing closes");
    assert!(hub.supervisor_state().baselines_at.is_none());
}

#[tokio::test]
async fn mute_reply_ignores_the_sensor() {
    let tmp = tempfile::tempdir().unwrap();
    let hub = hub_with(None, tmp.path());
    let p = stuck_problem("sensor.tank", NOW - 5 * H);
    let alert_id = p.alert_key();
    loaded(&hub, vec![p, stuck_problem("sensor.other", NOW - 5 * H)]);
    let reply = serde_json::json!({"id": alert_id, "action": "mute"}).to_string();
    hub.on_supervisor_reply(reply.as_bytes());
    let view = hub.supervisor_view();
    assert_eq!(view.settings.ignored, vec!["sensor.tank".to_string()]);
    assert_eq!(view.problems.len(), 1);
    assert_eq!(view.problems[0].entity_id, "sensor.other");

    // A malformed or grouped alert id, or another action, changes nothing.
    hub.on_supervisor_reply(br#"{"id": "supervisor-group-silent-1", "action": "mute"}"#);
    hub.on_supervisor_reply(br#"{"id": "supervisor-bogus", "action": "mute"}"#);
    let other = stuck_problem("sensor.other", NOW - 5 * H).alert_key();
    hub.on_supervisor_reply(
        serde_json::json!({"id": other, "action": "open"})
            .to_string()
            .as_bytes(),
    );
    assert_eq!(hub.supervisor_view().problems.len(), 1);

    // The muted sensor is left out of the next run: no recovered push for it.
    let (_, sent) = hub.supervisor_step(&[], &ctx(false));
    assert!(sent.is_empty());
}

#[tokio::test]
async fn a_mute_reply_on_the_responses_topic_reaches_the_supervisor() {
    let tmp = tempfile::tempdir().unwrap();
    let hub = hub_with(None, tmp.path());
    let p = stuck_problem("sensor.tank", NOW - 5 * H);
    let reply = serde_json::json!({"id": p.alert_key(), "action": "mute"}).to_string();
    loaded(&hub, vec![p]);
    hub.ingest_message("nidoo/push/responses", reply.as_bytes(), false);
    assert!(hub.supervisor_view().problems.is_empty());
    assert_eq!(
        hub.supervisor_view().settings.ignored,
        vec!["sensor.tank".to_string()]
    );
}

#[test]
fn ignored_sensors_are_not_supervised() {
    let settings = SupervisorSettings {
        ignored: vec!["sensor.b".into()],
        ..SupervisorSettings::default()
    };
    let readings = vec![reading("sensor.a"), reading("sensor.b")];
    let kept = without_ignored(readings, &settings);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].entity_id, "sensor.a");
}

#[tokio::test]
async fn disable_clears_problems() {
    let tmp = tempfile::tempdir().unwrap();
    let hub = hub_with(None, tmp.path());
    loaded(&hub, vec![stuck_problem("sensor.tank", NOW - 5 * H)]);
    let off = SupervisorSettings {
        enabled: false,
        ..SupervisorSettings::default()
    };
    // Without a database the setting cannot be stored: nothing changes.
    assert!(hub.set_supervisor(off.clone()).await.is_err());
    assert_eq!(hub.supervisor_view().problems.len(), 1);

    let cleared = hub.apply_supervisor_settings(off);
    assert_eq!(cleared.len(), 1);
    let view = hub.supervisor_view();
    assert!(view.problems.is_empty());
    assert!(!view.settings.enabled);
    // Off: a run sends nothing, whatever it sees.
    hub.supervisor_tick(NOW).await;
    assert!(hub.supervisor_view().problems.is_empty());
}

#[tokio::test]
async fn adding_to_ignored_drops_its_problems() {
    let tmp = tempfile::tempdir().unwrap();
    let hub = hub_with(None, tmp.path());
    loaded(&hub, vec![stuck_problem("sensor.tank", NOW - 5 * H)]);
    let muted = hub.apply_supervisor_settings(SupervisorSettings {
        ignored: vec!["sensor.tank".into()],
        ..SupervisorSettings::default()
    });
    assert_eq!(muted.len(), 1);
    assert!(hub.supervisor_view().problems.is_empty());
}

#[tokio::test]
async fn reports_set_the_clock_only_in_the_current_link_epoch() {
    let tmp = tempfile::tempdir().unwrap();
    let hub = hub_with(None, tmp.path());
    let epoch = hub.link_epoch();
    hub.supervisor_touched("sensor.a", epoch, true, NOW);
    hub.supervisor_touched("sensor.b", epoch, false, NOW);
    hub.supervisor_touched("sensor.c", epoch + 1, true, NOW);
    let rt = hub.supervisor_state();
    assert_eq!(rt.last_report.get("sensor.a"), Some(&NOW));
    assert!(!rt.last_report.contains_key("sensor.b"), "not read");
    assert!(!rt.last_report.contains_key("sensor.c"), "stale epoch");
}

#[tokio::test]
async fn a_run_respects_settings_changed_while_it_ran() {
    let tmp = tempfile::tempdir().unwrap();
    let hub = hub_with(None, tmp.path());
    loaded(&hub, Vec::new());
    // Muted (or switched off) after the run read its settings: the step uses the current
    // ones.
    hub.apply_supervisor_settings(SupervisorSettings {
        ignored: vec!["sensor.tank".into()],
        ..SupervisorSettings::default()
    });
    let (step, sent) = hub.supervisor_step(&[reading("sensor.tank")], &ctx(false));
    assert!(step.opened.is_empty() && sent.is_empty());
    hub.apply_supervisor_settings(SupervisorSettings {
        enabled: false,
        ..SupervisorSettings::default()
    });
    let (step, sent) = hub.supervisor_step(&[reading("sensor.a")], &ctx(false));
    assert!(step.opened.is_empty() && sent.is_empty());
    assert!(hub.supervisor_view().problems.is_empty());
}

#[tokio::test]
async fn the_bridge_coming_back_restarts_the_clocks() {
    let tmp = tempfile::tempdir().unwrap();
    let hub = hub_with(None, tmp.path());
    let mut rt = hub.supervisor_state();
    rt.armed_at = NOW - 30 * 24 * H;
    rt.observe_bridge(Availability::Online, NOW - 9 * H);
    assert_eq!(rt.armed_at, NOW - 30 * 24 * H);
    rt.observe_bridge(Availability::Offline, NOW - 8 * H);
    rt.observe_bridge(Availability::Offline, NOW - 7 * H);
    assert_eq!(rt.armed_at, NOW - 30 * 24 * H, "offline only pauses");
    rt.observe_bridge(Availability::Online, NOW);
    assert_eq!(rt.armed_at, NOW);
    rt.observe_bridge(Availability::Online, NOW + H);
    assert_eq!(rt.armed_at, NOW, "once per return");
}

#[tokio::test]
async fn the_retained_replay_after_a_reconnect_is_not_a_report() {
    let tmp = tempfile::tempdir().unwrap();
    let hub = hub_with(None, tmp.path());
    let epoch = hub.link_epoch();
    hub.supervisor_rearm(NOW);
    hub.supervisor_touched("sensor.a", epoch, true, NOW + 5);
    assert!(!hub.supervisor_state().last_report.contains_key("sensor.a"));
    hub.supervisor_touched("sensor.a", epoch, true, NOW + 60);
    assert_eq!(
        hub.supervisor_state().last_report.get("sensor.a"),
        Some(&(NOW + 60))
    );
}

#[tokio::test]
async fn mute_works_before_the_problems_are_loaded() {
    let tmp = tempfile::tempdir().unwrap();
    let hub = hub_with(None, tmp.path());
    // Right after a restart nothing is loaded yet; the id itself names the sensor.
    let id = stuck_problem("sensor.tank-2", NOW - 5 * H).alert_key();
    hub.on_supervisor_reply(
        serde_json::json!({"id": id, "action": "mute"})
            .to_string()
            .as_bytes(),
    );
    assert_eq!(
        hub.supervisor_view().settings.ignored,
        vec!["sensor.tank-2".to_string()]
    );
}

#[test]
fn alert_ids_name_their_sensor() {
    assert_eq!(
        entity_of_alert("supervisor-stuck-sensor.tank-2-1800000000"),
        Some("sensor.tank-2")
    );
    assert_eq!(entity_of_alert("supervisor-silent-x-1"), Some("x"));
    assert_eq!(entity_of_alert("supervisor-group-silent-1"), None);
    assert_eq!(entity_of_alert("supervisor-recovered-stuck-x-1"), None);
    assert_eq!(entity_of_alert("supervisor-stuck--1"), None);
    assert_eq!(entity_of_alert("supervisor-stuck-x-abc"), None);
    assert_eq!(entity_of_alert("alarm-x"), None);
}
