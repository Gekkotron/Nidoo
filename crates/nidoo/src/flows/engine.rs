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
use nidoo_core::flow::math::{fit_to_entity, parse_formula};
use nidoo_core::flow::sun::is_dark_with_offset;
use nidoo_core::flow::template::{Lang, render};
use nidoo_core::flow::{
    ASK_NONE_PORT, AskParams, BACKUP_FAILED_PORT, BACKUP_OK_PORT, BACKUP_PARTIAL_PORT, Flow,
    FlowMode, MathParams, Missing, Node, NodeKind, OffPeakEdge, Op, Reading, Scalar, Schedule,
    command_with_number, compare, field_scalar, format_duration, headline_scalar, in_time_range,
    parse_clock, parse_duration, pick_duration, switch_exits, window,
};
use nidoo_core::history::buckets::{Sample, readings_mean};
use nidoo_core::model::{Command, StateValue};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};
use tokio::task::{AbortHandle, JoinSet};

use super::EngineRequest;
use super::labels::switch_detail;
use super::schedule::{Armed, Now, now_in, rearm, timer_clock};
use super::sun::{ArmedSun, next_sun_fire};
use crate::assist::stt::ask::{ASK_LIMIT, AskOutcome};
use crate::clock;
use crate::home::OffPeakNow;
use crate::hub::{Hub, HubError, StateTouched};
use crate::sound::SpeakTarget;
use nidoo_api::ConnectionStatus;

