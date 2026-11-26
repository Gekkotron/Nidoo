//! The flow engine: turns state changes and manual requests into runs.
//!
//! Boot and reconnect safety is the point of the `known` map. The broker replays every retained
//! state when the hub (re)connects, and those replays must never start a run: a light switching
//! because the hub restarted is worse than a light that misses one trigger. So a trigger only
//! fires on a change between two values the engine itself has seen, and the map is emptied
//! whenever the link drops or the engine falls behind the event stream. Every report of a
//! watched entity (changed or not) missing from the map seeds it silently again, so an
//! identical retained replay arms the entity without firing.
//!
//! Values are tracked per (entity, field): a trigger on a vacuum's battery and one on its state
//! fire independently, each on a change of its own value.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::Timelike;

use nidoo_api::{RunStatus, StepOutcome, StepView, TriggerInfo, WsEvent};
use nidoo_core::flow::answer::{first_number, match_answer, not_understood};
use nidoo_core::flow::climate::until_after;
use nidoo_core::flow::math::{fit_to_entity, parse_formula};
use nidoo_core::flow::sun::is_dark_with_offset;
use nidoo_core::flow::template::{Lang, render};
use nidoo_core::flow::wake_up;
use nidoo_core::flow::{
    ASK_NONE_PORT, AskParams, BACKUP_FAILED_PORT, BACKUP_OK_PORT, BACKUP_PARTIAL_PORT,
    ClimateEcoParams, ClimateHoldParams, ClimateSetpoint, Flow, FlowMode, MathParams, Missing,
    Node, NodeKind, OffPeakEdge, Op, Reading, Scalar, Schedule, TriggerStateParams,
    command_with_number, compare, field_scalar, format_duration, headline_scalar, hold,
    in_time_range, parse_clock, parse_duration, pick_duration, switch_exits, window,
};
use nidoo_core::history::buckets::{Sample, readings_mean};
use nidoo_core::model::{Command, StateValue};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};
use tokio::task::{AbortHandle, JoinSet};

use super::EngineRequest;
use super::labels::switch_detail;
use super::schedule::{Armed, Now, now_in, rearm, timer_clock};
use super::sun::{ArmedSun, latest_sun_fire_in, next_sun_fire};
use super::wake_up_timer::ArmedWakeUp;
use crate::assist::stt::ask::{ASK_LIMIT, AskOutcome};
use crate::clock;
use crate::home::OffPeakNow;
use crate::hub::{Hub, HubError, StateTouched};
use crate::sound::SpeakTarget;
use nidoo_api::ConnectionStatus;

/// How long after the link comes back the off-peak base is seeded when no device report did.
const OFF_PEAK_SETTLE: chrono::Duration = chrono::Duration::seconds(60);

/// How long after the hub starts, or the link comes back, the catch-up check waits: the broker
/// replays the retained flows and markers first, so a fire already recorded is never repeated.
const CATCH_UP_SETTLE: chrono::Duration = chrono::Duration::seconds(10);

/// The port a `limit` node reports when it holds the run back; `execute` follows no edge then.
const LIMITED: &str = "limited";

/// What a finished run task reports: the flow, the run and how it ended.
type Finished = (String, String, RunStatus);

/// A run in progress; at most one per flow.
struct Active {
    run_id: String,
    /// The definition the run started with; a changed definition cancels the run.
    flow: Flow,
    abort: AbortHandle,
    /// The task's id, to find the run again when the task panics.
    task: tokio::task::Id,
}

/// Runs the engine until `requests` closes. Subscribes to the hub's events itself; the hub
/// starts it through `Hub::start_engine`, which subscribes earlier so no event is missed.
pub async fn run(hub: Arc<Hub>, requests: mpsc::UnboundedReceiver<EngineRequest>) {
    let events = hub.subscribe();
    let seen = hub.subscribe_state_touched();
    run_with(hub, events, seen, requests).await;
}

/// [`run`] with the event subscription already taken.
pub(super) async fn run_with(
    hub: Arc<Hub>,
    mut events: broadcast::Receiver<WsEvent>,
    mut seen: broadcast::Receiver<StateTouched>,
    mut requests: mpsc::UnboundedReceiver<EngineRequest>,
) {
    let mut engine = Engine {
        hub: Arc::clone(&hub),
        flows: Vec::new(),
        known: BTreeMap::new(),
        watched: BTreeSet::new(),
        confirmed_unknown: BTreeSet::new(),
        active: BTreeMap::new(),
        keep: BTreeSet::new(),
        tasks: JoinSet::new(),
        zone: hub.zone(),
        now: timer_clock(),
        schedules: BTreeMap::new(),
        silents: BTreeMap::new(),
        silent_epoch: hub.link_epoch(),
        holds: BTreeMap::new(),
        sun_timers: BTreeMap::new(),
        wake_timers: BTreeMap::new(),
        off_peak_base: None,
        off_peak_timer: None,
        off_peak_settle: None,
        catch_up_settle: None,
    };
    if engine.online() {
        engine.catch_up_settle = Some(now_in(&engine.now, engine.zone) + CATCH_UP_SETTLE);
    }
    engine.reload();
    loop {
        let deadline = engine.next_deadline();
        // Requests first: a flow saved before the state it watches changes must be armed
        // before that change is looked at. Finished runs before events: a run that is over must
        // not count as active for a trigger in the same iteration.
        tokio::select! {
            biased;
            request = requests.recv() => {
                let Some(request) = request else { return };
                let mut pending = vec![request];
                // Coalesce a burst of edits into one reload.
                while let Ok(next) = requests.try_recv() {
                    if next != EngineRequest::Reload
                        || pending.last() != Some(&EngineRequest::Reload)
                    {
                        pending.push(next);
                    }
                }
                for request in pending {
                    engine.handle_request(request);
                }
            }
            Some(joined) = engine.tasks.join_next_with_id(), if !engine.tasks.is_empty() => {
                engine.run_finished(joined);
            }
            // Before the events: a tick that is due is not held back by a busy event stream.
            () = sleep_until_deadline(deadline), if deadline.is_some() => {
                // One reading for every timer, so a tick and the catch-up check cannot disagree
                // about which side of `now` an instant falls on.
                let now = now_in(&engine.now, engine.zone);
                engine.tick(now);
                engine.tick_silents();
                engine.tick_holds();
                engine.tick_suns(now);
                engine.tick_wake_ups(now);
                engine.tick_off_peak();
                engine.tick_catch_up(now);
            }
            event = events.recv() => match event {
                Ok(event) => engine.handle_event(event),
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "flow engine lagged behind the events, forgetting values");
                    engine.events_lagged();
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            // After the events, so a change is stored by `StateChanged` first. Every message on
            // an entity's state topic, read or not: it re-seeds an unseeded value, restarts that
            // entity's silent clocks and confirms a value that is still unknown.
            seen = seen.recv() => match seen {
                Ok(seen) => engine.state_touched(&seen),
                // Reports may have been missed: no entity can be called silent from here.
                Err(broadcast::error::RecvError::Lagged(_)) => engine.seen_lagged(),
                Err(broadcast::error::RecvError::Closed) => return,
            },
        }
    }
}

