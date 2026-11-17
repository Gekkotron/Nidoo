//! The hourly supervisor run and the live report clocks.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use nidoo_api::{ConnectionStatus, SupervisorSettings};
use nidoo_core::model::{Availability, Kind, StateValue};
use nidoo_watchdog::supervisor::{Baseline, Ctx, Reading, Step, Supervisor, alerts, baseline};
use serde_json::Value;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

use super::{sensitivity, store};
use crate::clock;
use crate::hub::Hub;

/// Seconds between two runs; the first run waits as long after the start.
pub const RUN_SECS: u64 = 3600;
/// The baseline looks back this far.
const WINDOW_SECS: u64 = 14 * 86_400;
/// The baselines are recomputed this often.
const BASELINE_EVERY: u64 = 86_400;
/// Closed problems are kept this long (they keep past outages out of the baselines).
const KEEP_CLOSED: u64 = 30 * 86_400;

/// The silent check waits while the broker link is down or the Zigbee2MQTT bridge says it is
/// offline: that outage has its own push. A bridge never heard from (no Zigbee2MQTT) does not
/// pause it.
pub fn silent_paused(link: &ConnectionStatus, bridge: Availability) -> bool {
    !matches!(link, ConnectionStatus::Online) || bridge == Availability::Offline
}

/// `readings` without the ignored sensors.
pub(crate) fn without_ignored(readings: Vec<Reading>, s: &SupervisorSettings) -> Vec<Reading> {
    readings
        .into_iter()
        .filter(|r| !s.ignored.contains(&r.entity_id))
        .collect()
}

/// A supervised sensor as the registry has it.
struct Sensor {
    id: String,
    name: String,
    device_class: Option<String>,
    unit: Option<String>,
    value: Option<f64>,
    offline: bool,
}