/// How long after the link comes back the off-peak base is seeded when no device report did.
const OFF_PEAK_SETTLE: chrono::Duration = chrono::Duration::seconds(60);

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
        sun_timers: BTreeMap::new(),
        off_peak_base: None,
        off_peak_timer: None,
        off_peak_settle: None,
    };
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
                engine.tick();
                engine.tick_silents();
                engine.tick_suns();
                engine.tick_off_peak();
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
    /// Armed sun nodes, keyed `"{flow}/{node}"`.
    sun_timers: BTreeMap<String, ArmedSun>,
    /// The off-peak answer `trigger.off_peak` compares against; `None` until seeded. Seeding
    /// never fires: boot, a reconnect replay, a flow save or a settings change only set it.
    off_peak_base: Option<bool>,
    /// The next off-peak window start or end, when the answer may change by the clock.
    off_peak_timer: Option<chrono::DateTime<chrono_tz::Tz>>,
    /// A minute after the link comes back: seeds the base if no device report did by then.
    off_peak_settle: Option<chrono::DateTime<chrono_tz::Tz>>,
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
        self.arm_suns();
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
            .chain(self.off_peak_timer.map(|at| at.to_utc()))
            .chain(self.off_peak_settle.map(|at| at.to_utc()))
            .chain(silent)
            .min()?;
        let wait = (at - (self.now)())
            .to_std()
            .unwrap_or(std::time::Duration::ZERO);
        Some(tokio::time::Instant::now() + wait)
    }

    /// Fires every schedule whose tick is due and re-arms it for the next one. A tick while the
    /// broker link is not online is recorded as skipped instead: commands could not go out.
    fn tick(&mut self) {
        let now = now_in(&self.now, self.zone);
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
            if online {
                self.fire(flow, node, None);
            } else {
                self.hub
                    .add_skipped_run_because(&flow.id, &node, "the broker link is offline");
            }
        }
    }

    /// Fires every sun node whose event is due and re-arms it for the next one, skipping while
    /// the broker link is not online like `tick`. Events missed while the hub was busy are
    /// never replayed: the next one is searched from now.
    fn tick_suns(&mut self) {
        let now = now_in(&self.now, self.zone);
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
            if online {
                self.fire(flow, node, None);
            } else {
                self.hub
                    .add_skipped_run_because(&flow.id, &node, "the broker link is offline");
            }
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
            }
            WsEvent::ButtonPressed { id } => self.button_pressed(&id),
            WsEvent::SignalReceived { id, signal } => self.signal_received(&id, &signal),
            // The broker is about to replay retained states: forget what we knew so the replay
            // is only recorded, never acted on.
            WsEvent::ConnectionChanged(status) if status != ConnectionStatus::Online => {
                self.known.clear();
                self.confirmed_unknown.clear();
                self.off_peak_lost();
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
        for key in keys {
            let next = watched_value(&component, state, key.1.as_deref());
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
                    let NodeKind::TriggerState(params) = &node.kind else {
                        continue;
                    };
                    if params.entity == id
                        && params.field == key.1
                        && !fires.iter().any(|(f, _, _)| f.id == flow.id)
                        && filter_matches(params.to.as_ref(), &next)
                        && match &previous {
                            Some(previous) => filter_matches(params.from.as_ref(), previous),
                            None => params.from.is_none() && params.to.is_some(),
                        }
                    {
                        fires.push((flow.clone(), node.id.clone(), trigger.clone()));
                    }
                }
            }
        }
        for (flow, node, trigger) in fires {
            self.fire(flow, node, Some(trigger));
        }
    }

    /// Starts a run for a satisfied trigger, honouring the flow's mode when one is active.
    fn fire(&mut self, flow: Flow, node_id: String, trigger: Option<TriggerInfo>) {
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
        self.spawn(flow, run.id, node_id, trigger);
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
                self.spawn(flow, run_id, start, trigger);
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

    fn spawn(&mut self, flow: Flow, run_id: String, start: String, trigger: Option<TriggerInfo>) {
        let hub = Arc::clone(&self.hub);
        let now = Arc::clone(&self.now);
        let (flow_id, task_flow, task_run) = (flow.id.clone(), flow.clone(), run_id.clone());
        let abort = self.tasks.spawn(async move {
            let status = execute(hub, now, task_flow, task_run.clone(), start, trigger).await;
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

/// `eq` semantics of `compare`: the filter must be a scalar equal to the value, type included.
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
/// successors in edge order. The first failed step ends the run.
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
) -> RunStatus {
    let mut stack = vec![start];
    // The results of the run's math cards, by name; every branch reads and writes this one map.
    let mut results: BTreeMap<String, f64> = BTreeMap::new();
    while let Some(id) = stack.pop() {
        let Some(node) = flow.node(&id) else {
            let step = step(&id, StepOutcome::Failed, "unknown node".to_owned(), None);
            if !hub.record_step(&flow.id, &run_id, step) {
                return RunStatus::Cancelled;
            }
            return RunStatus::Failed;
        };
        let (outcome, detail, mut ports) = run_node(
            &hub,
            &now,
            &flow.id,
            &run_id,
            node,
            trigger.as_ref(),
            &mut results,
        )
        .await;
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
            return RunStatus::Failed;
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
    RunStatus::Done
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

/// Runs one node: its outcome, the detail to record and the output port to follow.
async fn run_node(
    hub: &Hub,
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
        NodeKind::TriggerSun(params) => (
            StepOutcome::Ok,
            format!(
                "sun {} {:+}m",
                format!("{:?}", params.which).to_lowercase(),
                params.offset()
            ),
            Vec::new(),
        ),
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
                _ => outcome.to_string(),
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
mod tests {
    use super::*;
    use chrono::TimeZone;
    use nidoo_core::command::Publish;
    use serde_json::json;

    fn sun_flow(id: &str, enabled: bool) -> String {
        serde_json::json!({
            "id": id, "name": id, "enabled": enabled, "mode": "single",
            "nodes": [{ "id": "sun", "type": "trigger.sun", "params": { "which": "sunrise" } }],
            "edges": []
        })
        .to_string()
    }

    /// An engine whose clock reads 2026-01-01 00:00 UTC, on a hub at (`lat`, `lon`).
    fn engine_at(lat: f64, lon: f64, zone: &str) -> Engine {
        let mut cfg = crate::config::Config::default();
        cfg.flows.timezone = Some(zone.to_owned());
        cfg.flows.latitude = Some(lat);
        cfg.flows.longitude = Some(lon);
        let (tx, _rx) = mpsc::channel::<Publish>(8);
        let hub = Hub::new(&cfg, tx);
        let fixed = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Engine {
            zone: hub.zone(),
            hub,
            flows: Vec::new(),
            known: BTreeMap::new(),
            watched: BTreeSet::new(),
            confirmed_unknown: BTreeSet::new(),
            active: BTreeMap::new(),
            keep: BTreeSet::new(),
            tasks: JoinSet::new(),
            now: Arc::new(move || fixed),
            schedules: BTreeMap::new(),
            silents: BTreeMap::new(),
            silent_epoch: 0,
            sun_timers: BTreeMap::new(),
            off_peak_base: None,
            off_peak_timer: None,
            off_peak_settle: None,
        }
    }

    #[tokio::test]
    async fn polar_night_flow_stays_disarmed_across_a_toggle() {
        let mut engine = engine_at(78.22, 15.65, "Europe/Oslo");
        for enabled in [true, false, true] {
            engine.hub.ingest(
                "nidoo/automation/polar",
                sun_flow("polar", enabled).as_bytes(),
            );
            engine.reload();
            assert!(engine.sun_timers.is_empty(), "enabled={enabled}");
        }
    }

    #[tokio::test]
    async fn temperate_flow_arms_on_the_same_date_and_disarms_when_disabled() {
        let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
        engine
            .hub
            .ingest("nidoo/automation/day", sun_flow("day", true).as_bytes());
        engine.reload();
        assert_eq!(engine.sun_timers.keys().collect::<Vec<_>>(), ["day/sun"]);
        engine
            .hub
            .ingest("nidoo/automation/day", sun_flow("day", false).as_bytes());
        engine.reload();
        assert!(engine.sun_timers.is_empty());
    }

    /// A Paris engine, online, with a text sensor `tariff` (availability on its own topic) and
    /// a clock the test moves. Returns the engine, the entity id and the clock.
    fn tariff_engine(
        utc: (i32, u32, u32, u32, u32),
    ) -> (
        Engine,
        String,
        Arc<std::sync::Mutex<chrono::DateTime<chrono::Utc>>>,
    ) {
        let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
        let (y, mo, d, h, mi) = utc;
        let clock = Arc::new(std::sync::Mutex::new(
            chrono::Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap(),
        ));
        let read = Arc::clone(&clock);
        engine.now = Arc::new(move || *read.lock().unwrap());
        engine.hub.ingest(
            "nidoo/sensor/meter/tariff/config",
            br#"{"unique_id":"tariff","state_topic":"test/tariff","availability_topic":"test/tariff/avail"}"#,
        );
        engine.hub.set_connection(ConnectionStatus::Online);
        let id = engine.hub.snapshot().entities[0].id.clone();
        (engine, id, clock)
    }

    fn off_peak_flow(engine: &Engine, id: &str, edge: &str) {
        let flow = serde_json::json!({
            "id": id, "name": id, "enabled": true, "mode": "single",
            "nodes": [{ "id": "t", "type": "trigger.off_peak", "params": { "edge": edge } }],
            "edges": []
        });
        engine.hub.ingest(
            &format!("nidoo/automation/{id}"),
            flow.to_string().as_bytes(),
        );
    }

    /// Stores home settings with the device (when `device`) and the windows, and tells the
    /// engine as the hub's broadcast would.
    fn off_peak_settings(engine: &mut Engine, device: Option<&str>, windows: &[(&str, &str)]) {
        let windows: Vec<_> = windows
            .iter()
            .map(|(start, end)| serde_json::json!({ "start": start, "end": end }))
            .collect();
        let mut doc = serde_json::json!({ "off_peak": windows });
        if let Some(id) = device {
            doc["off_peak_device"] = serde_json::json!({ "entity": id, "values": ["OFF"] });
        }
        engine.hub.ingest_home(doc.to_string().as_bytes());
        engine.handle_event(WsEvent::HomeSettingsChanged {
            settings: engine.hub.home_settings(),
        });
    }

    /// Publishes `payload` on the tariff's `topic` and hands the engine the change event.
    fn tariff_report(engine: &mut Engine, id: &str, topic: &str, payload: &str) {
        engine.hub.ingest(topic, payload.as_bytes());
        let state = engine.hub.read().state(id).cloned().expect("tariff state");
        engine.handle_event(WsEvent::StateChanged {
            id: id.to_owned(),
            state,
        });
    }

    fn run_count(engine: &Engine, flow: &str) -> usize {
        engine.hub.flow(flow).map_or(0, |f| f.runs.len())
    }

    #[tokio::test]
    async fn device_change_fires_start_then_end() {
        let (mut engine, id, _) = tariff_engine((2026, 1, 1, 12, 0));
        off_peak_flow(&engine, "on", "start");
        off_peak_flow(&engine, "off", "end");
        engine.reload();
        off_peak_settings(&mut engine, Some(&id), &[]);
        tariff_report(&mut engine, &id, "test/tariff", "PEAK");
        assert_eq!(
            (run_count(&engine, "on"), run_count(&engine, "off")),
            (0, 0)
        );
        tariff_report(&mut engine, &id, "test/tariff", "OFF");
        assert_eq!(
            (run_count(&engine, "on"), run_count(&engine, "off")),
            (1, 0)
        );
        let trigger = engine.hub.flow("on").unwrap().runs[0]
            .trigger
            .clone()
            .unwrap();
        assert_eq!(trigger.entity.as_deref(), Some(id.as_str()));
        assert_eq!(trigger.value, serde_json::json!("start"));
        tariff_report(&mut engine, &id, "test/tariff", "PEAK");
        assert_eq!(
            (run_count(&engine, "on"), run_count(&engine, "off")),
            (1, 1)
        );
    }

    #[tokio::test]
    async fn reconnect_replay_seeds_without_firing() {
        let (mut engine, id, _) = tariff_engine((2026, 1, 1, 12, 0));
        off_peak_flow(&engine, "on", "start");
        off_peak_flow(&engine, "off", "end");
        engine.reload();
        off_peak_settings(&mut engine, Some(&id), &[]);
        tariff_report(&mut engine, &id, "test/tariff", "PEAK");
        engine.handle_event(WsEvent::ConnectionChanged(ConnectionStatus::Offline {
            reason: "test".to_owned(),
        }));
        engine.handle_event(WsEvent::ConnectionChanged(ConnectionStatus::Online));
        tariff_report(&mut engine, &id, "test/tariff", "OFF");
        assert_eq!(run_count(&engine, "on"), 0);
        tariff_report(&mut engine, &id, "test/tariff", "PEAK");
        assert_eq!(run_count(&engine, "off"), 1);
    }

    #[tokio::test]
    async fn settings_change_never_fires() {
        // 12:00 UTC is 13:00 in Paris.
        let (mut engine, _, _) = tariff_engine((2026, 1, 1, 12, 0));
        off_peak_flow(&engine, "on", "start");
        engine.reload();
        off_peak_settings(&mut engine, None, &[("22:00", "06:00")]);
        off_peak_settings(&mut engine, None, &[("12:00", "14:00")]);
        assert_eq!(run_count(&engine, "on"), 0);
    }

    #[tokio::test]
    async fn flow_save_never_fires() {
        let (mut engine, _, _) = tariff_engine((2026, 1, 1, 12, 0));
        off_peak_settings(&mut engine, None, &[("12:00", "14:00")]);
        off_peak_flow(&engine, "on", "start");
        engine.reload();
        off_peak_flow(&engine, "off", "end");
        engine.reload();
        assert_eq!(
            (run_count(&engine, "on"), run_count(&engine, "off")),
            (0, 0)
        );
    }

    #[tokio::test]
    async fn window_boundary_fires_without_device() {
        // 20:59 UTC is 21:59 in Paris.
        let (mut engine, _, clock) = tariff_engine((2026, 1, 1, 20, 59));
        off_peak_flow(&engine, "on", "start");
        engine.reload();
        off_peak_settings(&mut engine, None, &[("22:00", "06:00")]);
        engine.tick_off_peak();
        assert_eq!(run_count(&engine, "on"), 0);
        *clock.lock().unwrap() = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 21, 0, 0).unwrap();
        engine.tick_off_peak();
        assert_eq!(run_count(&engine, "on"), 1);
        let trigger = engine.hub.flow("on").unwrap().runs[0]
            .trigger
            .clone()
            .unwrap();
        assert_eq!(trigger.entity, None);
        assert_eq!(trigger.value, serde_json::json!("start"));
    }

    #[tokio::test]
    async fn fallback_without_windows_ends_once() {
        let (mut engine, id, _) = tariff_engine((2026, 1, 1, 12, 0));
        off_peak_flow(&engine, "off", "end");
        engine.reload();
        off_peak_settings(&mut engine, Some(&id), &[]);
        tariff_report(&mut engine, &id, "test/tariff/avail", "online");
        tariff_report(&mut engine, &id, "test/tariff", "OFF");
        tariff_report(&mut engine, &id, "test/tariff/avail", "offline");
        assert_eq!(run_count(&engine, "off"), 1);
        tariff_report(&mut engine, &id, "test/tariff/avail", "offline");
        assert_eq!(run_count(&engine, "off"), 1);
    }

    #[tokio::test]
    async fn boot_flows_before_settings_then_first_report_never_fires() {
        // At boot the flows can replay before the home settings, and both before the device.
        let (mut engine, id, _) = tariff_engine((2026, 1, 1, 1, 0));
        off_peak_flow(&engine, "on", "start");
        engine.reload();
        off_peak_settings(&mut engine, Some(&id), &[]);
        tariff_report(&mut engine, &id, "test/tariff", "OFF");
        assert_eq!(run_count(&engine, "on"), 0);
        tariff_report(&mut engine, &id, "test/tariff", "PEAK");
        tariff_report(&mut engine, &id, "test/tariff", "OFF");
        assert_eq!(run_count(&engine, "on"), 1);
    }

    #[tokio::test]
    async fn a_lag_keeps_the_next_window_edge() {
        let (mut engine, _, clock) = tariff_engine((2026, 1, 1, 20, 59));
        off_peak_flow(&engine, "on", "start");
        engine.reload();
        off_peak_settings(&mut engine, None, &[("22:00", "06:00")]);
        engine.events_lagged();
        *clock.lock().unwrap() = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 21, 0, 0).unwrap();
        engine.tick_off_peak();
        assert_eq!(run_count(&engine, "on"), 1);
    }

    #[tokio::test]
    async fn a_window_edge_during_an_outage_is_recorded_skipped() {
        let (mut engine, _, clock) = tariff_engine((2026, 1, 1, 20, 59));
        off_peak_flow(&engine, "on", "start");
        engine.reload();
        off_peak_settings(&mut engine, None, &[("22:00", "06:00")]);
        let offline = ConnectionStatus::Offline {
            reason: "test".to_owned(),
        };
        engine.hub.set_connection(offline.clone());
        engine.handle_event(WsEvent::ConnectionChanged(offline));
        *clock.lock().unwrap() = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 21, 0, 0).unwrap();
        engine.tick_off_peak();
        let runs = engine.hub.flow("on").unwrap().runs;
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, RunStatus::Skipped);
    }

    #[tokio::test]
    async fn is_off_peak_reads_the_resolver() {
        let (mut engine, id, _) = tariff_engine((2026, 1, 1, 12, 0));
        off_peak_settings(&mut engine, Some(&id), &[]);
        tariff_report(&mut engine, &id, "test/tariff", "OFF");
        let check = node(serde_json::json!({
            "id": "c", "type": "if", "params": { "op": "is_off_peak" }
        }));
        let (outcome, detail, ports) = run_node(
            &engine.hub,
            &engine.now,
            "f",
            "r",
            &check,
            None,
            &mut BTreeMap::new(),
        )
        .await;
        assert_eq!(outcome, StepOutcome::Ok);
        assert_eq!(ports, ["true"]);
        assert_eq!(detail, "is_off_peak → true");
    }

    /// Runs one `if` node with `average` on an engine hub that has no history.
    async fn run_if_average(extra: serde_json::Value) -> (StepOutcome, String, Vec<String>) {
        let engine = engine_at(48.8566, 2.3522, "Europe/Paris");
        let mut params = serde_json::json!({
            "op": "lte", "value": 19, "average": "30m", "entity": "sensor_temperature"
        });
        params
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let node: Node = serde_json::from_value(serde_json::json!({
            "id": "check", "type": "if", "params": params
        }))
        .unwrap();
        run_node(
            &engine.hub,
            &engine.now,
            "f",
            "r",
            &node,
            None,
            &mut BTreeMap::new(),
        )
        .await
    }

    #[tokio::test]
    async fn if_average_without_history_takes_the_missing_port() {
        let (outcome, detail, port) =
            run_if_average(serde_json::json!({ "missing": "false" })).await;
        assert_eq!(outcome, StepOutcome::Ok);
        assert_eq!(detail, "missing → false");
        assert_eq!(port, ["false"]);
        let (_, detail, port) = run_if_average(serde_json::json!({ "missing": "true" })).await;
        assert_eq!(detail, "missing → true");
        assert_eq!(port, ["true"]);
    }

    #[tokio::test]
    async fn if_average_without_history_and_default_missing_compares_nothing() {
        let (outcome, _, port) = run_if_average(serde_json::json!({})).await;
        assert_eq!(outcome, StepOutcome::Ok);
        assert_eq!(port, ["false"]);
    }

    #[tokio::test]
    async fn if_average_counts_every_reading_including_the_newest() {
        let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
        engine.hub.ingest(
            "nidoo/sensor/room/temperature/config",
            br#"{"unique_id":"room_temperature","state_topic":"test/room","value_template":"{{ value_json.temperature }}","unit_of_measurement":"\u00b0C"}"#,
        );
        let entity = engine.hub.snapshot().entities[0].id.clone();
        let flow = serde_json::json!({
            "id": "avg", "name": "avg", "enabled": true, "mode": "single",
            "nodes": [
                { "id": "t", "type": "trigger.state", "params": { "entity": entity } },
                { "id": "check", "type": "if", "params": { "op": "lte", "value": 19, "average": "30m" } }
            ],
            "edges": [{ "from": "t", "to": "check" }]
        });
        engine
            .hub
            .ingest("nidoo/automation/avg", flow.to_string().as_bytes());
        engine.reload();
        // A repeated reading counts again, and the newest one counts.
        for t in [20, 20, 17] {
            engine
                .hub
                .ingest("test/room", format!(r#"{{"temperature":{t}}}"#).as_bytes());
        }
        let node = engine.flows[0].nodes[1].clone();
        let trigger = TriggerInfo {
            entity: Some(entity.clone()),
            field: None,
            value: serde_json::json!(17),
            previous: serde_json::json!(20),
        };
        let (outcome, detail, port) = run_node(
            &engine.hub,
            &engine.now,
            "avg",
            "r",
            &node,
            Some(&trigger),
            &mut BTreeMap::new(),
        )
        .await;
        assert_eq!(outcome, StepOutcome::Ok);
        assert_eq!(detail, "average 30m = 19.0 → true");
        assert_eq!(port, ["true"]);
    }

    #[tokio::test]
    async fn switch_average_counts_every_reading_including_the_newest() {
        let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
        let entity = add_sensor(&engine, None);
        let flow = serde_json::json!({
            "id": "swavg", "name": "swavg", "enabled": true, "mode": "single",
            "nodes": [
                { "id": "t", "type": "trigger.state", "params": { "entity": entity } },
                { "id": "s", "type": "switch", "params": { "average": "30m",
                    "rules": [{ "op": "lte", "value": 18 }, { "op": "lte", "value": 19 }] } }
            ],
            "edges": [{ "from": "t", "to": "s" }]
        });
        engine
            .hub
            .ingest("nidoo/automation/swavg", flow.to_string().as_bytes());
        engine.reload();
        // Only a switch registers the entity: its readings must be buffered.
        for t in [20, 20, 17] {
            engine
                .hub
                .ingest("test/room", format!(r#"{{"temperature":{t}}}"#).as_bytes());
        }
        let node = engine.flows[0].nodes[1].clone();
        let trigger = TriggerInfo {
            entity: Some(entity.clone()),
            field: None,
            value: serde_json::json!(17),
            previous: serde_json::json!(20),
        };
        let (outcome, detail, ports) = run_node(
            &engine.hub,
            &engine.now,
            "swavg",
            "r",
            &node,
            Some(&trigger),
            &mut BTreeMap::new(),
        )
        .await;
        assert_eq!(outcome, StepOutcome::Ok);
        // The mean is 19, not the current 17 (which would take rule 1).
        assert_eq!(detail, "average 30m = 19.0 → 2");
        assert_eq!(ports, ["2"]);
    }

    #[test]
    fn math_input_average_registers_the_window() {
        let flow = Flow::decode(
            serde_json::json!({
                "id": "m", "name": "m", "enabled": true, "mode": "single",
                "nodes": [
                    { "id": "t", "type": "trigger.schedule", "params": { "cron": "* * * * *" } },
                    { "id": "calc", "type": "math", "params": {
                        "inputs": [
                            { "name": "a", "entity": "sensor_a", "average": "15m" },
                            { "name": "b", "entity": "sensor_b" }
                        ],
                        "formula": "a + b", "name": "sum" } }
                ],
                "edges": [{ "from": "t", "to": "calc" }]
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        let windows = averaged_windows(&[flow]);
        assert_eq!(windows.len(), 1, "{windows:?}");
        assert_eq!(windows["sensor_a"], 15 * 60 * 1_000_000);
    }

    /// Reports a numeric sensor and returns its entity id.
    fn add_sensor(engine: &Engine, value: Option<f64>) -> String {
        engine.hub.ingest(
            "nidoo/sensor/room/temperature/config",
            br#"{"unique_id":"room_temperature","state_topic":"test/room","value_template":"{{ value_json.temperature }}","unit_of_measurement":"\u00b0C"}"#,
        );
        if let Some(value) = value {
            engine.hub.ingest(
                "test/room",
                format!(r#"{{"temperature":{value}}}"#).as_bytes(),
            );
        }
        engine.hub.snapshot().entities[0].id.clone()
    }

    fn node(value: serde_json::Value) -> Node {
        serde_json::from_value(value).unwrap()
    }

    #[tokio::test]
    async fn switch_takes_first_matching_exit() {
        let engine = engine_at(48.8566, 2.3522, "Europe/Paris");
        let entity = add_sensor(&engine, Some(20.0));
        let node = node(serde_json::json!({
            "id": "s", "type": "switch", "params": { "entity": entity, "rules": [
                { "op": "lt", "value": 18 },
                { "op": "between", "value": 18, "to": 22 },
                { "op": "gt", "value": 22 } ] }
        }));
        let (outcome, detail, ports) = run_node(
            &engine.hub,
            &engine.now,
            "f",
            "r",
            &node,
            None,
            &mut BTreeMap::new(),
        )
        .await;
        assert_eq!(outcome, StepOutcome::Ok);
        assert_eq!(ports, ["2"]);
        assert!(detail.ends_with("→ 2"), "{detail}");
    }

    #[tokio::test]
    async fn switch_unknown_value_takes_otherwise() {
        let engine = engine_at(48.8566, 2.3522, "Europe/Paris");
        let node = node(serde_json::json!({
            "id": "s", "type": "switch", "params": {
                "entity": "nobody", "otherwise": true,
                "rules": [{ "op": "gt", "value": 1 }] }
        }));
        let (outcome, detail, ports) = run_node(
            &engine.hub,
            &engine.now,
            "f",
            "r",
            &node,
            None,
            &mut BTreeMap::new(),
        )
        .await;
        assert_eq!(outcome, StepOutcome::Ok);
        assert_eq!(ports, ["otherwise"]);
        assert_eq!(detail, "unknown → otherwise");
    }

    #[tokio::test]
    async fn switch_nothing_fired_reads_none() {
        let engine = engine_at(48.8566, 2.3522, "Europe/Paris");
        let entity = add_sensor(&engine, Some(20.0));
        let node = node(serde_json::json!({
            "id": "s", "type": "switch", "params": {
                "entity": entity, "rules": [{ "op": "gt", "value": 99 }] }
        }));
        let (_, detail, ports) = run_node(
            &engine.hub,
            &engine.now,
            "f",
            "r",
            &node,
            None,
            &mut BTreeMap::new(),
        )
        .await;
        assert!(ports.is_empty());
        assert_eq!(detail, "20 → none");
    }

    /// The ids of the steps a run recorded, in order.
    fn recorded(hub: &Hub, flow: &str) -> Vec<String> {
        hub.flow(flow).unwrap().runs[0]
            .steps
            .iter()
            .map(|s| s.node.clone())
            .collect()
    }

    #[tokio::test]
    async fn switch_all_runs_exits_in_rule_order() {
        let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
        let entity = add_sensor(&engine, Some(20.0));
        let flow = serde_json::json!({
            "id": "sw", "name": "sw", "enabled": true, "mode": "single",
            "nodes": [
                { "id": "t", "type": "trigger.state", "params": { "entity": entity } },
                { "id": "s", "type": "switch", "params": { "entity": entity, "all": true,
                    "rules": [{ "op": "gte", "value": 10 }, { "op": "lte", "value": 30 }] } },
                { "id": "a", "type": "notify", "params": { "title": "x" } },
                { "id": "a2", "type": "notify", "params": { "title": "x" } },
                { "id": "b", "type": "notify", "params": { "title": "x" } }
            ],
            "edges": [
                { "from": "t", "to": "s" },
                { "from": "s", "to": "b", "port": "2" },
                { "from": "s", "to": "a", "port": "1" },
                { "from": "a", "to": "a2" }
            ]
        });
        engine
            .hub
            .ingest("nidoo/automation/sw", flow.to_string().as_bytes());
        engine.reload();
        let flow = engine.flows[0].clone();
        let run = engine.hub.begin_run("sw", None).unwrap();
        execute(
            engine.hub.clone(),
            engine.now.clone(),
            flow,
            run.id,
            "t".to_owned(),
            None,
        )
        .await;
        // Exit 1 runs before exit 2 whatever the edge order, depth first.
        assert_eq!(recorded(&engine.hub, "sw"), ["t", "s", "a", "a2", "b"]);
    }

    /// Runs a `time.between` node with the clock at `utc`.
    async fn run_time(utc: (i32, u32, u32, u32, u32), from: &str, to: &str) -> Vec<String> {
        let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
        let at = chrono::Utc
            .with_ymd_and_hms(utc.0, utc.1, utc.2, utc.3, utc.4, 0)
            .unwrap();
        engine.now = Arc::new(move || at);
        let node = node(serde_json::json!({
            "id": "tm", "type": "time.between", "params": { "from": from, "to": to }
        }));
        run_node(
            &engine.hub,
            &engine.now,
            "f",
            "r",
            &node,
            None,
            &mut BTreeMap::new(),
        )
        .await
        .2
    }

    #[tokio::test]
    async fn time_between_uses_house_zone() {
        // 21:40 UTC is 22:40 in Paris (CET is UTC+1 only in winter; this is CET after 25 Oct).
        let now = (2026, 10, 27, 21, 40);
        assert_eq!(run_time(now, "07:00", "22:30").await, ["out"]);
        assert_eq!(run_time(now, "22:00", "06:00").await, ["in"]);
    }

    #[tokio::test]
    async fn time_between_wall_clock_on_dst_day() {
        // 06:30 UTC on 2026-10-25 is 07:30 in Paris, after the clocks went back.
        assert_eq!(
            run_time((2026, 10, 25, 6, 30), "07:00", "08:00").await,
            ["in"]
        );
    }

    #[tokio::test]
    async fn step_view_records_ports_for_branching_nodes_only() {
        let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
        let entity = add_sensor(&engine, Some(20.0));
        let flow = serde_json::json!({
            "id": "pv", "name": "pv", "enabled": true, "mode": "single",
            "nodes": [
                { "id": "t", "type": "trigger.state", "params": { "entity": entity } },
                { "id": "tm", "type": "time.between", "params": { "from": "00:00", "to": "23:59" } },
                { "id": "c", "type": "notify", "params": { "title": "x" } }
            ],
            "edges": [{ "from": "t", "to": "tm" }, { "from": "tm", "to": "c", "port": "in" }]
        });
        engine
            .hub
            .ingest("nidoo/automation/pv", flow.to_string().as_bytes());
        engine.reload();
        let flow = engine.flows[0].clone();
        let run = engine.hub.begin_run("pv", None).unwrap();
        execute(
            engine.hub.clone(),
            engine.now.clone(),
            flow,
            run.id,
            "t".to_owned(),
            None,
        )
        .await;
        let steps = engine.hub.flow("pv").unwrap().runs[0].steps.clone();
        assert_eq!(steps[0].ports, None);
        assert_eq!(steps[1].ports, Some(vec!["in".to_owned()]));
        assert_eq!(steps[2].node, "c");
        assert_eq!(steps[2].ports, None);
    }

    #[tokio::test]
    async fn step_view_records_no_ports_for_a_failed_branching_step() {
        let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
        let entity = add_sensor(&engine, Some(20.0));
        let flow = serde_json::json!({
            "id": "pf", "name": "pf", "enabled": true, "mode": "single",
            "nodes": [
                { "id": "t", "type": "trigger.state", "params": { "entity": entity } },
                { "id": "tm", "type": "switch", "params": { "rules": [{ "op": "gt", "value": 1 }] } }
            ],
            "edges": [{ "from": "t", "to": "tm" }]
        });
        engine
            .hub
            .ingest("nidoo/automation/pf", flow.to_string().as_bytes());
        engine.reload();
        let flow = engine.flows[0].clone();
        let run = engine.hub.begin_run("pf", None).unwrap();
        execute(
            engine.hub.clone(),
            engine.now.clone(),
            flow,
            run.id,
            "t".to_owned(),
            None,
        )
        .await;
        let steps = engine.hub.flow("pf").unwrap().runs[0].steps.clone();
        assert_eq!(steps[1].node, "tm");
        assert_eq!(steps[1].outcome, StepOutcome::Failed);
        assert_eq!(steps[1].ports, None);
    }

    /// An online Paris engine with a vacuum `robo` reporting `{state, battery_level}` on
    /// `test/robo`; returns the engine and the vacuum's id.
    fn vacuum_engine() -> (Engine, String) {
        let engine = engine_at(48.8566, 2.3522, "Europe/Paris");
        engine.hub.ingest(
            "nidoo/vacuum/robo/config",
            br#"{"unique_id":"robo","state_topic":"test/robo","fan_speed_list":["Silent","Turbo"]}"#,
        );
        engine.hub.set_connection(ConnectionStatus::Online);
        let id = engine.hub.snapshot().entities[0].id.clone();
        (engine, id)
    }

    /// Publishes the vacuum's state and hands the engine the change event.
    fn vacuum_report(engine: &mut Engine, id: &str, state: &str, battery: u8) {
        let payload = format!(r#"{{"state":"{state}","battery_level":{battery}}}"#);
        engine.hub.ingest("test/robo", payload.as_bytes());
        let state = engine.hub.read().state(id).cloned().expect("vacuum state");
        engine.handle_event(WsEvent::StateChanged {
            id: id.to_owned(),
            state,
        });
    }

    /// A flow `id` triggered by `params` of a state trigger, with nothing after it.
    fn state_flow(engine: &Engine, id: &str, params: serde_json::Value) {
        let flow = serde_json::json!({
            "id": id, "name": id, "enabled": true, "mode": "single",
            "nodes": [{ "id": "t", "type": "trigger.state", "params": params }],
            "edges": []
        });
        engine.hub.ingest(
            &format!("nidoo/automation/{id}"),
            flow.to_string().as_bytes(),
        );
    }

    #[tokio::test]
    async fn field_trigger_fires_only_on_its_field() {
        let (mut engine, id) = vacuum_engine();
        state_flow(&engine, "plain", serde_json::json!({ "entity": id }));
        state_flow(
            &engine,
            "battery",
            serde_json::json!({ "entity": id, "field": "battery" }),
        );
        engine.reload();
        vacuum_report(&mut engine, &id, "cleaning", 62);
        vacuum_report(&mut engine, &id, "cleaning", 55);
        assert_eq!(
            (run_count(&engine, "plain"), run_count(&engine, "battery")),
            (0, 1)
        );
        let trigger = engine.hub.flow("battery").unwrap().runs[0]
            .trigger
            .clone()
            .unwrap();
        assert_eq!(trigger.entity.as_deref(), Some(id.as_str()));
        assert_eq!(trigger.field.as_deref(), Some("battery"));
        assert_eq!(trigger.value, serde_json::json!(55.0));
        assert_eq!(trigger.previous, serde_json::json!(62.0));
        vacuum_report(&mut engine, &id, "docked", 55);
        assert_eq!(
            (run_count(&engine, "plain"), run_count(&engine, "battery")),
            (1, 1)
        );
    }

    #[tokio::test]
    async fn field_trigger_seeds_silently_after_reconnect() {
        let (mut engine, id) = vacuum_engine();
        state_flow(
            &engine,
            "battery",
            serde_json::json!({ "entity": id, "field": "battery" }),
        );
        engine.reload();
        vacuum_report(&mut engine, &id, "cleaning", 62);
        engine.handle_event(WsEvent::ConnectionChanged(ConnectionStatus::Offline {
            reason: "test".to_owned(),
        }));
        engine.handle_event(WsEvent::ConnectionChanged(ConnectionStatus::Online));
        vacuum_report(&mut engine, &id, "cleaning", 55);
        assert_eq!(run_count(&engine, "battery"), 0);
        vacuum_report(&mut engine, &id, "cleaning", 50);
        assert_eq!(run_count(&engine, "battery"), 1);
    }

    #[tokio::test]
    async fn if_without_entity_reads_the_trigger_field() {
        let (mut engine, id) = vacuum_engine();
        vacuum_report(&mut engine, &id, "cleaning", 55);
        let node = node(serde_json::json!({
            "id": "check", "type": "if", "params": { "op": "lte", "value": 55 }
        }));
        let trigger = TriggerInfo {
            entity: Some(id.clone()),
            field: Some("battery".to_owned()),
            value: serde_json::json!(55),
            previous: serde_json::json!(62),
        };
        let (outcome, detail, port) = run_node(
            &engine.hub,
            &engine.now,
            "f",
            "r",
            &node,
            Some(&trigger),
            &mut BTreeMap::new(),
        )
        .await;
        assert_eq!(outcome, StepOutcome::Ok, "{detail}");
        assert_eq!(port, ["true"]);
    }

    #[tokio::test]
    async fn if_reads_entity_field() {
        let (mut engine, id) = vacuum_engine();
        vacuum_report(&mut engine, &id, "cleaning", 50);
        let node = node(serde_json::json!({
            "id": "check", "type": "if",
            "params": { "entity": id, "field": "battery", "op": "lte", "value": 55 }
        }));
        let (outcome, detail, port) = run_node(
            &engine.hub,
            &engine.now,
            "f",
            "r",
            &node,
            None,
            &mut BTreeMap::new(),
        )
        .await;
        assert_eq!(outcome, StepOutcome::Ok, "{detail}");
        assert_eq!(port, ["true"]);
    }

    #[tokio::test]
    async fn switch_reads_entity_field() {
        let (mut engine, id) = vacuum_engine();
        vacuum_report(&mut engine, &id, "cleaning", 50);
        let node = node(serde_json::json!({
            "id": "s", "type": "switch", "params": { "entity": id, "field": "battery",
                "rules": [{ "op": "lte", "value": 20 }, { "op": "lte", "value": 60 }] }
        }));
        let (outcome, detail, ports) = run_node(
            &engine.hub,
            &engine.now,
            "f",
            "r",
            &node,
            None,
            &mut BTreeMap::new(),
        )
        .await;
        assert_eq!(outcome, StepOutcome::Ok, "{detail}");
        assert_eq!(ports, ["2"]);
    }

    #[tokio::test]
    async fn if_average_reads_a_recorded_field() {
        let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
        engine.hub.ingest(
            "nidoo/climate/clim/config",
            br#"{"unique_id":"clim","mode_state_topic":"test/clim","mode_state_template":"{{ value_json.mode }}","current_temperature_topic":"test/clim","current_temperature_template":"{{ value_json.current }}","modes":["off","heat"]}"#,
        );
        let entity = engine.hub.snapshot().entities[0].id.clone();
        let flow = serde_json::json!({
            "id": "avg", "name": "avg", "enabled": true, "mode": "single",
            "nodes": [
                { "id": "t", "type": "trigger.state", "params": { "entity": entity } },
                { "id": "check", "type": "if", "params": { "entity": entity, "field": "current",
                    "op": "lte", "value": 19, "average": "30m" } }
            ],
            "edges": [{ "from": "t", "to": "check" }]
        });
        engine
            .hub
            .ingest("nidoo/automation/avg", flow.to_string().as_bytes());
        engine.reload();
        for t in [20, 20, 17] {
            engine.hub.ingest(
                "test/clim",
                format!(r#"{{"mode":"heat","current":{t}}}"#).as_bytes(),
            );
        }
        let node = engine.flows[0].nodes[1].clone();
        let (outcome, detail, port) = run_node(
            &engine.hub,
            &engine.now,
            "avg",
            "r",
            &node,
            None,
            &mut BTreeMap::new(),
        )
        .await;
        assert_eq!(outcome, StepOutcome::Ok, "{detail}");
        assert_eq!(detail, "average 30m = 19.0 → true");
        assert_eq!(port, ["true"]);
    }

    #[tokio::test]
    async fn generic_headline_trigger_fires() {
        let mut engine = engine_at(48.8566, 2.3522, "Europe/Paris");
        engine.hub.ingest(
            "nidoo/lock/front/config",
            br#"{"unique_id":"front","state_topic":"test/front"}"#,
        );
        engine.hub.set_connection(ConnectionStatus::Online);
        let id = engine.hub.snapshot().entities[0].id.clone();
        state_flow(
            &engine,
            "lock",
            serde_json::json!({ "entity": id, "to": "unlocked" }),
        );
        engine.reload();
        for payload in ["LOCKED", "UNLOCKED"] {
            engine.hub.ingest("test/front", payload.as_bytes());
            let state = engine.hub.read().state(&id).cloned().expect("lock state");
            engine.handle_event(WsEvent::StateChanged {
                id: id.clone(),
                state,
            });
        }
        assert_eq!(run_count(&engine, "lock"), 1);
        assert_eq!(
            engine.hub.entity_value(&id, None),
            Some(Scalar::Text("unlocked".into()))
        );
    }

    /// An online Paris engine with a binary sensor `det` whose alarm is the `alarm` key of the
    /// JSON on `test/det`: `{"alarm":"yes"}` is on, `{}` leaves the alarm unknown.
    fn alarm_engine() -> (Engine, String) {
        let engine = engine_at(48.8566, 2.3522, "Europe/Paris");
        engine.hub.ingest(
            "nidoo/binary_sensor/det/alarm/config",
            br#"{"unique_id":"alarm","state_topic":"test/det","value_template":"{{ value_json.alarm }}","payload_on":"yes","payload_off":"no"}"#,
        );
        engine.hub.set_connection(ConnectionStatus::Online);
        let id = engine.hub.snapshot().entities[0].id.clone();
        (engine, id)
    }

    /// Publishes `payload` on the detector's topic and hands the engine what the hub sends: the
    /// change event when the value moved, then every message signal the hub broadcast.
    fn alarm_report(engine: &mut Engine, id: &str, payload: &str) {
        let mut touched = engine.hub.subscribe_state_touched();
        let changes = engine.hub.ingest("test/det", payload.as_bytes());
        if changes
            .iter()
            .any(|c| matches!(c, nidoo_core::registry::Change::StateChanged(_)))
        {
            let state = engine.hub.read().state(id).cloned().expect("state");
            engine.handle_event(WsEvent::StateChanged {
                id: id.to_owned(),
                state,
            });
        }
        while let Ok(signal) = touched.try_recv() {
            engine.state_touched(&signal);
        }
    }

    /// Lets every spawned run end, as the engine loop would.
    async fn finish_runs(engine: &mut Engine) {
        while let Some(joined) = engine.tasks.join_next_with_id().await {
            engine.run_finished(joined);
        }
    }

    #[tokio::test]
    async fn unknown_then_value_fires_after_a_confirming_report() {
        let (mut engine, id) = alarm_engine();
        state_flow(
            &engine,
            "a",
            serde_json::json!({ "entity": id, "to": true }),
        );
        engine.reload();
        alarm_report(&mut engine, &id, "{}");
        alarm_report(&mut engine, &id, r#"{"alarm":"yes"}"#);
        assert_eq!(run_count(&engine, "a"), 1);
        let trigger = engine.hub.flow("a").unwrap().runs[0]
            .trigger
            .clone()
            .unwrap();
        assert_eq!(trigger.value, serde_json::json!(true));
        assert_eq!(trigger.previous, serde_json::Value::Null);
    }

    /// An online engine with the binary sensor a Zigbee2MQTT smoke detector announces: the
    /// `smoke` key of the JSON on `test/det`, `true` on and `false` off.
    fn smoke_engine() -> (Engine, String) {
        let engine = engine_at(48.8566, 2.3522, "Europe/Paris");
        engine.hub.ingest(
            "nidoo/binary_sensor/det/smoke/config",
            br#"{"unique_id":"smoke","state_topic":"test/det","value_template":"{{ value_json[\"smoke\"] }}","payload_on":true,"payload_off":false}"#,
        );
        engine.hub.set_connection(ConnectionStatus::Online);
        let id = engine.hub.snapshot().entities[0].id.clone();
        (engine, id)
    }

    #[tokio::test]
    async fn null_reports_then_true_fires_once() {
        let (mut engine, id) = smoke_engine();
        state_flow(
            &engine,
            "a",
            serde_json::json!({ "entity": id, "to": true }),
        );
        engine.reload();
        alarm_report(&mut engine, &id, r#"{"battery":13,"smoke":null}"#);
        alarm_report(&mut engine, &id, r#"{"battery":13,"smoke":null}"#);
        alarm_report(&mut engine, &id, r#"{"battery":13,"smoke":true}"#);
        assert_eq!(run_count(&engine, "a"), 1);
    }

    /// At boot the restore waits for the broker's replay: a message that read nothing lands
    /// first and confirms the unknown, so a recorded value put back after it would look like a
    /// first value. The registry refuses to restore over an entity that spoke.
    #[tokio::test]
    async fn restore_after_a_null_report_does_not_fire() {
        for (recorded, to) in [(true, true), (false, false)] {
            let (mut engine, id) = smoke_engine();
            state_flow(&engine, "a", serde_json::json!({ "entity": id, "to": to }));
            engine.reload();
            alarm_report(&mut engine, &id, r#"{"battery":13,"smoke":null}"#);
            let restored = engine.hub.restore_recorded_states(vec![(
                id.clone(),
                0,
                StateValue::OnOff(recorded),
            )]);
            assert_eq!(restored, 0, "recorded {recorded}");
            let state = engine.hub.read().state(&id).cloned().expect("state");
            assert_eq!(state.value, StateValue::Unknown);
            assert_eq!(run_count(&engine, "a"), 0, "recorded {recorded}");
            alarm_report(&mut engine, &id, r#"{"battery":13,"smoke":true}"#);
            assert_eq!(
                run_count(&engine, "a"),
                usize::from(to),
                "recorded {recorded}"
            );
        }
    }

    #[tokio::test]
    async fn missing_key_reports_then_true_fires_once() {
        let (mut engine, id) = smoke_engine();
        state_flow(
            &engine,
            "a",
            serde_json::json!({ "entity": id, "to": true }),
        );
        engine.reload();
        alarm_report(&mut engine, &id, r#"{"battery":13}"#);
        alarm_report(&mut engine, &id, r#"{"battery":13,"smoke":true}"#);
        assert_eq!(run_count(&engine, "a"), 1);
    }

    #[tokio::test]
    async fn a_first_true_without_a_prior_report_stays_silent() {
        let (mut engine, id) = smoke_engine();
        state_flow(
            &engine,
            "a",
            serde_json::json!({ "entity": id, "to": true }),
        );
        engine.reload();
        alarm_report(&mut engine, &id, r#"{"battery":13,"smoke":true}"#);
        assert_eq!(run_count(&engine, "a"), 0);
    }

    #[tokio::test]
    async fn first_report_with_the_value_is_silent() {
        let (mut engine, id) = alarm_engine();
        state_flow(
            &engine,
            "a",
            serde_json::json!({ "entity": id, "to": true }),
        );
        engine.reload();
        alarm_report(&mut engine, &id, r#"{"alarm":"yes"}"#);
        assert_eq!(run_count(&engine, "a"), 0);
    }

    #[tokio::test]
    async fn restore_of_the_to_value_does_not_fire() {
        let (mut engine, id) = alarm_engine();
        state_flow(
            &engine,
            "a",
            serde_json::json!({ "entity": id, "to": true }),
        );
        engine.reload();
        engine
            .hub
            .restore_recorded_states(vec![(id.clone(), 0, StateValue::OnOff(true))]);
        let state = engine.hub.read().state(&id).cloned().expect("state");
        engine.handle_event(WsEvent::StateChanged {
            id: id.clone(),
            state,
        });
        assert_eq!(run_count(&engine, "a"), 0);
    }

    #[tokio::test]
    async fn from_or_no_to_stays_silent() {
        for params in [
            serde_json::json!({ "to": true, "from": false }),
            serde_json::json!({}),
        ] {
            let (mut engine, id) = alarm_engine();
            let mut params = params;
            params["entity"] = serde_json::json!(id);
            state_flow(&engine, "a", params.clone());
            engine.reload();
            alarm_report(&mut engine, &id, "{}");
            alarm_report(&mut engine, &id, r#"{"alarm":"yes"}"#);
            assert_eq!(run_count(&engine, "a"), 0, "{params}");
        }
    }

    #[tokio::test]
    async fn vanish_and_return_fires_again() {
        let (mut engine, id) = alarm_engine();
        state_flow(
            &engine,
            "a",
            serde_json::json!({ "entity": id, "to": true }),
        );
        engine.reload();
        alarm_report(&mut engine, &id, "{}");
        alarm_report(&mut engine, &id, r#"{"alarm":"yes"}"#);
        assert_eq!(run_count(&engine, "a"), 1);
        finish_runs(&mut engine).await;
        alarm_report(&mut engine, &id, r#"{"alarm":null}"#);
        alarm_report(&mut engine, &id, r#"{"alarm":"yes"}"#);
        assert_eq!(run_count(&engine, "a"), 2);
    }

    #[tokio::test]
    async fn link_drop_clears_the_mark() {
        let (mut engine, id) = alarm_engine();
        state_flow(
            &engine,
            "a",
            serde_json::json!({ "entity": id, "to": true }),
        );
        engine.reload();
        alarm_report(&mut engine, &id, "{}");
        engine.handle_event(WsEvent::ConnectionChanged(ConnectionStatus::Offline {
            reason: "test".to_owned(),
        }));
        engine.handle_event(WsEvent::ConnectionChanged(ConnectionStatus::Online));
        alarm_report(&mut engine, &id, r#"{"alarm":"yes"}"#);
        assert_eq!(run_count(&engine, "a"), 0);
    }

    type TestClock = Arc<std::sync::Mutex<chrono::DateTime<chrono::Utc>>>;

    /// An online detector engine whose clock the test moves, starting at 2026-01-01 00:00 UTC.
    fn clocked_engine() -> (Engine, String, TestClock) {
        let (mut engine, id) = alarm_engine();
        let clock = Arc::new(std::sync::Mutex::new(
            chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        ));
        let read = Arc::clone(&clock);
        engine.now = Arc::new(move || *read.lock().unwrap());
        (engine, id, clock)
    }

    /// Moves the clock to `minutes` after 2026-01-01 00:00 UTC.
    fn at(clock: &TestClock, minutes: i64) {
        *clock.lock().unwrap() = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
            + chrono::Duration::minutes(minutes);
    }

    /// Stores flow `id` with a silent trigger on `entity` for `for_`, then a delay of `delay`.
    fn silent_flow(engine: &Engine, id: &str, name: &str, for_: &str, delay: &str, entity: &str) {
        let flow = serde_json::json!({
            "id": id, "name": name, "enabled": true, "mode": "single",
            "nodes": [
                { "id": "t", "type": "trigger.silent", "params": { "entity": entity, "for": for_ } },
                { "id": "d", "type": "delay", "params": { "for": delay } }
            ],
            "edges": [{ "from": "t", "to": "d" }]
        });
        engine.hub.ingest(
            &format!("nidoo/automation/{id}"),
            flow.to_string().as_bytes(),
        );
    }

    /// An entity report, as the engine's `StateTouched` branch gets it.
    fn report(engine: &mut Engine, id: &str) {
        let epoch = engine.hub.link_epoch();
        engine.state_touched(&StateTouched {
            id: id.to_owned(),
            epoch,
            read: true,
        });
    }

    #[tokio::test(start_paused = true)]
    async fn silent_fires_once_after_for() {
        let (mut engine, id, clock) = clocked_engine();
        silent_flow(&engine, "s", "s", "50m", "1s", &id);
        engine.reload();
        at(&clock, 49);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 0);
        at(&clock, 50);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 1);
        finish_runs(&mut engine).await;
        at(&clock, 180);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 1);
        let run = &engine.hub.flow("s").unwrap().runs[0];
        let trigger = run.trigger.clone().unwrap();
        assert_eq!(trigger.entity.as_deref(), Some(id.as_str()));
        assert_eq!(trigger.value, serde_json::json!("50m"));
        assert_eq!(run.steps[0].detail, format!("{id} silent for 50m"));
    }

    #[tokio::test(start_paused = true)]
    async fn messages_that_read_nothing_restart_the_clock() {
        let (mut engine, id, clock) = clocked_engine();
        silent_flow(&engine, "s", "s", "50m", "1s", &id);
        engine.reload();
        for minutes in [40, 80, 120] {
            at(&clock, minutes);
            alarm_report(&mut engine, &id, r#"{"battery":13}"#);
            engine.tick_silents();
            assert_eq!(run_count(&engine, "s"), 0, "at {minutes}m");
        }
        at(&clock, 170);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn report_restarts_the_clock() {
        let (mut engine, id, clock) = clocked_engine();
        silent_flow(&engine, "s", "s", "50m", "1s", &id);
        engine.reload();
        at(&clock, 40);
        report(&mut engine, &id);
        at(&clock, 50);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 0);
        at(&clock, 90);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn fires_again_after_a_report_and_new_silence() {
        let (mut engine, id, clock) = clocked_engine();
        silent_flow(&engine, "s", "s", "50m", "1s", &id);
        engine.reload();
        at(&clock, 50);
        engine.tick_silents();
        finish_runs(&mut engine).await;
        at(&clock, 120);
        report(&mut engine, &id);
        at(&clock, 170);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn silent_counts_from_arming_not_last_update() {
        let (mut engine, id, clock) = clocked_engine();
        alarm_report(&mut engine, &id, r#"{"alarm":"no"}"#);
        at(&clock, 600);
        silent_flow(&engine, "s", "s", "50m", "1s", &id);
        engine.reload();
        at(&clock, 601);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn silent_starts_at_enable() {
        let (mut engine, id, clock) = clocked_engine();
        silent_flow(&engine, "s", "s", "50m", "1s", &id);
        let mut flow = engine.hub.flow_definitions().remove(0);
        flow.enabled = false;
        engine
            .hub
            .ingest("nidoo/automation/s", flow.encode().as_bytes());
        engine.reload();
        at(&clock, 300);
        flow.enabled = true;
        engine
            .hub
            .ingest("nidoo/automation/s", flow.encode().as_bytes());
        engine.reload();
        at(&clock, 300 + 49);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 0);
        at(&clock, 300 + 50);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn silent_frozen_while_link_down_and_restarts_on_link_up() {
        let (mut engine, id, clock) = clocked_engine();
        silent_flow(&engine, "s", "s", "50m", "1s", &id);
        engine.reload();
        at(&clock, 10);
        let down = ConnectionStatus::Offline {
            reason: "test".to_owned(),
        };
        engine.hub.set_connection(down.clone());
        engine.handle_event(WsEvent::ConnectionChanged(down));
        assert!(engine.next_deadline().is_none());
        at(&clock, 120);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 0);
        engine.hub.set_connection(ConnectionStatus::Online);
        engine.handle_event(WsEvent::ConnectionChanged(ConnectionStatus::Online));
        assert!(engine.next_deadline().is_some());
        at(&clock, 120 + 49);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 0);
        at(&clock, 120 + 50);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn silent_clock_survives_unrelated_edit() {
        let (mut engine, id, clock) = clocked_engine();
        silent_flow(&engine, "s", "s", "50m", "1s", &id);
        engine.reload();
        at(&clock, 30);
        silent_flow(&engine, "s", "renamed", "50m", "1s", &id);
        engine.reload();
        at(&clock, 50);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 1);

        let (mut engine, id, clock) = clocked_engine();
        silent_flow(&engine, "s", "s", "50m", "1s", &id);
        engine.reload();
        at(&clock, 30);
        silent_flow(&engine, "s", "s", "1h", "1s", &id);
        engine.reload();
        at(&clock, 50);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 0);
        at(&clock, 90);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn seen_lag_restarts_every_clock() {
        let (mut engine, id, clock) = clocked_engine();
        silent_flow(&engine, "s", "s", "50m", "1s", &id);
        engine.reload();
        at(&clock, 30);
        engine.seen_lagged();
        at(&clock, 50);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 0);
        at(&clock, 80);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn skipped_fire_retries_after_the_run_ends() {
        let (mut engine, id, clock) = clocked_engine();
        // The first run sleeps for an hour of real time and stays active.
        silent_flow(&engine, "s", "s", "50m", "1h", &id);
        engine.reload();
        at(&clock, 50);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 1);
        at(&clock, 60);
        report(&mut engine, &id);
        at(&clock, 110);
        // Due, but the single-mode run is still active: no skipped run, no busy loop.
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 1);
        assert!(engine.next_deadline().is_none());
        finish_runs(&mut engine).await;
        assert!(engine.next_deadline().is_some());
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 2);
        assert_ne!(
            engine.hub.flow("s").unwrap().runs[0].status,
            RunStatus::Skipped
        );
        finish_runs(&mut engine).await;
        at(&clock, 400);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_link_that_came_back_unseen_restarts_the_clocks() {
        let (mut engine, id, clock) = clocked_engine();
        silent_flow(&engine, "s", "s", "50m", "1s", &id);
        engine.reload();
        at(&clock, 10);
        // The hub's link bounced but the engine has not handled the Online event yet.
        engine.hub.set_connection(ConnectionStatus::Offline {
            reason: "test".to_owned(),
        });
        engine.hub.set_connection(ConnectionStatus::Online);
        at(&clock, 60);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 0);
        at(&clock, 109);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 0);
        at(&clock, 110);
        engine.tick_silents();
        assert_eq!(run_count(&engine, "s"), 1);
    }

    /// Stores flow `id`: a manual trigger, a limit of `max` per `per`, then a delay of 1s.
    fn limit_flow(engine: &Engine, id: &str, name: &str, max: u32, per: &str) {
        let flow = serde_json::json!({
            "id": id, "name": name, "enabled": true, "mode": "single",
            "nodes": [
                { "id": "m", "type": "trigger.manual", "params": {} },
                { "id": "l", "type": "limit", "params": { "max": max, "per": per } },
                { "id": "d", "type": "delay", "params": { "for": "1s" } }
            ],
            "edges": [{ "from": "m", "to": "l" }, { "from": "l", "to": "d" }]
        });
        engine.hub.ingest(
            &format!("nidoo/automation/{id}"),
            flow.to_string().as_bytes(),
        );
    }

    /// Runs flow `id` by hand to its end; returns the step nodes and details and the status.
    async fn run_now(engine: &mut Engine, id: &str) -> (Vec<(String, String)>, RunStatus) {
        let run = engine.hub.run_flow(id).unwrap();
        engine.run_manual(id, run.id, None);
        finish_runs(engine).await;
        let run = engine.hub.flow(id).unwrap().runs[0].clone();
        let steps = run
            .steps
            .iter()
            .map(|s| (s.node.clone(), s.detail.clone()))
            .collect();
        (steps, run.status)
    }

    #[tokio::test(start_paused = true)]
    async fn limit_passes_max_times_then_stops() {
        let (mut engine, _, _) = clocked_engine();
        limit_flow(&engine, "l", "l", 2, "1h");
        engine.reload();
        for passes in 1..=2 {
            let (steps, status) = run_now(&mut engine, "l").await;
            assert_eq!(status, RunStatus::Done);
            assert_eq!(
                steps[1],
                (
                    "l".to_owned(),
                    format!("passed ({passes} of 2 in the last 1h)")
                )
            );
            assert_eq!(steps[2].0, "d");
        }
        let (steps, status) = run_now(&mut engine, "l").await;
        assert_eq!(status, RunStatus::Done);
        assert_eq!(steps.len(), 2);
        assert_eq!(
            steps[1],
            ("l".to_owned(), "limited: 2 in the last 1h".to_owned())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn limit_window_rolls() {
        let (mut engine, _, clock) = clocked_engine();
        limit_flow(&engine, "l", "l", 2, "1h");
        engine.reload();
        run_now(&mut engine, "l").await;
        at(&clock, 30);
        run_now(&mut engine, "l").await;
        at(&clock, 45);
        let (steps, _) = run_now(&mut engine, "l").await;
        assert!(steps[1].1.starts_with("limited:"));
        at(&clock, 61);
        let (steps, _) = run_now(&mut engine, "l").await;
        assert!(steps[1].1.starts_with("passed"), "{steps:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn limit_count_survives_flow_save() {
        let (mut engine, _, _) = clocked_engine();
        limit_flow(&engine, "l", "l", 2, "1h");
        engine.reload();
        run_now(&mut engine, "l").await;
        run_now(&mut engine, "l").await;
        limit_flow(&engine, "l", "renamed", 2, "1h");
        engine.reload();
        let (steps, _) = run_now(&mut engine, "l").await;
        assert!(steps[1].1.starts_with("limited:"), "{steps:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn limit_count_dropped_when_node_removed_or_changed() {
        let (mut engine, _, _) = clocked_engine();
        limit_flow(&engine, "l", "l", 2, "1h");
        engine.reload();
        run_now(&mut engine, "l").await;
        run_now(&mut engine, "l").await;
        limit_flow(&engine, "l", "l", 3, "1h");
        engine.reload();
        let (steps, _) = run_now(&mut engine, "l").await;
        assert_eq!(steps[1].1, "passed (1 of 3 in the last 1h)");
    }

    fn backup_flow(partial_wired: bool) -> Flow {
        let mut edges = vec![
            json!({ "from": "bk", "to": "yes", "port": "ok" }),
            json!({ "from": "bk", "to": "no", "port": "failed" }),
        ];
        if partial_wired {
            edges.push(json!({ "from": "bk", "to": "part", "port": "partial" }));
        }
        let tail =
            |id: &str| json!({ "id": id, "type": "flow.enable", "params": { "enabled": true } });
        serde_json::from_value(json!({
            "id": "f", "name": "f", "enabled": true, "mode": "single",
            "nodes": [
                { "id": "bk", "type": "backup.run", "params": {} },
                tail("yes"), tail("no"), tail("part"),
            ],
            "edges": edges,
        }))
        .unwrap()
    }

    fn strings(ports: &[&str]) -> Vec<String> {
        ports.iter().map(|p| (*p).to_owned()).collect()
    }

    #[test]
    fn backup_partial_follows_partial_when_wired() {
        let flow = backup_flow(true);
        assert_eq!(
            backup_ports(&flow, "bk", strings(&["partial"])),
            ["partial"]
        );
    }

    #[test]
    fn backup_partial_falls_back_to_ok_when_unwired() {
        let flow = backup_flow(false);
        assert_eq!(backup_ports(&flow, "bk", strings(&["partial"])), ["ok"]);
    }

    #[test]
    fn backup_ok_and_failed_pass_through() {
        for wired in [true, false] {
            let flow = backup_flow(wired);
            assert_eq!(backup_ports(&flow, "bk", strings(&["ok"])), ["ok"]);
            assert_eq!(backup_ports(&flow, "bk", strings(&["failed"])), ["failed"]);
        }
    }

    #[test]
    fn backup_detail_names_the_missing_zigbee_data() {
        assert_eq!(
            backup_detail("1.2 MB", "nas", "b.nidoo", None),
            "Backed up 1.2 MB to nas as b.nidoo"
        );
        assert_eq!(
            backup_detail("1.2 MB", "nas", "b.nidoo", Some("Zigbee2MQTT is offline")),
            "Backed up 1.2 MB to nas as b.nidoo, without Zigbee2MQTT data: Zigbee2MQTT is offline"
        );
    }
}