/// Sleeps until `deadline`; never completes without one (the select branch is disabled then).
async fn sleep_until_deadline(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// A `trigger.silent` node's clock: it runs from `since`, which every report of the entity
/// restarts, and fires once when it reaches `period`.
struct Silent {
    node: String,
    entity: String,
    /// The node's `for` text, to tell an edited node from an unchanged one.
    for_: String,
    period: chrono::Duration,
    since: chrono::DateTime<chrono::Utc>,
    /// Set when the clock fired; cleared by the next report. A due clock whose flow is busy in
    /// `single` mode does not fire (and stays unset) until that run ends.
    fired: bool,
}

/// A `trigger.state` edge waiting out its `for`: it fires at `at` if the value still holds.
struct Hold {
    flow: String,
    node: String,
    /// The node's kind when armed, to tell an edited node from an unchanged one.
    kind: NodeKind,
    params: TriggerStateParams,
    /// The value the edge brought, which a trigger with no `to`, `above` or `below` must keep.
    edge: Scalar,
    info: TriggerInfo,
    at: chrono::DateTime<chrono::Utc>,
}

struct Engine {
    hub: Arc<Hub>,
    /// The house time zone schedules run in.
    zone: chrono_tz::Tz,
    now: Now,
    /// Armed schedule nodes, keyed `"{flow}/{node}"`.
    schedules: BTreeMap<String, Armed>,
    /// Armed silent nodes, keyed `"{flow}/{node}"`.
    silents: BTreeMap<String, Silent>,
    /// The hub's [`Hub::link_epoch`] the silent clocks were last restarted in. A different one
    /// while online means the link bounced and the clocks are from before it.
    silent_epoch: u64,
    /// `trigger.state` holds in progress, keyed `"{flow}/{node}"`.
    holds: BTreeMap<String, Hold>,
    /// Armed sun nodes, keyed `"{flow}/{node}"`.
    sun_timers: BTreeMap<String, ArmedSun>,
    /// Armed wake-up nodes, keyed `"{flow}/{node}"`.
    wake_timers: BTreeMap<String, ArmedWakeUp>,
    /// The off-peak answer `trigger.off_peak` compares against; `None` until seeded. Seeding
    /// never fires: boot, a reconnect replay, a flow save or a settings change only set it.
    off_peak_base: Option<bool>,
    /// The next off-peak window start or end, when the answer may change by the clock.
    off_peak_timer: Option<chrono::DateTime<chrono_tz::Tz>>,
    /// A minute after the link comes back: seeds the base if no device report did by then.
    off_peak_settle: Option<chrono::DateTime<chrono_tz::Tz>>,
    /// When the next catch-up check runs: a few seconds after the hub starts or the link comes
    /// back online, never after an edit.
    catch_up_settle: Option<chrono::DateTime<chrono_tz::Tz>>,
    /// Valid and enabled flows.
    flows: Vec<Flow>,
    /// The last value seen for every (entity, field) a trigger watches; `None` inside means
    /// the value was absent when last seen. Absent means never seen since the last reset.
    known: BTreeMap<Watched, Option<Scalar>>,
    /// The values watched as of the last reload. Kept apart from `known` because a
    /// disconnect empties `known` while the watched set stays: those values must stay
    /// unseeded until the broker's replay shows them, not be re-seeded from stale values.
    watched: BTreeSet<Watched>,
    /// Watched values that a report of their entity showed to be unknown, as opposed to unknown
    /// because nothing was seen yet. A value appearing out of this state is a real first value
    /// and fires a `trigger.state` with a `to`. Cleared with `known`.
    confirmed_unknown: BTreeSet<Watched>,
    active: BTreeMap<String, Active>,
    /// `(flow, run)` pairs whose run disabled its own flow with a `flow.enable` node: that run
    /// survives the reload the disable causes. Forgotten when the run ends.
    keep: BTreeSet<(String, String)>,
    tasks: JoinSet<Finished>,
}

/// A value a trigger watches: an entity and a field of its state (`None` for its headline value).
type Watched = (String, Option<String>);

/// The value `field` names in `value` of an entity of discovery `component`: its headline value
/// when `None`.
fn watched_value(component: &str, value: &StateValue, field: Option<&str>) -> Option<Scalar> {
    match field {
        None => headline_scalar(component, value),
        Some(field) => field_scalar(value, field),
    }
}

/// Whether two definitions of a flow do the same thing: they may differ only in how the UI
/// presents the flow (`name`, `description`, `area`, `labels`), so renaming it, moving it to
/// another room or relabelling it never cancels its run.
fn same_behaviour(a: &Flow, b: &Flow) -> bool {
    let behaviour = |flow: &Flow| Flow {
        name: String::new(),
        description: None,
        area: None,
        labels: Vec::new(),
        ..flow.clone()
    };
    behaviour(a) == behaviour(b)
}

impl Engine {
    fn handle_request(&mut self, request: EngineRequest) {
        match request {
            EngineRequest::Reload => self.reload(),
            EngineRequest::Run {
                flow_id,
                run_id,
                node,
            } => self.run_manual(&flow_id, run_id, node),
            EngineRequest::KeepRun { flow_id, run_id } => {
                self.keep.insert((flow_id, run_id));
            }
        }
    }

    /// Re-reads the definitions, seeds the values of newly watched entities from the hub (so a
    /// flow written while the entity already has a value fires on its very next change) and
    /// cancels runs whose flow was removed, disabled or changed in what it does.
    fn reload(&mut self) {
        let definitions = self.hub.flow_definitions();
        self.flows = definitions
            .iter()
            .filter(|flow| flow.enabled)
            .cloned()
            .collect();
        let watched = self.watched_values();
        self.known.retain(|key, _| watched.contains(key));
        self.confirmed_unknown.retain(|key| watched.contains(key));
        // Seed only what became watched now, and only while the link is up: otherwise the
        // registry may hold pre-drop values and the broker's replay would look like a change.
        let online = *self.hub.connection().borrow() == ConnectionStatus::Online;
        if online {
            for key in watched.difference(&self.watched) {
                let scalar = self.hub.entity_value(&key.0, key.1.as_deref());
                self.known.insert(key.clone(), scalar);
            }
        }
        self.watched = watched;
        self.hub.set_averaged(averaged_windows(&self.flows));
        let stale: Vec<String> = self
            .active
            .iter()
            .filter(|(id, active)| {
                let unchanged = self
                    .flows
                    .iter()
                    .any(|f| f.id == **id && same_behaviour(f, &active.flow));
                !(unchanged || self.survives_self_disable(id, active, &definitions))
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            self.cancel(&id);
        }
        self.arm_schedules();
        self.arm_silents();
        self.keep_holds();
        self.arm_suns();
        self.arm_wake_ups();
        self.off_peak_reload();
        // Disabled flows keep their counts: only a removed or edited node forgets them.
        let mut limits = BTreeMap::new();
        for flow in &definitions {
            for node in &flow.nodes {
                if let NodeKind::Limit(params) = &node.kind {
                    limits.insert(
                        (flow.id.clone(), node.id.clone()),
                        (params.max, params.per.clone()),
                    );
                }
            }
        }
        self.hub.limit_retain(&limits);
    }

    /// Whether `active` is a run that disabled its own flow (announced by `KeepRun`) and whose
    /// flow behaves like the definition it started with but for `enabled`.
    fn survives_self_disable(&self, id: &str, active: &Active, definitions: &[Flow]) -> bool {
        if !self.keep.contains(&(id.to_owned(), active.run_id.clone())) {
            return false;
        }
        let enabled = |flow: &Flow| Flow {
            enabled: true,
            ..flow.clone()
        };
        definitions
            .iter()
            .any(|f| f.id == id && same_behaviour(&enabled(f), &enabled(&active.flow)))
    }

    /// Arms every schedule node of an enabled flow that is not armed with the same expression
    /// yet, from now (never immediately), and disarms the rest: disabled, removed or edited.
    fn arm_schedules(&mut self) {
        let now = now_in(&self.now, self.zone);
        let mut wanted = BTreeMap::new();
        for flow in &self.flows {
            for node in flow.triggers() {
                let NodeKind::TriggerSchedule(params) = &node.kind else {
                    continue;
                };
                wanted.insert(
                    format!("{}/{}", flow.id, node.id),
                    (node.id.clone(), params.cron.clone()),
                );
            }
        }
        self.schedules.retain(|key, armed| {
            wanted
                .get(key)
                .is_some_and(|(node, cron)| *node == armed.node && *cron == armed.cron)
        });
        for (key, (node, cron)) in wanted {
            if self.schedules.contains_key(&key) {
                continue;
            }
            // The definition was validated when the flow was stored.
            let Ok(schedule) = Schedule::parse(&cron) else {
                continue;
            };
            if let Some(at) = schedule.next_after(&now) {
                self.schedules.insert(
                    key,
                    Armed {
                        node,
                        at,
                        schedule,
                        cron,
                    },
                );
            }
        }
    }

    /// Arms every silent node of an enabled flow that is not armed with the same entity and
    /// duration yet, counting from now, and disarms the rest: disabled, removed or edited. A
    /// node armed with the same parameters keeps its clock, so renaming the flow does not
    /// restart it.
    fn arm_silents(&mut self) {
        let now = (self.now)();
        let mut wanted = BTreeMap::new();
        for flow in &self.flows {
            for node in flow.triggers() {
                let NodeKind::TriggerSilent(params) = &node.kind else {
                    continue;
                };
                // The definition was validated when the flow was stored.
                let Ok(period) = window(&params.for_) else {
                    continue;
                };
                let Ok(period) = chrono::Duration::from_std(period) else {
                    continue;
                };
                wanted.insert(
                    format!("{}/{}", flow.id, node.id),
                    (
                        node.id.clone(),
                        params.entity.clone(),
                        params.for_.clone(),
                        period,
                    ),
                );
            }
        }
        self.silents.retain(|key, armed| {
            wanted.get(key).is_some_and(|(node, entity, for_, _)| {
                *node == armed.node && *entity == armed.entity && *for_ == armed.for_
            })
        });
        for (key, (node, entity, for_, period)) in wanted {
            self.silents.entry(key).or_insert(Silent {
                node,
                entity,
                for_,
                period,
                since: now,
                fired: false,
            });
        }
    }

    /// Fires every silent node whose entity stayed silent for its duration, once. Nothing fires
    /// while the broker link is not online: no report could arrive.
    ///
    /// A link that bounced since the clocks last restarted (the `Online` event may be handled
    /// late or lost) restarts them instead of firing: no report could arrive while it was down.
    /// A due clock whose flow is busy in `single` mode waits for that run to end.
    fn tick_silents(&mut self) {
        if !self.online() {
            return;
        }
        if self.silent_epoch != self.hub.link_epoch() {
            self.restart_silents();
            return;
        }
        let now = (self.now)();
        let due: Vec<String> = self
            .silents
            .iter()
            .filter(|(key, armed)| {
                !armed.fired && armed.since + armed.period <= now && !self.silent_blocked(key)
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in due {
            let Some(armed) = self.silents.get_mut(&key) else {
                continue;
            };
            armed.fired = true;
            let node = armed.node.clone();
            let trigger = TriggerInfo {
                entity: Some(armed.entity.clone()),
                field: None,
                value: Value::String(armed.for_.clone()),
                previous: Value::Null,
            };
            let flow_id = key.split('/').next().unwrap_or_default();
            let Some(flow) = self.flows.iter().find(|f| f.id == flow_id).cloned() else {
                continue;
            };
            self.fire(flow, node, Some(trigger));
        }
    }

    /// Whether the flow of silent clock `key` is running in `single` mode, so a fire would only
    /// be recorded as skipped.
    fn silent_blocked(&self, key: &str) -> bool {
        let flow_id = key.split('/').next().unwrap_or_default();
        self.active.contains_key(flow_id)
            && self
                .flows
                .iter()
                .any(|f| f.id == flow_id && f.mode == FlowMode::Single)
    }

    /// Restarts every silent clock from now: the link came back or reports may have been lost.
    fn restart_silents(&mut self) {
        self.silent_epoch = self.hub.link_epoch();
        let now = (self.now)();
        for armed in self.silents.values_mut() {
            armed.since = now;
            armed.fired = false;
        }
    }

    /// Drops the holds whose flow is gone, disabled or whose node was edited.
    fn keep_holds(&mut self) {
        let flows = &self.flows;
        self.holds.retain(|_, hold| {
            flows.iter().any(|flow| {
                flow.id == hold.flow
                    && flow
                        .triggers()
                        .iter()
                        .any(|node| node.id == hold.node && node.kind == hold.kind)
            })
        });
    }

    /// Drops the holds of `key` that the reported value `value` no longer satisfies.
    fn release_holds(&mut self, id: &str, key: &Watched, value: &Scalar) {
        self.holds.retain(|_, hold| {
            hold.params.entity != id
                || hold.params.field != key.1
                || still_holds(&hold.params, &hold.edge, value)
        });
    }

    /// Fires every hold whose wait is over and whose value is still the one it waited on, and
    /// drops them all. Nothing fires while the broker link is not online.
    fn tick_holds(&mut self) {
        let now = (self.now)();
        let due: Vec<String> = self
            .holds
            .iter()
            .filter(|(_, hold)| hold.at <= now)
            .map(|(key, _)| key.clone())
            .collect();
        for key in due {
            let Some(hold) = self.holds.remove(&key) else {
                continue;
            };
            if !self.online() {
                continue;
            }
            let current = self
                .hub
                .entity_value(&hold.params.entity, hold.params.field.as_deref());
            if !current.is_some_and(|v| still_holds(&hold.params, &hold.edge, &v)) {
                continue;
            }
            let Some(flow) = self.flows.iter().find(|f| f.id == hold.flow).cloned() else {
                continue;
            };
            self.fire(flow, hold.node, Some(hold.info));
        }
    }

    /// The `StateTouched` receiver lagged: reports of any entity may be missing.
    fn seen_lagged(&mut self) {
        self.restart_silents();
    }

    /// Arms every sun node of an enabled flow that is not armed for the same event and offset
    /// yet, and disarms the rest: disabled, removed or edited. A node whose next event is more
    /// than seven days out (polar night, midnight sun) stays disarmed until the next reload.
    fn arm_suns(&mut self) {
        let now = now_in(&self.now, self.zone);
        let (lat, lon) = self.hub.house_point();
        let mut wanted = BTreeMap::new();
        for flow in &self.flows {
            for node in flow.triggers() {
                let NodeKind::TriggerSun(params) = &node.kind else {
                    continue;
                };
                wanted.insert(
                    format!("{}/{}", flow.id, node.id),
                    (
                        flow.id.clone(),
                        node.id.clone(),
                        params.which,
                        params.offset(),
                    ),
                );
            }
        }
        self.sun_timers.retain(|key, armed| {
            wanted.get(key).is_some_and(|(_, node, which, offset)| {
                *node == armed.node && *which == armed.which && *offset == armed.offset_minutes
            })
        });
        for (key, (flow, node, which, offset)) in wanted {
            if self.sun_timers.contains_key(&key) {
                continue;
            }
            match next_sun_fire(lat, lon, self.zone, which, offset, now) {
                Some(at) => {
                    self.sun_timers.insert(
                        key,
                        ArmedSun {
                            node,
                            at,
                            which,
                            offset_minutes: offset,
                        },
                    );
                }
                None => tracing::warn!(
                    flow = %flow,
                    node = %node,
                    "sun flow disarmed: next event more than 7 days out or does not exist"
                ),
            }
        }
    }

    /// When the timer branch should wake: the earliest armed tick, sun event or silent clock, as
    /// a tokio instant.
    fn next_deadline(&self) -> Option<tokio::time::Instant> {
        let silent = if !self.online() {
            None
        } else if self.silent_epoch != self.hub.link_epoch() {
            // The next tick restarts the clocks.
            Some((self.now)())
        } else {
            self.silents
                .iter()
                .filter(|(key, armed)| {
                    let due = armed.since + armed.period;
                    // A due clock behind a busy `single` flow wakes up when the run ends.
                    !armed.fired && !(due <= (self.now)() && self.silent_blocked(key))
                })
                .map(|(_, armed)| armed.since + armed.period)
                .min()
        };
        let at = self
            .schedules
            .values()
            .map(|armed| armed.at.to_utc())
            .chain(self.sun_timers.values().map(|armed| armed.at.to_utc()))
            .chain(self.wake_timers.values().map(|armed| armed.at.to_utc()))
            .chain(self.off_peak_timer.map(|at| at.to_utc()))
            .chain(self.off_peak_settle.map(|at| at.to_utc()))
            .chain(self.catch_up_settle.map(|at| at.to_utc()))
            .chain(silent)
            .chain(self.holds.values().map(|hold| hold.at))
            .min()?;
        let wait = (at - (self.now)())
            .to_std()
            .unwrap_or(std::time::Duration::ZERO);
        Some(tokio::time::Instant::now() + wait)
    }

    /// Fires every schedule whose tick is due and re-arms it for the next one. A tick while the
    /// broker link is not online is recorded as skipped instead: commands could not go out.
    fn tick(&mut self, now: chrono::DateTime<chrono_tz::Tz>) {
        let due: Vec<String> = self
            .schedules
            .iter()
            .filter(|(_, armed)| armed.at <= now)
            .map(|(key, _)| key.clone())
            .collect();
        let online = *self.hub.connection().borrow() == ConnectionStatus::Online;
        for key in due {
            let Some(armed) = self.schedules.get_mut(&key) else {
                continue;
            };
            let node = armed.node.clone();
            let due_at = armed.at;
            let next = rearm(&armed.schedule, armed.at, now);
            match next {
                Some(at) => armed.at = at,
                None => {
                    self.schedules.remove(&key);
                }
            }
            let flow_id = key.split('/').next().unwrap_or_default();
            let Some(flow) = self.flows.iter().find(|f| f.id == flow_id).cloned() else {
                continue;
            };
            self.fire_due(flow, node, due_at, online);
        }
    }

    /// Fires every sun node whose event is due and re-arms it for the next one, skipping while
    /// the broker link is not online like `tick`. Events missed while the hub was busy are
    /// never replayed: the next one is searched from now.
    fn tick_suns(&mut self, now: chrono::DateTime<chrono_tz::Tz>) {
        let (lat, lon) = self.hub.house_point();
        let due: Vec<String> = self
            .sun_timers
            .iter()
            .filter(|(_, armed)| armed.at <= now)
            .map(|(key, _)| key.clone())
            .collect();
        let online = *self.hub.connection().borrow() == ConnectionStatus::Online;
        for key in due {
            let Some(armed) = self.sun_timers.get_mut(&key) else {
                continue;
            };
            let node = armed.node.clone();
            let due_at = armed.at;
            let next = next_sun_fire(lat, lon, self.zone, armed.which, armed.offset_minutes, now);
            match next {
                Some(at) => armed.at = at,
                None => {
                    self.sun_timers.remove(&key);
                }
            }
            let flow_id = key.split('/').next().unwrap_or_default();
            let Some(flow) = self.flows.iter().find(|f| f.id == flow_id).cloned() else {
                continue;
            };
            self.fire_due(flow, node, due_at, online);
        }
    }

    /// The alarms a wake-up node follows and the fingerprint of what its pre-ring depends on.
    fn wake_up_inputs(
        &self,
        params: &nidoo_core::flow::TriggerWakeUpParams,
    ) -> (Vec<nidoo_core::flow::Schedule>, String) {
        let found = wake_up::alarm_schedules(&self.flows, &params.alarms);
        let mut parts = vec![params.before_minutes.to_string(), params.alarms.join(",")];
        parts.extend(found.iter().map(|(id, cron, _)| format!("{id}={cron}")));
        let schedules = found.into_iter().map(|(_, _, schedule)| schedule).collect();
        (schedules, parts.join("|"))
    }

    /// Arms every wake-up node of an enabled flow for the next pre-ring of its alarms and
    /// disarms the rest. A node whose alarms and lead time are unchanged keeps its instant;
    /// otherwise it is re-armed from now. No alarm means no timer.
    fn arm_wake_ups(&mut self) {
        let now = now_in(&self.now, self.zone);
        let mut wanted = BTreeMap::new();
        for flow in &self.flows {
            for node in flow.triggers() {
                let NodeKind::TriggerWakeUp(params) = &node.kind else {
                    continue;
                };
                let (schedules, fingerprint) = self.wake_up_inputs(params);
                wanted.insert(
                    format!("{}/{}", flow.id, node.id),
                    (
                        node.id.clone(),
                        params.before_minutes,
                        schedules,
                        fingerprint,
                    ),
                );
            }
        }
        // An existing entry whose inputs changed is re-armed; what it last fired is kept.
        let mut carried: BTreeMap<String, Option<chrono::DateTime<chrono_tz::Tz>>> =
            BTreeMap::new();
        self.wake_timers.retain(|key, armed| {
            let keep = wanted
                .get(key)
                .is_some_and(|(_, _, _, fingerprint)| *fingerprint == armed.fingerprint);
            if !keep && wanted.contains_key(key) {
                carried.insert(key.clone(), armed.last_fired);
            }
            keep
        });
        let mut fires = Vec::new();
        for (key, (node, before, schedules, fingerprint)) in wanted {
            if self.wake_timers.contains_key(&key) {
                continue;
            }
            let mut last_fired = None;
            if let Some(previous) = carried.get(&key) {
                last_fired = *previous;
                // The new pre-ring may already be behind us while the ring is still ahead:
                // fire now, unless the node already fired within its lead time.
                let lead = chrono::TimeDelta::minutes(i64::from(before));
                if wake_up::ring_within(&schedules, before, &now).is_some()
                    && last_fired.is_none_or(|at| at < now - lead)
                {
                    fires.push((key.clone(), node.clone()));
                    last_fired = Some(now);
                }
            }
            if let Some(at) = wake_up::next_pre_ring(&schedules, before, &now) {
                self.wake_timers.insert(
                    key,
                    ArmedWakeUp {
                        node,
                        at,
                        fingerprint,
                        last_fired,
                    },
                );
            }
        }
        if !fires.is_empty() {
            let online = self.online();
            for (key, node) in fires {
                if let Some(flow) = key
                    .split_once('/')
                    .and_then(|(id, _)| self.flows.iter().find(|f| f.id == id).cloned())
                {
                    self.fire_due(flow, node, now, online);
                }
            }
        }
    }

    /// Fires every wake-up node whose pre-ring is due and re-arms it for the next one,
    /// skipping while the broker link is not online like `tick`. A pre-ring missed while the
    /// hub was busy is never replayed: the next one is searched from the later of the missed
    /// instant and now.
    fn tick_wake_ups(&mut self, now: chrono::DateTime<chrono_tz::Tz>) {
        let due: Vec<String> = self
            .wake_timers
            .iter()
            .filter(|(_, armed)| armed.at <= now)
            .map(|(key, _)| key.clone())
            .collect();
        let online = *self.hub.connection().borrow() == ConnectionStatus::Online;
        for key in due {
            let Some((flow_id, _)) = key.split_once('/') else {
                continue;
            };
            let Some(flow) = self.flows.iter().find(|f| f.id == flow_id).cloned() else {
                self.wake_timers.remove(&key);
                continue;
            };
            let Some(armed) = self.wake_timers.get(&key) else {
                continue;
            };
            let node = armed.node.clone();
            let due_at = armed.at;
            let from = armed.at.max(now);
            let params = flow.nodes.iter().find_map(|n| match &n.kind {
                NodeKind::TriggerWakeUp(p) if n.id == node => Some(p.clone()),
                _ => None,
            });
            let next = params.and_then(|p| {
                let (schedules, _) = self.wake_up_inputs(&p);
                wake_up::next_pre_ring(&schedules, p.before_minutes, &from)
            });
            match (next, self.wake_timers.get_mut(&key)) {
                (Some(at), Some(armed)) => {
                    armed.at = at;
                    if online {
                        armed.last_fired = Some(now);
                    }
                }
                _ => {
                    self.wake_timers.remove(&key);
                }
            }
            self.fire_due(flow, node, due_at, online);
        }
    }

    /// The off-peak device's entity id, when one is set.
    fn off_peak_device(&self) -> Option<String> {
        self.hub
            .home_settings()
            .off_peak_device
            .map(|device| device.entity)
    }

    fn online(&self) -> bool {
        *self.hub.connection().borrow() == ConnectionStatus::Online
    }

    /// Resolves off-peak now and re-arms the boundary timer.
    fn off_peak_resolve(&mut self) -> OffPeakNow {
        let now = self.hub.off_peak_now(now_in(&self.now, self.zone));
        self.off_peak_timer = now.next_boundary;
        now
    }

    /// Events were missed: values seen before may be stale. A device's answer is forgotten and
    /// seeded again by its next report, or a minute later.
    fn events_lagged(&mut self) {
        // The `Online` event may be among the missed ones.
        self.restart_silents();
        self.known.clear();
        self.confirmed_unknown.clear();
        if self.off_peak_device().is_some() {
            self.off_peak_base = None;
        }
        if self.online() {
            self.off_peak_settle = Some(now_in(&self.now, self.zone) + OFF_PEAK_SETTLE);
        }
    }

    /// The link dropped: the device's next reading may be a replay, so its answer is forgotten.
    /// The windows' answer is kept (nothing replays a clock), so an edge during the outage is
    /// recorded as skipped.
    fn off_peak_lost(&mut self) {
        if self.off_peak_device().is_some() {
            self.off_peak_base = None;
        }
        self.off_peak_settle = None;
    }

    /// The answer to start from without firing, `None` to wait. Only while online. With a
    /// device, only from a usable reading seen since the last seed: before its report the
    /// windows (or a pre-drop registry value) may disagree with it, and its first report would
    /// look like a change.
    fn off_peak_seed_value(&self, now: &OffPeakNow) -> Option<bool> {
        if !self.online() {
            return None;
        }
        match self.off_peak_device() {
            None => Some(now.off_peak),
            Some(_) if self.off_peak_base.is_some() && now.device.is_some() => Some(now.off_peak),
            Some(_) => None,
        }
    }

    /// A settings change: takes the new answer as the starting point, never fires.
    fn off_peak_reseed(&mut self) {
        let now = self.off_peak_resolve();
        self.off_peak_base = self.off_peak_seed_value(&now);
    }

    /// A flow save: re-arms the boundary timer and seeds an unseeded base; a seeded one is
    /// kept, so an edge being processed is not swallowed.
    fn off_peak_reload(&mut self) {
        let now = self.off_peak_resolve();
        if self.off_peak_base.is_none() {
            self.off_peak_base = self.off_peak_seed_value(&now);
        }
    }

    /// Re-reads the answer. Unseeded, it seeds when `seed` (a fresh device report); seeded, a
    /// different answer fires the matching `trigger.off_peak` nodes.
    fn off_peak_check(&mut self, seed: bool) {
        let now = self.off_peak_resolve();
        match self.off_peak_base {
            None if seed => self.off_peak_base = Some(now.off_peak),
            Some(previous) if previous != now.off_peak => {
                self.off_peak_base = Some(now.off_peak);
                self.fire_off_peak(&now);
            }
            _ => {}
        }
    }

    /// Fires every enabled flow's `trigger.off_peak` on the edge `now` crossed, once per flow;
    /// recorded as skipped while the broker link is not online, like a schedule tick.
    fn fire_off_peak(&mut self, now: &OffPeakNow) {
        let edge = if now.off_peak {
            OffPeakEdge::Start
        } else {
            OffPeakEdge::End
        };
        let other = if now.off_peak { "end" } else { "start" };
        let fires: Vec<(Flow, String)> = self
            .flows
            .iter()
            .filter_map(|flow| {
                let node = flow.triggers().into_iter().find(
                    |node| matches!(&node.kind, NodeKind::TriggerOffPeak(p) if p.edge == edge),
                )?;
                Some((flow.clone(), node.id.clone()))
            })
            .collect();
        let online = self.online();
        for (flow, node) in fires {
            if online {
                let trigger = TriggerInfo {
                    entity: now.device.clone(),
                    field: None,
                    value: Value::String(edge.wire_name().to_owned()),
                    previous: Value::String(other.to_owned()),
                };
                self.fire(flow, node, Some(trigger));
            } else {
                self.hub
                    .add_skipped_run_because(&flow.id, &node, "the broker link is offline");
            }
        }
    }

    /// Runs the off-peak timers that are due: the settle timer seeds a base no device report
    /// set, the boundary timer re-reads the answer (seeding it when no device is set).
    fn tick_off_peak(&mut self) {
        let now = now_in(&self.now, self.zone);
        if self.off_peak_settle.is_some_and(|at| at <= now) {
            self.off_peak_settle = None;
            if self.off_peak_base.is_none() && self.online() {
                self.off_peak_check(true);
            }
        }
        if self.off_peak_timer.is_some_and(|at| at <= now) {
            let seed = self.online() && self.off_peak_device().is_none();
            self.off_peak_check(seed);
        }
    }

    /// The (entity, field) values some enabled flow triggers on.
    fn watched_values(&self) -> BTreeSet<Watched> {
        self.flows
            .iter()
            .flat_map(|flow| flow.triggers())
            .filter_map(|node| match &node.kind {
                NodeKind::TriggerState(params) => {
                    Some((params.entity.clone(), params.field.clone()))
                }
                NodeKind::TriggerAlarm(_) => Some((crate::alarm::ALARM_ENTITY_ID.to_owned(), None)),
                _ => None,
            })
            .collect()
    }

    fn handle_event(&mut self, event: WsEvent) {
        match event {
            WsEvent::StateChanged { id, state } => {
                self.state_changed(&id, &state.value);
                if self.off_peak_device().as_deref() == Some(id.as_str()) {
                    self.off_peak_check(true);
                }
            }
            WsEvent::HomeSettingsChanged { .. } => self.off_peak_reseed(),
            WsEvent::ConnectionChanged(ConnectionStatus::Online) => {
                self.restart_silents();
                self.off_peak_settle = Some(now_in(&self.now, self.zone) + OFF_PEAK_SETTLE);
                self.catch_up_settle = Some(now_in(&self.now, self.zone) + CATCH_UP_SETTLE);
            }
            WsEvent::ButtonPressed { id } => self.button_pressed(&id),
            WsEvent::SignalReceived { id, signal } => self.signal_received(&id, &signal),
            // The broker is about to replay retained states: forget what we knew so the replay
            // is only recorded, never acted on.
            WsEvent::ConnectionChanged(status) if status != ConnectionStatus::Online => {
                self.known.clear();
                self.confirmed_unknown.clear();
                self.off_peak_lost();
                self.catch_up_settle = None;
            }
            _ => {}
        }
    }

    /// Fires every trigger on button `id`. A press is an event that is never replayed (the hub
    /// ignores retained messages on a button topic), so the `known` map is bypassed. A button
    /// has no value, so the trigger carries the press itself.
    fn button_pressed(&mut self, id: &str) {
        let mut fires = Vec::new();
        for flow in &self.flows {
            for node in flow.triggers() {
                if matches!(&node.kind, NodeKind::TriggerState(params) if params.entity == id)
                    && !fires.iter().any(|(f, _): &(Flow, String)| f.id == flow.id)
                {
                    fires.push((flow.clone(), node.id.clone()));
                }
            }
        }
        let trigger = TriggerInfo {
            entity: Some(id.to_owned()),
            field: None,
            value: Value::String("PRESS".to_owned()),
            previous: Value::Null,
        };
        for (flow, node) in fires {
            self.fire(flow, node, Some(trigger.clone()));
        }
    }

    /// Fires every `trigger.signal` on `id` that wants this signal. Like a press, a signal is an
    /// event the hub never replays from retained messages, so the `known` map is bypassed. The
    /// trigger value is the signal name.
    fn signal_received(&mut self, id: &str, signal: &str) {
        let mut fires = Vec::new();
        for flow in &self.flows {
            for node in flow.triggers() {
                if matches!(&node.kind, NodeKind::TriggerSignal(params)
                        if params.entity == id
                            && params.signal.as_deref().is_none_or(|s| s == signal))
                    && !fires.iter().any(|(f, _): &(Flow, String)| f.id == flow.id)
                {
                    fires.push((flow.clone(), node.id.clone()));
                }
            }
        }
        let trigger = TriggerInfo {
            entity: Some(id.to_owned()),
            field: None,
            value: Value::String(signal.to_owned()),
            previous: Value::Null,
        };
        for (flow, node) in fires {
            self.fire(flow, node, Some(trigger.clone()));
        }
    }

    /// A message arrived on a state topic of `id`, whatever the entity read from it. A watched
    /// entity missing from the map is seeded silently from the registry: after a reconnect the
    /// retained replay is usually identical to the last state, so the registry emits no
    /// `StateChanged` for it. A message that left the value unknown (`read` false) seeds only
    /// keys the registry also has unknown, and still confirms an unknown value.
    ///
    /// A report from before the latest link drop is stale (the engine may only get to it after
    /// the clear) and is ignored: its registry value is exactly the pre-drop one.
    fn state_touched(&mut self, seen: &StateTouched) {
        let id = seen.id.as_str();
        if seen.epoch == self.hub.link_epoch() {
            let now = (self.now)();
            for armed in self.silents.values_mut().filter(|a| a.entity == id) {
                armed.since = now;
                armed.fired = false;
            }
            let unseeded: Vec<Watched> = self
                .watched
                .iter()
                .filter(|key| key.0 == id && !self.known.contains_key(*key))
                .cloned()
                .collect();
            for key in unseeded {
                let scalar = self.hub.entity_value(id, key.1.as_deref());
                if seen.read || scalar.is_none() {
                    self.known.insert(key, scalar);
                }
            }
            // The entity reported while this value was unknown: a value that shows up in a
            // later report is a first value, not a replay.
            let unknown: Vec<Watched> = self
                .watched
                .iter()
                .filter(|key| key.0 == id && self.known.get(*key) == Some(&None))
                .cloned()
                .collect();
            self.confirmed_unknown.extend(unknown);
        }
        if seen.read
            && seen.epoch == self.hub.link_epoch()
            && self.off_peak_base.is_none()
            && self.off_peak_device().as_deref() == Some(id)
        {
            self.off_peak_check(true);
        }
    }

    /// Fires the triggers an entity's change satisfies, value by watched value. A change between
    /// two present, different values counts, and so does a value appearing after a report
    /// showed it unknown (only for a trigger with a `to` and no `from`). A first sighting, an
    /// availability-only report (same value), a restore and a value vanishing are recorded
    /// silently.
    fn state_changed(&mut self, id: &str, state: &StateValue) {
        let keys: Vec<Watched> = self
            .watched
            .iter()
            .filter(|key| key.0 == id)
            .cloned()
            .collect();
        if keys.is_empty() {
            return;
        }
        let component = self
            .hub
            .read()
            .entity(id)
            .map(|e| e.component.clone())
            .unwrap_or_default();
        let mut fires: Vec<(Flow, String, TriggerInfo)> = Vec::new();
        let mut arms: Vec<(String, Hold)> = Vec::new();
        for key in keys {
            let next = watched_value(&component, state, key.1.as_deref());
            if let Some(value) = &next {
                self.release_holds(id, &key, value);
            }
            // Never seen since the last reset: recorded, not acted on.
            let Some(previous) = self.known.insert(key.clone(), next.clone()) else {
                continue;
            };
            let confirmed = next.is_some() && self.confirmed_unknown.remove(&key);
            let (previous, next) = match (previous, next) {
                (Some(previous), Some(next)) => (Some(previous), next),
                (None, Some(next)) if confirmed => (None, next),
                _ => continue,
            };
            if previous.as_ref() == Some(&next) {
                continue;
            }
            let trigger = TriggerInfo {
                entity: Some(id.to_owned()),
                field: key.1.clone(),
                value: next.to_json(),
                previous: previous.as_ref().map_or(Value::Null, Scalar::to_json),
            };
            for flow in &self.flows {
                for node in flow.triggers() {
                    if fires.iter().any(|(f, _, _)| f.id == flow.id) {
                        continue;
                    }
                    let satisfied = match &node.kind {
                        NodeKind::TriggerState(params) => {
                            params.entity == id
                                && params.field == key.1
                                && if params.above.is_some() || params.below.is_some() {
                                    params.in_bounds(&next) == Some(true)
                                        && previous
                                            .as_ref()
                                            .is_none_or(|p| params.in_bounds(p) != Some(true))
                                } else {
                                    filter_matches(params.to.as_ref(), &next)
                                        && match &previous {
                                            Some(previous) => {
                                                filter_matches(params.from.as_ref(), previous)
                                            }
                                            None => params.from.is_none() && params.to.is_some(),
                                        }
                                }
                        }
                        NodeKind::TriggerAlarm(params) => {
                            let Scalar::Text(next) = &next else {
                                continue;
                            };
                            id == crate::alarm::ALARM_ENTITY_ID
                                && key.1.is_none()
                                && params.to.is_none_or(|to| to.matches(next))
                                && match &previous {
                                    Some(Scalar::Text(previous)) => {
                                        params.from.is_none_or(|from| from.matches(previous))
                                    }
                                    Some(_) => false,
                                    None => params.from.is_none() && params.to.is_some(),
                                }
                        }
                        _ => false,
                    };
                    if !satisfied {
                        continue;
                    }
                    let wait = match &node.kind {
                        NodeKind::TriggerState(params) => params.for_.as_deref().map(|text| {
                            // The definition was validated when the flow was stored.
                            (
                                params,
                                hold(text)
                                    .ok()
                                    .and_then(|d| chrono::Duration::from_std(d).ok()),
                            )
                        }),
                        _ => None,
                    };
                    match wait {
                        None => fires.push((flow.clone(), node.id.clone(), trigger.clone())),
                        Some((_, None)) => {}
                        Some((params, Some(period))) => arms.push((
                            format!("{}/{}", flow.id, node.id),
                            Hold {
                                flow: flow.id.clone(),
                                node: node.id.clone(),
                                kind: node.kind.clone(),
                                params: params.clone(),
                                edge: next.clone(),
                                info: trigger.clone(),
                                at: (self.now)() + period,
                            },
                        )),
                    }
                }
            }
        }
        self.holds.extend(arms);
        for (flow, node, trigger) in fires {
            self.fire(flow, node, Some(trigger));
        }
    }

    /// Fires a time trigger that came due at `due_at`, or records the run as skipped while the
    /// broker link is not online. A trigger with a catch-up window records the fire, so the
    /// next check does not repeat it.
    fn fire_due(
        &mut self,
        flow: Flow,
        node: String,
        due_at: chrono::DateTime<chrono_tz::Tz>,
        online: bool,
    ) {
        if !online {
            self.hub
                .add_skipped_run_because(&flow.id, &node, "the broker link is offline");
            return;
        }
        if catch_up_window(&flow, &node) > 0 {
            self.hub.record_fired(&flow.id, &node, due_at.to_utc());
        }
        self.fire(flow, node, None);
    }

    /// Runs the catch-up check once its settle time is up, if the link is still online.
    fn tick_catch_up(&mut self, now: chrono::DateTime<chrono_tz::Tz>) {
        if self.catch_up_settle.is_some_and(|at| at <= now) {
            self.catch_up_settle = None;
            if self.online() {
                self.catch_up(now);
            }
        }
    }

    /// For every time trigger with a catch-up window, fires its flow once when the trigger came
    /// due inside the window and no fire is recorded for that instant. Only the latest missed
    /// instant counts. A trigger with no marker yet is taken as caught up: the marker is seeded
    /// and nothing fires.
    fn catch_up(&mut self, now: chrono::DateTime<chrono_tz::Tz>) {
        let (lat, lon) = self.hub.house_point();
        let mut fires = Vec::new();
        for flow in &self.flows {
            for node in flow.triggers() {
                let minutes = node.kind.catch_up_minutes();
                if minutes == 0 {
                    continue;
                }
                let from = now - chrono::Duration::minutes(i64::from(minutes));
                let due = match &node.kind {
                    NodeKind::TriggerSchedule(p) => Schedule::parse(&p.cron)
                        .ok()
                        .and_then(|schedule| schedule.latest_in(&from, &now)),
                    NodeKind::TriggerSun(p) => {
                        latest_sun_fire_in(lat, lon, self.zone, p.which, p.offset(), from, now)
                    }
                    NodeKind::TriggerWakeUp(p) => {
                        let (schedules, _) = self.wake_up_inputs(p);
                        wake_up::latest_pre_ring_in(&schedules, p.before_minutes, &from, &now)
                    }
                    _ => None,
                };
                match (self.hub.fired_due(&flow.id, &node.id), due) {
                    (None, due) => {
                        self.hub
                            .record_fired(&flow.id, &node.id, due.unwrap_or(now).to_utc());
                    }
                    (Some(marker), Some(due)) if marker < due.timestamp() => {
                        self.hub.record_fired(&flow.id, &node.id, due.to_utc());
                        fires.push((flow.clone(), node.id.clone(), due));
                    }
                    _ => {}
                }
            }
        }
        for (flow, node, due) in fires {
            // No trigger value, like an on-time fire: the label only replaces the trigger
            // step's detail and never reaches the run's JSON or `{trigger}`.
            let label = format!("caught up: due {}", due.format("%H:%M"));
            self.fire_labelled(flow, node, None, Some(label));
        }
    }

    /// Starts a run for a satisfied trigger, honouring the flow's mode when one is active.
    fn fire(&mut self, flow: Flow, node_id: String, trigger: Option<TriggerInfo>) {
        self.fire_labelled(flow, node_id, trigger, None);
    }

    /// [`Self::fire`], with `label` as the trigger step's detail when set (a catch-up run).
    fn fire_labelled(
        &mut self,
        flow: Flow,
        node_id: String,
        trigger: Option<TriggerInfo>,
        label: Option<String>,
    ) {
        if self.active.contains_key(&flow.id) {
            match flow.mode {
                nidoo_core::flow::FlowMode::Single => {
                    self.hub.add_skipped_run(&flow.id, &node_id, trigger);
                    return;
                }
                nidoo_core::flow::FlowMode::Restart => self.cancel(&flow.id),
            }
        }
        let run = match self.hub.begin_run(&flow.id, trigger.clone()) {
            Ok(run) => run,
            Err(err) => {
                tracing::warn!(flow = %flow.id, error = %err, "could not begin a run");
                return;
            }
        };
        self.spawn(flow, run.id, node_id, trigger, label);
    }

    /// Executes a run the hub created for a manual request, from the Manual card `node` or the
    /// flow's first trigger. A card press on a `restart` flow replaces the active run.
    fn run_manual(&mut self, flow_id: &str, run_id: String, node: Option<String>) {
        let flow = self.flows.iter().find(|f| f.id == flow_id).cloned();
        let start = node
            .clone()
            .or_else(|| flow.as_ref()?.triggers().first().map(|n| n.id.clone()));
        match (flow, start) {
            (Some(flow), Some(start)) => {
                if self.active.contains_key(flow_id) {
                    if node.is_none() || flow.mode != FlowMode::Restart {
                        self.hub.end_run(flow_id, &run_id, RunStatus::Cancelled);
                        return;
                    }
                    self.cancel(flow_id);
                }
                let trigger = manual_trigger(&flow, &start);
                self.spawn(flow, run_id, start, trigger, None);
            }
            _ => {
                tracing::warn!(
                    flow = flow_id,
                    "manual run of a flow the engine does not have"
                );
                self.hub.end_run(flow_id, &run_id, RunStatus::Cancelled);
            }
        }
    }

    fn spawn(
        &mut self,
        flow: Flow,
        run_id: String,
        start: String,
        trigger: Option<TriggerInfo>,
        label: Option<String>,
    ) {
        let hub = Arc::clone(&self.hub);
        let now = Arc::clone(&self.now);
        let (flow_id, task_flow, task_run) = (flow.id.clone(), flow.clone(), run_id.clone());
        let abort = self.tasks.spawn(async move {
            let status =
                execute(hub, now, task_flow, task_run.clone(), start, trigger, label).await;
            (flow_id, task_run, status)
        });
        self.active.insert(
            flow.id.clone(),
            Active {
                run_id,
                flow,
                task: abort.id(),
                abort,
            },
        );
    }

    /// Aborts a flow's active run and ends it `Cancelled`.
    fn cancel(&mut self, flow_id: &str) {
        if let Some(active) = self.active.remove(flow_id) {
            self.keep.retain(|(flow, _)| flow != flow_id);
            active.abort.abort();
            self.hub
                .end_run(flow_id, &active.run_id, RunStatus::Cancelled);
        }
    }

    /// A run task ended. A cancelled one was already ended by whoever cancelled it, and a run
    /// that finished at the very moment it was cancelled no longer matches `active`.
    fn run_finished(
        &mut self,
        joined: Result<(tokio::task::Id, Finished), tokio::task::JoinError>,
    ) {
        match joined {
            Ok((_, (flow_id, run_id, status))) => {
                if self
                    .active
                    .get(&flow_id)
                    .is_some_and(|active| active.run_id == run_id)
                {
                    self.active.remove(&flow_id);
                    self.keep.retain(|(flow, _)| *flow != flow_id);
                    self.hub.end_run(&flow_id, &run_id, status);
                }
            }
            Err(err) if err.is_cancelled() => {}
            Err(err) => {
                tracing::error!(error = %err, "a flow run panicked");
                // Without this the flow would stay active forever and never run again.
                let flow_id = self
                    .active
                    .iter()
                    .find(|(_, active)| active.task == err.id())
                    .map(|(id, _)| id.clone());
                if let Some(flow_id) = flow_id
                    && let Some(active) = self.active.remove(&flow_id)
                {
                    self.keep.retain(|(flow, _)| *flow != flow_id);
                    let failure = step(
                        "engine",
                        StepOutcome::Failed,
                        "run panicked".to_owned(),
                        None,
                    );
                    self.hub.record_step(&flow_id, &active.run_id, failure);
                    self.hub
                        .end_run(&flow_id, &active.run_id, RunStatus::Failed);
                }
            }
        }
    }
}

/// The catch-up window of the time trigger `node` of `flow`, in minutes; 0 when it has none.
fn catch_up_window(flow: &Flow, node: &str) -> u32 {
    flow.nodes
        .iter()
        .find(|n| n.id == node)
        .map_or(0, |n| n.kind.catch_up_minutes())
}

/// Whether the reported `value` still satisfies a state trigger that waits on it: its `to`, its
/// bounds, or, with neither, the value the edge brought.
fn still_holds(params: &TriggerStateParams, edge: &Scalar, value: &Scalar) -> bool {
    if params.to.is_some() {
        filter_matches(params.to.as_ref(), value)
    } else if let Some(inside) = params.in_bounds(value) {
        inside
    } else {
        value == edge
    }
}

fn filter_matches(filter: Option<&Value>, actual: &Scalar) -> bool {
    filter.is_none_or(|value| Scalar::from_json(value).as_ref() == Some(actual))
}

/// What a press of the Manual card `node` carries: its value, with no entity behind it. `None`
/// when `node` is not a Manual card or has no value.
pub(crate) fn manual_trigger(flow: &Flow, node: &str) -> Option<TriggerInfo> {
    let node = flow.nodes.iter().find(|n| n.id == node)?;
    let NodeKind::TriggerManual(params) = &node.kind else {
        return None;
    };
    Some(TriggerInfo {
        entity: None,
        field: None,
        value: params.value.as_ref()?.to_json(),
        previous: Value::Null,
    })
}

/// Walks the flow depth-first from `start`, recording a step per node before following its
/// successors in edge order. A failed step ends its own branch (its successors never run) but
/// not its siblings: the branches still waiting on the stack go on, and the run then ends
/// `Failed`. Fan-out branches are independent, so one device that does not answer must not
/// leave the others untouched (a TV in standby inside a light group kept three other lights on).
///
/// A cancelled run must not act: `abort()` only lands at the task's next `.await`, so a run can
/// wake from its delay after the flow was deleted, disabled or the run ended. `record_step`
/// reports whether the run is still the flow's active one; the run stops (as `Cancelled`) as
/// soon as it is not, and every delay is followed by a step, so a command never follows a
/// wake-up that was not checked. The window between that check and the publish inside
/// `send_command` (no `.await` in between) is accepted.
pub(super) async fn execute(
    hub: Arc<Hub>,
    now: Now,
    flow: Flow,
    run_id: String,
    start: String,
    trigger: Option<TriggerInfo>,
    mut start_label: Option<String>,
) -> RunStatus {
    let mut stack = vec![start];
    let mut failed = false;
    // The results of the run's math cards, by name; every branch reads and writes this one map.
    let mut results: BTreeMap<String, f64> = BTreeMap::new();
    while let Some(id) = stack.pop() {
        let Some(node) = flow.node(&id) else {
            let step = step(&id, StepOutcome::Failed, "unknown node".to_owned(), None);
            if !hub.record_step(&flow.id, &run_id, step) {
                return RunStatus::Cancelled;
            }
            failed = true;
            continue;
        };
        let (outcome, mut detail, mut ports) = run_node(
            &hub,
            &now,
            &flow.id,
            &run_id,
            node,
            trigger.as_ref(),
            &mut results,
        )
        .await;
        // The start node is popped first: a catch-up run's trigger step says so.
        if let Some(label) = start_label.take() {
            detail = label;
        }
        let branching = matches!(
            node.kind,
            NodeKind::If(_)
                | NodeKind::Switch(_)
                | NodeKind::TimeBetween(_)
                | NodeKind::Ask(_)
                | NodeKind::BackupRun(_)
        );
        if matches!(node.kind, NodeKind::BackupRun(_)) {
            ports = backup_ports(&flow, &node.id, ports);
        }
        let shown = (branching && outcome == StepOutcome::Ok).then(|| ports.clone());
        if !hub.record_step(&flow.id, &run_id, step(&node.id, outcome, detail, shown)) {
            return RunStatus::Cancelled;
        }
        if outcome == StepOutcome::Failed {
            failed = true;
            continue;
        }
        // A limit that held the run back ends it here, successfully.
        if matches!(node.kind, NodeKind::Limit(_)) && ports == [LIMITED] {
            continue;
        }
        // A node with no port to follow continues on its port-less edges; a branching node
        // that fired nothing follows nothing (its edges all carry a port).
        let next: Vec<&Node> = if branching {
            ports
                .iter()
                .flat_map(|port| flow.successors(&node.id, Some(port.as_str())))
                .collect()
        } else {
            flow.successors(&node.id, None)
        };
        stack.extend(next.into_iter().rev().map(|next| next.id.clone()));
    }
    if failed {
        RunStatus::Failed
    } else {
        RunStatus::Done
    }
}

/// The detail of a finished `backup.run` step; a warning says what the backup lacks.
fn backup_detail(size: &str, host: &str, file: &str, warning: Option<&str>) -> String {
    let base = format!("Backed up {size} to {host} as {file}");
    match warning {
        Some(reason) => format!("{base}, without Zigbee2MQTT data: {reason}"),
        None => base,
    }
}

/// The ports a `backup.run` step follows: `partial` falls back to `ok` when nothing is wired to
/// it, so a flow written before the port existed goes on as it did.
fn backup_ports(flow: &Flow, node_id: &str, ports: Vec<String>) -> Vec<String> {
    let unwired = ports == [BACKUP_PARTIAL_PORT]
        && flow
            .successors(node_id, Some(BACKUP_PARTIAL_PORT))
            .is_empty();
    if unwired {
        vec![BACKUP_OK_PORT.to_owned()]
    } else {
        ports
    }
}

fn step(node: &str, outcome: StepOutcome, detail: String, ports: Option<Vec<String>>) -> StepView {
    StepView {
        node: node.to_owned(),
        at: clock::now(),
        outcome,
        detail,
        ports,
    }
}

/// `text` with its placeholders filled: `{entity_id}` is the entity's value (a math result of
/// this run wins over an entity of the same name) and `{trigger}` the value that started the
/// run, with the unit and words of its entity. `validate` refused a text that does not parse, so
/// the fallback to the raw text never shows.
fn fill_text(
    hub: &Hub,
    results: &BTreeMap<String, f64>,
    trigger: Option<&TriggerInfo>,
    lang: Lang,
    text: &str,
) -> String {
    let lookup = |id: &str| {
        results
            .get(id)
            .map(|value| Reading {
                value: Scalar::Number(*value),
                unit: None,
                device_class: None,
            })
            .or_else(|| hub.reading(id))
    };
    // The value the trigger carried, with the unit and words of its entity.
    let trigger = trigger.and_then(|t| {
        let value = Scalar::from_json(&t.value)?;
        let mut reading = t
            .entity
            .as_deref()
            .and_then(|id| hub.reading(id))
            .unwrap_or(Reading {
                value: value.clone(),
                unit: None,
                device_class: None,
            });
        reading.value = value;
        Some(reading)
    });
    render(text, lang, &lookup, trigger.as_ref()).unwrap_or_else(|_| text.to_owned())
}

/// The language a spoken sentence's values are written in: French when the house speaks French,
/// English otherwise.
fn speech_lang(hub: &Hub) -> Lang {
    let house = hub
        .tts()
        .and_then(|tts| tts.status().languages.into_iter().next());
    match house.as_ref().and_then(|l| l.code.split('-').next()) {
        Some("fr") => Lang::Fr,
        _ => Lang::En,
    }
}

/// The end of a climate step at engine time `now`: `minutes` from now, or the next local `until`
/// time, as Unix seconds.
fn climate_end(
    hub: &Hub,
    at: chrono::DateTime<chrono::Utc>,
    minutes: Option<u32>,
    until: Option<&str>,
) -> Result<u64, String> {
    match (minutes, until) {
        (Some(minutes), None) => Ok(at.timestamp().max(0) as u64 + u64::from(minutes) * 60),
        (None, Some(hhmm)) => until_after(&at.with_timezone(&hub.zone()), hhmm)
            .map(|end| end.timestamp().max(0) as u64)
            .ok_or_else(|| format!("bad time `{hhmm}`")),
        _ => Err("give exactly one of minutes and until".to_owned()),
    }
}

/// The local `HH:MM` of Unix time `end`, for a step's detail.
fn climate_clock(hub: &Hub, end: u64) -> String {
    let at = chrono::DateTime::from_timestamp(end as i64, 0).unwrap_or_default();
    at.with_timezone(&hub.zone()).format("%H:%M").to_string()
}

/// The outcome of a climate step over several zones: every zone was tried, in list order, and
/// the ones that worked stay applied.
fn climate_outcome(done: Vec<String>, failed: Vec<String>) -> (StepOutcome, String, Vec<String>) {
    if failed.is_empty() {
        (StepOutcome::Ok, done.join(", "), Vec::new())
    } else {
        (StepOutcome::Failed, failed.join("; "), Vec::new())
    }
}

/// `climate.hold`: an override on each zone until the end, the way a person's boost is.
async fn climate_hold(
    hub: &Arc<Hub>,
    now: &Now,
    params: &ClimateHoldParams,
) -> (StepOutcome, String, Vec<String>) {
    let fail = |detail: String| (StepOutcome::Failed, detail, Vec::new());
    let at = now();
    let end = match climate_end(hub, at, params.minutes, params.until.as_deref()) {
        Ok(end) => end,
        Err(err) => return fail(err),
    };
    let mode = match params.mode.as_deref() {
        None => None,
        Some("heat") => Some(nidoo_climate::doc::Mode::Heat),
        Some("cool") => Some(nidoo_climate::doc::Mode::Cool),
        Some(other) => return fail(format!("unknown mode `{other}`")),
    };
    let (setpoint, label) = match params.setpoint {
        ClimateSetpoint::Temp(t) => (nidoo_climate::doc::Setpoint::Temp(t), format!("{t} °C")),
        ClimateSetpoint::Off => (nidoo_climate::doc::Setpoint::Off, "off".to_owned()),
    };
    let request = nidoo_climate::view::OverrideRequest {
        setpoint: Some(setpoint),
        mode,
        until: Some(end),
        ..Default::default()
    };
    let at = at.timestamp().max(0) as u64;
    let (mut done, mut failed) = (Vec::new(), Vec::new());
    for zone in &params.zones {
        let result = match request.clone().into_override(at) {
            Ok(ov) => hub.set_climate_override(zone, ov).await,
            Err(why) => Err(HubError::Rejected(why)),
        };
        match result {
            Ok(()) => done.push(format!("{zone} {label} until {}", climate_clock(hub, end))),
            Err(err) => failed.push(format!("{zone}: {err}")),
        }
    }
    climate_outcome(done, failed)
}

/// `climate.eco`: each zone in eco until the end.
async fn climate_eco(
    hub: &Arc<Hub>,
    now: &Now,
    params: &ClimateEcoParams,
) -> (StepOutcome, String, Vec<String>) {
    let at = now();
    let end = match climate_end(hub, at, params.minutes, params.until.as_deref()) {
        Ok(end) => end,
        Err(err) => return (StepOutcome::Failed, err, Vec::new()),
    };
    let at = at.timestamp().max(0) as u64;
    let (mut done, mut failed) = (Vec::new(), Vec::new());
    for zone in &params.zones {
        let result = match (nidoo_climate::view::RoomEcoRequest { until: end }).into_room_eco(at) {
            Ok(eco) => hub.set_climate_eco(zone, eco).await,
            Err(why) => Err(HubError::Rejected(why)),
        };
        match result {
            Ok(()) => done.push(format!("{zone} eco until {}", climate_clock(hub, end))),
            Err(err) => failed.push(format!("{zone}: {err}")),
        }
    }
    climate_outcome(done, failed)
}

/// Runs one node: its outcome, the detail to record and the output port to follow.
async fn run_node(
    hub: &Arc<Hub>,
    now: &Now,
    flow_id: &str,
    run_id: &str,
    node: &Node,
    trigger: Option<&TriggerInfo>,
    results: &mut BTreeMap<String, f64>,
) -> (StepOutcome, String, Vec<String>) {
    let failed = |detail: String| (StepOutcome::Failed, detail, Vec::new());
    match &node.kind {
        NodeKind::TriggerState(params) => {
            let detail = match trigger {
                Some(trigger) => format!("{} = {}", params.entity, trigger.value),
                None => "manual".to_owned(),
            };
            (StepOutcome::Ok, detail, Vec::new())
        }
        NodeKind::TriggerSignal(params) => {
            let detail = match trigger {
                Some(trigger) => format!("{} sent {}", params.entity, trigger.value),
                None => "manual".to_owned(),
            };
            (StepOutcome::Ok, detail, Vec::new())
        }
        NodeKind::TriggerSilent(params) => {
            let detail = match trigger {
                Some(_) => format!("{} silent for {}", params.entity, params.for_),
                None => "manual".to_owned(),
            };
            (StepOutcome::Ok, detail, Vec::new())
        }
        NodeKind::Limit(params) => match window(&params.per) {
            Ok(per) => match hub.limit_pass(flow_id, &node.id, params.max, per, now()) {
                Ok(passes) => (
                    StepOutcome::Ok,
                    format!(
                        "passed ({passes} of {} in the last {})",
                        params.max, params.per
                    ),
                    Vec::new(),
                ),
                // `execute` ends the run on this port.
                Err(_) => (
                    StepOutcome::Ok,
                    format!("limited: {} in the last {}", params.max, params.per),
                    vec![LIMITED.to_owned()],
                ),
            },
            Err(err) => failed(err),
        },
        NodeKind::TriggerSchedule(params) => (
            StepOutcome::Ok,
            format!("schedule {}", params.cron),
            Vec::new(),
        ),
        NodeKind::TriggerManual(p) => {
            let detail = match &p.value {
                Some(value) => format!("manual: {}", value.to_json()),
                None => "manual".to_owned(),
            };
            (StepOutcome::Ok, detail, Vec::new())
        }
        NodeKind::TriggerOffPeak(params) => (
            StepOutcome::Ok,
            format!("off-peak {}", params.edge.wire_name()),
            Vec::new(),
        ),
        NodeKind::TriggerAlarm(_) => {
            let detail = match trigger {
                Some(trigger) => format!("alarm {}", trigger.value),
                None => "manual".to_owned(),
            };
            (StepOutcome::Ok, detail, Vec::new())
        }
        NodeKind::TriggerSun(params) => (
            StepOutcome::Ok,
            format!(
                "sun {} {:+}m",
                format!("{:?}", params.which).to_lowercase(),
                params.offset()
            ),
            Vec::new(),
        ),
        NodeKind::TriggerWakeUp(params) => (
            StepOutcome::Ok,
            format!("wake-up -{}m", params.before_minutes),
            Vec::new(),
        ),
        NodeKind::ClimateHold(params) => climate_hold(hub, now, params).await,
        NodeKind::ClimateEco(params) => climate_eco(hub, now, params).await,
        NodeKind::FlowEnable(params) => {
            let target = params.flow.as_deref().unwrap_or(flow_id);
            if hub.flow(target).is_none() {
                return failed(format!("unknown flow `{target}`"));
            }
            let result = if target == flow_id {
                hub.set_flow_enabled_by_run(target, params.enabled, run_id)
                    .await
            } else {
                hub.set_flow_enabled(target, params.enabled).await
            };
            match result {
                Ok(_) => {
                    let verb = if params.enabled {
                        "enabled"
                    } else {
                        "disabled"
                    };
                    (StepOutcome::Ok, format!("{verb} {target}"), Vec::new())
                }
                Err(err) => failed(err.to_string()),
            }
        }
        NodeKind::FlowLabel(params) => {
            match hub
                .switch_flow_label(&params.label, params.action, Some((flow_id, run_id)))
                .await
            {
                Ok(result) => (StepOutcome::Ok, switch_detail(&result), Vec::new()),
                // The reason alone: "unknown label `x`" reads better than "rejected: …".
                Err(HubError::Rejected(reason)) => failed(reason),
                Err(err) => failed(err.to_string()),
            }
        }
        NodeKind::BackupRun(params) => {
            let limit = std::time::Duration::from_secs(60 * u64::from(params.timeout_minutes));
            // The limit is the job's own when this step starts it; when it joins a running job
            // it is how long to wait. Dropping the wait (the run cancelled, the limit reached)
            // leaves the backup running.
            let waited = tokio::time::timeout(limit, hub.backup_and_wait(limit))
                .await
                .unwrap_or(Err(crate::backup::BackupError::TimeLimit));
            match waited {
                Ok(done) => (
                    StepOutcome::Ok,
                    backup_detail(
                        &crate::backup::job::human_size(done.size),
                        &done.host,
                        &done.file,
                        done.z2m_warning.as_deref(),
                    ),
                    // `execute` turns `partial` into `ok` when nothing is wired to it.
                    vec![
                        if done.z2m_warning.is_some() {
                            BACKUP_PARTIAL_PORT
                        } else {
                            BACKUP_OK_PORT
                        }
                        .to_owned(),
                    ],
                ),
                Err(err) => (
                    StepOutcome::Ok,
                    err.to_string(),
                    vec![BACKUP_FAILED_PORT.to_owned()],
                ),
            }
        }
        NodeKind::Math(params) => match run_math(hub, now, params, trigger, results).await {
            Ok(detail) => (StepOutcome::Ok, detail, Vec::new()),
            Err(reason) => failed(reason),
        },
        NodeKind::TimeBetween(params) => {
            let (Ok(from), Ok(to)) = (parse_clock(&params.from), parse_clock(&params.to)) else {
                return failed("time is not HH:MM".to_owned());
            };
            let local = now().with_timezone(&hub.zone());
            let minute = u16::try_from(local.hour() * 60 + local.minute()).unwrap_or(0);
            let port = if in_time_range(from, to, minute) {
                "in"
            } else {
                "out"
            };
            (
                StepOutcome::Ok,
                format!("{:02}:{:02} → {port}", local.hour(), local.minute()),
                vec![port.to_owned()],
            )
        }
        NodeKind::Switch(params) => {
            let left = match node_left(
                hub,
                now,
                params.entity.as_deref(),
                params.field.as_deref(),
                params.average.as_deref(),
                params.result.as_deref(),
                trigger,
                results,
            )
            .await
            {
                Ok(left) => left,
                Err(reason) => return failed(reason),
            };
            let exits = match switch_exits(params, left.as_ref()) {
                Ok(exits) => exits,
                Err(err) => return failed(err.to_string()),
            };
            let taken = if exits.is_empty() {
                "none".to_owned()
            } else {
                exits.join(",")
            };
            let detail = match (&left, params.average.as_deref()) {
                (None, _) => format!("unknown → {taken}"),
                (Some(Scalar::Number(mean)), Some(window)) => {
                    format!("average {window} = {mean:.1} → {taken}")
                }
                (Some(value), _) => format!("{} → {taken}", scalar_text(value)),
            };
            (StepOutcome::Ok, detail, exits)
        }
        NodeKind::Notify(params) => {
            let lang = params.lang.unwrap_or_default();
            let fill = |text: &str| fill_text(hub, results, trigger, lang, text);
            let title = fill(&params.title);
            let mut alert = serde_json::json!({
                "v": 1,
                "id": params
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("flow-{flow_id}-{run_id}")),
                "title": title,
                "channel": params.channel.as_deref().unwrap_or("misc"),
                "severity": params.severity.as_deref().unwrap_or("info"),
            });
            for (key, value) in [
                ("body", params.body.as_deref().map(fill)),
                ("entity_id", params.entity_id.clone()),
                ("image_url", params.image_url.clone()),
            ] {
                if let Some(value) = value {
                    alert[key] = Value::String(value);
                }
            }
            if let Some(silent) = params.silent {
                alert["silent"] = Value::Bool(silent);
            }
            match hub.notify("flow", &alert) {
                Ok(id) => (StepOutcome::Ok, format!("sent {id}: {title}"), Vec::new()),
                Err(rejected) => failed(rejected.reason),
            }
        }
        NodeKind::Ask(params) => run_ask(hub, results, trigger, params).await,
        NodeKind::Speak(params) => {
            let text = fill_text(hub, results, trigger, speech_lang(hub), &params.text);
            let target = match &params.room {
                Some(room) => SpeakTarget::Room(room.clone()),
                None => SpeakTarget::All,
            };
            match hub.speak_in(target, &text, None, None).await {
                Ok(n) => (
                    StepOutcome::Ok,
                    format!("spoke on {n} satellite(s): {text}"),
                    Vec::new(),
                ),
                Err(err) => failed(err.to_string()),
            }
        }
        NodeKind::If(params) => {
            if params.op == Op::IsOffPeak {
                let off_peak = hub.off_peak_now(now_in(now, hub.zone())).off_peak;
                let port = if off_peak { "true" } else { "false" };
                return (
                    StepOutcome::Ok,
                    format!("is_off_peak → {off_peak}"),
                    vec![port.to_owned()],
                );
            }
            if params.op == Op::IsDark {
                let (lat, lon) = hub.house_point();
                let dark = is_dark_with_offset(lat, lon, now(), params.dark_offset());
                let port = if dark { "true" } else { "false" };
                return (
                    StepOutcome::Ok,
                    format!("is_dark → {dark}"),
                    vec![port.to_owned()],
                );
            }
            let Some(right) = Scalar::from_json(params.value.as_ref().unwrap_or(&Value::Null))
            else {
                return failed("value is not a scalar".to_owned());
            };
            let left = match node_left(
                hub,
                now,
                params.entity.as_deref(),
                params.field.as_deref(),
                params.average.as_deref(),
                params.result.as_deref(),
                trigger,
                results,
            )
            .await
            {
                Ok(left) => left,
                Err(reason) => return failed(reason),
            };
            match (&left, params.missing) {
                (None, Some(Missing::True)) => {
                    return (
                        StepOutcome::Ok,
                        "missing → true".to_owned(),
                        vec!["true".to_owned()],
                    );
                }
                (None, Some(Missing::False)) => {
                    return (
                        StepOutcome::Ok,
                        "missing → false".to_owned(),
                        vec!["false".to_owned()],
                    );
                }
                _ => {}
            }
            let shown = |outcome: bool| match (params.average.as_deref(), &left) {
                (Some(window), Some(Scalar::Number(mean))) => {
                    format!("average {window} = {mean:.1} → {outcome}")
                }
                // The value compared, so a run's history shows why it went that way.
                (_, Some(value)) => format!("{} → {outcome}", scalar_text(value)),
                (_, None) => format!("unknown → {outcome}"),
            };
            match compare(left.as_ref(), params.op, &right) {
                Ok(true) => (StepOutcome::Ok, shown(true), vec!["true".to_owned()]),
                Ok(false) => (StepOutcome::Ok, shown(false), vec!["false".to_owned()]),
                Err(err) => failed(err.to_string()),
            }
        }
        NodeKind::Delay(params) => match params.bounds() {
            Ok((low, None)) => {
                tokio::time::sleep(low).await;
                (
                    StepOutcome::Ok,
                    format!("waited {}", params.for_),
                    Vec::new(),
                )
            }
            Ok((low, Some(high))) => {
                let duration = pick_duration(low, Some(high), clock::random_roll());
                tokio::time::sleep(duration).await;
                let detail = format!(
                    "waited {} ({} to {})",
                    format_duration(duration),
                    params.for_,
                    params.up_to.as_deref().unwrap_or_default()
                );
                (StepOutcome::Ok, detail, Vec::new())
            }
            Err(reason) => failed(reason),
        },
        NodeKind::Command(params) => {
            let mut kept_in_range = false;
            let command = match &params.value_from {
                Some(name) => {
                    let value = match result_of(results, name) {
                        Ok(value) => value,
                        Err(reason) => return failed(reason),
                    };
                    match command_with_number(&params.command, value) {
                        // A number entity takes what fits its range and step, as `save_to` does.
                        Some(Command::SetNumber { value }) => {
                            let Some((min, max, step)) = hub.number_limits(&params.entity) else {
                                return failed(format!("unknown entity `{}`", params.entity));
                            };
                            let (fitted, clamped) = fit_to_entity(value, min, max, step);
                            kept_in_range = clamped;
                            Command::SetNumber { value: fitted }
                        }
                        Some(command) => command,
                        None => return failed("this command takes no number".to_owned()),
                    }
                }
                None => params.command.clone(),
            };
            match hub.send_command(&params.entity, command).await {
                Ok(response) if response.accepted => {
                    let detail = if kept_in_range {
                        "accepted (kept in range)"
                    } else {
                        "accepted"
                    };
                    (StepOutcome::Ok, detail.to_owned(), Vec::new())
                }
                Ok(response) => failed(
                    response
                        .reason
                        .unwrap_or_else(|| "command not accepted".to_owned()),
                ),
                Err(err) => failed(err.to_string()),
            }
        }
    }
}

/// The result a math card of this run stored under `name`.
fn result_of(results: &BTreeMap<String, f64>, name: &str) -> Result<f64, String> {
    results
        .get(name)
        .copied()
        .ok_or_else(|| format!("no result {name} in this run"))
}

/// Below this much time left, a not-understood answer gets no retry.
const ASK_RETRY_MIN: std::time::Duration = std::time::Duration::from_secs(5);

/// Runs an ask card: says the question, listens, and picks the output port of the answer
/// heard. A heard answer that matches nothing gets one retry on the satellite that heard it.
/// Every way of not getting an answer leaves by `none` with the step Ok.
async fn run_ask(
    hub: &Hub,
    results: &mut BTreeMap<String, f64>,
    trigger: Option<&TriggerInfo>,
    params: &AskParams,
) -> (StepOutcome, String, Vec<String>) {
    let none = |detail: String| (StepOutcome::Ok, detail, vec![ASK_NONE_PORT.to_owned()]);
    let lang = speech_lang(hub);
    let text = fill_text(hub, results, trigger, lang, &params.text);
    let target = match &params.room {
        Some(room) => SpeakTarget::Room(room.clone()),
        None => SpeakTarget::All,
    };
    // The whole step, retry included, stays within one limit.
    let deadline = tokio::time::Instant::now() + ASK_LIMIT;
    let (sid, first) = match hub.ask_aloud_within(target, &text, ASK_LIMIT).await {
        AskOutcome::Heard { sid, text } => (sid, text),
        AskOutcome::Silence => return none(format!("asked \"{text}\" → no answer")),
        AskOutcome::NoSatellite(reason) => return none(format!("ask: {reason}")),
        AskOutcome::Off => return none("ask: speech input is off".to_owned()),
    };
    let mut heard = vec![first];
    let mut found = match_answer(&params.answers, &heard[0]);
    let mut retry_failed = None;
    if found.is_none() {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left >= ASK_RETRY_MIN {
            let again = format!("{} {text}", not_understood(lang));
            match hub
                .ask_aloud_within(SpeakTarget::Satellite(sid), &again, left)
                .await
            {
                AskOutcome::Heard { text: second, .. } => {
                    found = match_answer(&params.answers, &second);
                    heard.push(second);
                }
                AskOutcome::Silence => {}
                AskOutcome::NoSatellite(reason) => retry_failed = Some(reason),
                AskOutcome::Off => retry_failed = Some("speech input is off".to_owned()),
            }
        }
    }
    let said = heard
        .iter()
        .map(|h| format!("\"{h}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let Some(index) = found else {
        let retry = retry_failed.map_or_else(String::new, |why| format!(" → retry: {why}"));
        return none(format!("asked \"{text}\" → heard {said}{retry} → none"));
    };
    let said_number = heard.last().and_then(|h| first_number(h));
    let number = said_number.or(params.default);
    let mut detail = format!(
        "asked \"{text}\" → heard {said} → {}",
        params.answers[index].label
    );
    if let (Some(name), Some(n)) = (&params.name, number) {
        results.insert(name.clone(), n);
        if said_number.is_some() {
            detail.push_str(&format!(" ({})", number_text(n)));
        } else {
            detail.push_str(&format!(" (default {})", number_text(n)));
        }
    }
    (StepOutcome::Ok, detail, vec![(index + 1).to_string()])
}

/// A number as shown in a step detail: at most 2 decimals, trailing zeros trimmed.
fn number_text(value: f64) -> String {
    let text = format!("{value:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

/// Runs a math card: reads its inputs, evaluates the formula, keeps the result under the
/// card's name for the rest of the run and, with `save_to`, writes it to that number entity
/// (rounded to its step, kept in its range). Returns the step detail.
async fn run_math(
    hub: &Hub,
    now: &Now,
    params: &MathParams,
    trigger: Option<&TriggerInfo>,
    results: &mut BTreeMap<String, f64>,
) -> Result<String, String> {
    let formula = parse_formula(&params.formula).map_err(|err| err.to_string())?;
    // Each input is read now but fails the step only if the formula reads it (an unused input
    // or one in an untaken `if` branch never does), so the editor's preview and the hub agree.
    let mut inputs: Vec<(&str, Result<f64, String>)> = Vec::new();
    for input in &params.inputs {
        let value = match &input.average {
            Some(window) => window_average(hub, &input.entity, None, window, now())
                .await
                .map(Scalar::Number),
            None => hub.entity_scalar(&input.entity),
        };
        let number = match value {
            Some(Scalar::Number(n)) => Ok(n),
            Some(Scalar::Bool(b)) => Ok(f64::from(u8::from(b))),
            Some(Scalar::Text(_)) => Err(format!("{} is not a number", input.name)),
            None => Err(format!("{} is unknown", input.name)),
        };
        inputs.push((input.name.as_str(), number));
    }
    let trigger_number = trigger.and_then(|t| match &t.value {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        _ => None,
    });
    let value = formula.eval(&|name| {
        if let Some((_, value)) = inputs.iter().find(|(input, _)| *input == name) {
            value.clone()
        } else if name == "trigger" {
            trigger_number.ok_or_else(|| "trigger has no number".to_owned())
        } else {
            result_of(results, name)
        }
    })?;
    results.insert(params.name.clone(), value);
    let shown = inputs
        .iter()
        .map(|(name, value)| match value {
            Ok(value) => format!("{name}={}", number_text(*value)),
            Err(_) => format!("{name}=?"),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut detail = if shown.is_empty() {
        String::new()
    } else {
        format!("{shown} → ")
    };
    detail.push_str(&format!("{} = {}", params.name, number_text(value)));
    let Some(target) = &params.save_to else {
        return Ok(detail);
    };
    let Some((min, max, step)) = hub.number_limits(target) else {
        return Err(format!("{detail} → unknown entity `{target}`"));
    };
    let (saved, clamped) = fit_to_entity(value, min, max, step);
    detail.push_str(&format!(" → {target} = {}", number_text(saved)));
    if clamped {
        detail.push_str(" (kept in range)");
    }
    let refusal = match hub
        .send_command(target, Command::SetNumber { value: saved })
        .await
    {
        Ok(response) if response.accepted => return Ok(detail),
        Ok(response) => response
            .reason
            .unwrap_or_else(|| "command not accepted".to_owned()),
        Err(err) => err.to_string(),
    };
    Err(format!("{detail} → {refusal}"))
}

/// A scalar as shown in a step detail.
fn scalar_text(value: &Scalar) -> String {
    match value {
        Scalar::Bool(b) => b.to_string(),
        Scalar::Number(n) => n.to_string(),
        Scalar::Text(t) => t.clone(),
    }
}

/// The value an `if` or `switch` node compares: the math result `result`, else `field` of
/// `entity` (else the value that fired the trigger), as its current value or, with `average`,
/// the mean of its readings over that window. `Ok(None)` when the value is unknown; `Err` carries the failure reason.
#[allow(clippy::too_many_arguments)]
async fn node_left(
    hub: &Hub,
    now: &Now,
    entity: Option<&str>,
    field: Option<&str>,
    average: Option<&str>,
    result: Option<&str>,
    trigger: Option<&TriggerInfo>,
    results: &BTreeMap<String, f64>,
) -> Result<Option<Scalar>, String> {
    if let Some(name) = result {
        return result_of(results, name).map(|value| Some(Scalar::Number(value)));
    }
    let (entity, field) = match (entity, trigger) {
        (Some(entity), _) => (entity, field),
        (None, Some(trigger)) => match trigger.entity.as_deref() {
            Some(entity) => (entity, trigger.field.as_deref()),
            // A Manual card's value: nothing to read, nothing to average.
            None if average.is_some() => return Err("no entity to average".to_owned()),
            None => return Ok(Scalar::from_json(&trigger.value)),
        },
        (None, None) => return Err("no triggering entity".to_owned()),
    };
    Ok(match average {
        Some(window) => window_average(hub, entity, field, window, now())
            .await
            .map(Scalar::Number),
        None => hub.entity_value(entity, field),
    })
}

/// The entities the `if` and `switch` nodes of `flows` average, each with its longest window in
/// microseconds. A node without `entity` averages the entities its flow's triggers watch.
fn averaged_windows(flows: &[Flow]) -> BTreeMap<String, i64> {
    let mut windows = BTreeMap::new();
    for flow in flows {
        let triggers: Vec<&String> = flow
            .nodes
            .iter()
            .filter_map(|node| match &node.kind {
                NodeKind::TriggerState(params) => Some(&params.entity),
                _ => None,
            })
            .collect();
        for node in &flow.nodes {
            if let NodeKind::Math(params) = &node.kind {
                for input in &params.inputs {
                    let Some(window) = input
                        .average
                        .as_deref()
                        .and_then(|w| parse_duration(w).ok())
                    else {
                        continue;
                    };
                    let micros = i64::try_from(window.as_micros()).unwrap_or(i64::MAX);
                    let longest = windows.entry(input.entity.clone()).or_insert(micros);
                    *longest = (*longest).max(micros);
                }
                continue;
            }
            let (entity, average) = match &node.kind {
                NodeKind::If(params) => (&params.entity, &params.average),
                NodeKind::Switch(params) => (&params.entity, &params.average),
                _ => continue,
            };
            let Some(window) = average.as_deref().and_then(|w| parse_duration(w).ok()) else {
                continue;
            };
            let micros = i64::try_from(window.as_micros()).unwrap_or(i64::MAX);
            let entities = match entity {
                Some(entity) => vec![entity],
                None => triggers.clone(),
            };
            for entity in entities {
                let longest = windows.entry(entity.clone()).or_insert(micros);
                *longest = (*longest).max(micros);
            }
        }
    }
    windows
}

/// The mean of the readings of `entity` (its recorded `field` when set) over the `window` ending
/// at `now`: the ones the hub kept, newest included, with history filling the part of the window
/// before them (after a restart, or a flow saved since). With no reading in the window the value
/// held throughout, the current one, is the mean. `None` when the entity is unknown, unavailable or not a number.
async fn window_average(
    hub: &Hub,
    entity: &str,
    field: Option<&str>,
    window: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<f64> {
    // A field is averaged only when it is the entity's recorded number, so its current value is
    // the number the readings and the history hold.
    let Scalar::Number(current) = hub.entity_value(entity, field)? else {
        return None;
    };
    let window = parse_duration(window).ok()?;
    let end = now.timestamp_micros();
    let start = end - i64::try_from(window.as_micros()).ok()?;
    let recent = hub.readings_since(entity, start);
    let recorded = match hub.history().map(|history| history.db()) {
        Some(Ok(db)) => match db.samples(start, end + 1, Some(entity)).await {
            Ok(rows) => rows.into_iter().map(|(_, sample)| sample).collect(),
            Err(err) => {
                tracing::warn!(%entity, %err, "average: history read failed");
                Vec::new()
            }
        },
        _ => Vec::<Sample>::new(),
    };
    Some(readings_mean(&recorded, &recent, start).unwrap_or(current))
}

#[cfg(test)]
mod tests;