impl Hub {
    /// Starts the supervisor: keeps the report clocks, and runs the checks an hour after the
    /// start, then every hour. `None` without a database.
    pub fn start_supervisor(self: &Arc<Self>) -> Option<JoinHandle<()>> {
        self.supervisor_db()?;
        let hub = Arc::clone(self);
        let mut touched = self.subscribe_state_touched();
        let mut link = self.connection();
        let every = Duration::from_secs(RUN_SECS);
        Some(tokio::spawn(async move {
            // The open problems show on the page and answer a Mute from the start.
            if let Some(db) = hub.supervisor_db() {
                hub.supervisor_load(&db).await;
            }
            let mut next = tokio::time::Instant::now() + every;
            let mut running: Option<JoinHandle<()>> = None;
            loop {
                tokio::select! {
                    msg = touched.recv() => match msg {
                        Ok(t) => hub.supervisor_touched(&t.id, t.epoch, t.read, clock::now()),
                        Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => break,
                    },
                    changed = link.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        let online = matches!(*link.borrow(), ConnectionStatus::Online);
                        if online {
                            hub.supervisor_rearm(clock::now());
                        }
                    }
                    () = tokio::time::sleep_until(next) => {
                        next = tokio::time::Instant::now() + every;
                        // A run still going (a slow database) is not doubled.
                        if running.as_ref().is_none_or(JoinHandle::is_finished) {
                            let run = Arc::clone(&hub);
                            running = Some(tokio::spawn(async move {
                                run.supervisor_tick(clock::now()).await;
                            }));
                        }
                    }
                }
            }
        }))
    }

    /// A state message for `id`: a read one in the current link epoch sets its report clock.
    /// The retained replay right after the link comes online is not a report.
    pub(crate) fn supervisor_touched(&self, id: &str, epoch: u64, read: bool, now: u64) {
        if read && epoch == self.link_epoch() {
            let mut rt = self.supervisor_state();
            if now >= rt.online_at.saturating_add(super::SETTLE_SECS) {
                rt.last_report.insert(id.to_owned(), now);
            }
        }
    }

    /// The broker link is back: the clocks of a new link epoch start now.
    pub(crate) fn supervisor_rearm(&self, now: u64) {
        let epoch = self.link_epoch();
        let mut rt = self.supervisor_state();
        rt.online_at = now;
        if rt.epoch != epoch {
            rt.epoch = epoch;
            rt.armed_at = now;
        }
    }

    /// One run at `now`: load the open problems once, refresh the baselines once a day, read
    /// the sensors, step, push, store. A database that does not answer skips the run and
    /// keeps every problem as it is.
    pub async fn supervisor_tick(&self, now: u64) {
        let settings = self.supervisor_state().settings.clone();
        if !settings.enabled {
            return;
        }
        let Some(db) = self.supervisor_db() else {
            return;
        };
        if !self.supervisor_load(&db).await {
            tracing::warn!("supervisor run skipped: the database did not answer");
            return;
        }

        let sensors = self.supervised_sensors(&settings);
        let ids: Vec<String> = sensors.iter().map(|s| s.id.clone()).collect();
        let due = self
            .supervisor_state()
            .baselines_at
            .is_none_or(|at| now.saturating_sub(at) >= BASELINE_EVERY);
        if due {
            let since = now.saturating_sub(WINDOW_SECS);
            let stats = store::gap_stats(&db, &ids, since).await;
            let excluded = store::intervals(&db, since).await;
            let (stats, excluded) = match (stats, excluded) {
                (Ok(stats), Ok(excluded)) => (stats, excluded),
                (Err(error), _) | (_, Err(error)) => {
                    tracing::warn!(%error, "supervisor run skipped: the database did not answer");
                    return;
                }
            };
            let baselines: HashMap<String, Baseline> = stats
                .iter()
                .filter_map(|s| {
                    let ex = excluded.get(&s.entity_id).map_or(&[][..], Vec::as_slice);
                    baseline(s, now, ex).map(|b| (s.entity_id.clone(), b))
                })
                .collect();
            let learning = ids.iter().filter(|id| !baselines.contains_key(*id)).count();
            let mut rt = self.supervisor_state();
            rt.baselines = baselines;
            rt.baselines_at = Some(now);
            rt.learning = u32::try_from(learning).unwrap_or(u32::MAX);
        }
        let last_changes = match store::last_changes(&db, &ids).await {
            Ok(last) => last,
            Err(error) => {
                tracing::warn!(%error, "supervisor run skipped: the database did not answer");
                return;
            }
        };

        let link = self.connection().borrow().clone();
        let bridge = self.read().bridge().state;
        let (readings, ctx) = {
            let mut rt = self.supervisor_state();
            let offline: HashSet<&str> = sensors
                .iter()
                .filter(|s| s.offline)
                .map(|s| s.id.as_str())
                .collect();
            rt.unavailable_since
                .retain(|id, _| offline.contains(id.as_str()));
            for id in offline {
                rt.unavailable_since.entry(id.to_owned()).or_insert(now);
            }
            let readings: Vec<Reading> = sensors
                .into_iter()
                .map(|s| Reading {
                    last_change: last_changes.get(&s.id).copied(),
                    last_report: rt.last_report.get(&s.id).copied(),
                    unavailable_since: rt.unavailable_since.get(&s.id).copied(),
                    baseline: rt.baselines.get(&s.id).copied(),
                    entity_id: s.id,
                    name: s.name,
                    device_class: s.device_class,
                    unit: s.unit,
                    value: s.value,
                })
                .collect();
            rt.observe_bridge(bridge, now);
            let ctx = Ctx {
                now,
                armed_at: rt.armed_at,
                factor: sensitivity(settings.sensitivity).factor(),
                silent_paused: silent_paused(&link, bridge),
            };
            (without_ignored(readings, &settings), ctx)
        };

        let (step, _) = self.supervisor_step(&readings, &ctx);
        for p in &step.opened {
            if let Err(error) = store::insert(&db, p).await {
                tracing::warn!(%error, entity = %p.entity_id, "could not store the problem");
            }
        }
        for c in &step.closed {
            if let Err(error) = store::close(&db, &c.problem, c.at).await {
                tracing::warn!(%error, entity = %c.problem.entity_id, "could not close the problem");
            }
        }
        let purge = {
            let mut rt = self.supervisor_state();
            let due = rt
                .purged_at
                .is_none_or(|at| now.saturating_sub(at) >= BASELINE_EVERY);
            if due {
                rt.purged_at = Some(now);
            }
            due
        };
        if purge && let Err(error) = store::purge(&db, now.saturating_sub(KEEP_CLOSED)).await {
            tracing::warn!(%error, "could not purge the closed problems");
        }
    }

    /// Loads the stored open problems once; true when they are loaded. Turned off meanwhile:
    /// nothing is installed (the rows were deleted).
    pub(crate) async fn supervisor_load(&self, db: &crate::history::db::Db) -> bool {
        if self.supervisor_state().loaded {
            return true;
        }
        match store::load_open(db).await {
            Ok(open) => {
                let mut rt = self.supervisor_state();
                if rt.settings.enabled && !rt.loaded {
                    let ignored = rt.settings.ignored.clone();
                    rt.engine = Supervisor::new(
                        open.into_iter()
                            .filter(|p| !ignored.contains(&p.entity_id))
                            .collect(),
                    );
                    rt.loaded = true;
                }
                true
            }
            Err(error) => {
                tracing::warn!(%error, "could not load the open problems");
                false
            }
        }
    }

    /// Steps the engine with `readings` and sends the resulting pushes (per the settings).
    /// Returns the step, with `alert_id` set on what was announced, and the alerts sent.
    pub(crate) fn supervisor_step(&self, readings: &[Reading], ctx: &Ctx) -> (Step, Vec<Value>) {
        let (step, sent) = {
            let mut rt = self.supervisor_state();
            // The settings now, not the ones the run read before its database round: a Mute
            // or a switch-off meanwhile stands.
            if !rt.settings.enabled {
                return (Step::default(), Vec::new());
            }
            let readings = without_ignored(readings.to_vec(), &rt.settings);
            let mut step = rt.engine.step(&readings, ctx);
            let sent = alerts(
                &mut step,
                rt.settings.notify,
                rt.settings.notify_recovered,
                ctx.now,
            );
            (step, sent)
        };
        for alert in &sent {
            self.raise_system(Some(alert.clone()));
        }
        (step, sent)
    }

    /// The registry's numeric sensors (secret ones never). A sensor whose value is no longer
    /// a number stays supervised while it has a baseline or an open problem, so a sensor
    /// sending garbage is not dropped from its own checks.
    fn supervised_sensors(&self, settings: &SupervisorSettings) -> Vec<Sensor> {
        let known: HashSet<String> = {
            let rt = self.supervisor_state();
            rt.baselines
                .keys()
                .cloned()
                .chain(rt.engine.open().map(|p| p.entity_id.clone()))
                .collect()
        };
        let reg = self.read();
        reg.entities()
            .filter(|e| e.kind == Kind::Sensor)
            .filter(|e| !e.capabilities.generic.as_ref().is_some_and(|g| g.secret))
            .filter(|e| !settings.ignored.contains(&e.unique_id))
            .filter_map(|e| {
                let state = reg.state(&e.unique_id);
                let value = state.and_then(|s| match s.value {
                    StateValue::Number(x) if x.is_finite() => Some(x),
                    _ => None,
                });
                if value.is_none() && !known.contains(&e.unique_id) {
                    return None;
                }
                Some(Sensor {
                    id: e.unique_id.clone(),
                    name: e.name.clone(),
                    device_class: e.device_class.clone(),
                    unit: e.unit.clone(),
                    value,
                    offline: state.is_some_and(|s| s.availability == Availability::Offline),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nidoo_api::{ConnectionStatus, SupervisorSettings};
    use nidoo_core::model::Availability;
    use nidoo_watchdog::supervisor::{
        Baseline, Check, Ctx, Evidence, Problem, Reading, Supervisor,
    };

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
}
