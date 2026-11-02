//! Planning a request against the live house and executing the plan.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use nidoo_api::{already_there, area_views, entity_views};
use nidoo_assist::plan::{Action, Plan};
use nidoo_assist::reply::{self, ActionResult, temp};
use nidoo_assist::view::{
    ActionTrace, AssistRequest, AssistTurn, AssistTurns, OutcomeKind, PendingReply, Question,
    QuestionKind,
};
use nidoo_assist::{HeatRange, House, Pending, PendingAnswer, answer_text, plan, plan_request};
use nidoo_climate::doc::{Override, OverrideOrigin, Setpoint};
use nidoo_core::model::{Availability, Command};
use tokio::time::timeout;

use super::log::TurnLog;
use crate::clock;
use crate::hub::{Hub, HubError};

/// How long a database read or write of the turn log may take.
const DB_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a question waits for its answer, in seconds.
pub const PENDING_TTL_S: u64 = 60;

/// The assistant's memory: pending questions by client, and the turn log.
#[derive(Debug, Default)]
pub struct AssistState {
    pending: Mutex<HashMap<String, Slot>>,
    pub(crate) log: Mutex<TurnLog>,
}

#[derive(Debug)]
struct Slot {
    id: String,
    pending: Pending,
    expires_at: u64,
}

impl AssistState {
    fn slots(&self) -> std::sync::MutexGuard<'_, HashMap<String, Slot>> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn turns(&self) -> std::sync::MutexGuard<'_, TurnLog> {
        self.log
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Leaves `pending` as the client's one question; returns its id.
    fn put(&self, client: &str, pending: Pending, now: u64) -> String {
        let id = new_id();
        self.slots().insert(
            client.to_owned(),
            Slot {
                id: id.clone(),
                pending,
                expires_at: now + PENDING_TTL_S,
            },
        );
        id
    }

    /// Takes the client's question `id` when it is still open.
    fn take(&self, client: &str, id: &str, now: u64) -> Option<Pending> {
        let mut slots = self.slots();
        let open = slots
            .get(client)
            .is_some_and(|s| s.id == id && now < s.expires_at);
        let slot = slots.remove(client)?;
        open.then_some(slot.pending)
    }

    /// The client's open question, if any; an expired one is dropped.
    fn current(&self, client: &str, now: u64) -> Option<Pending> {
        let mut slots = self.slots();
        match slots.get(client) {
            Some(slot) if now < slot.expires_at => Some(slot.pending.clone()),
            Some(_) => {
                slots.remove(client);
                None
            }
            None => None,
        }
    }

    fn forget(&self, client: &str) {
        self.slots().remove(client);
    }
}

/// A random id, 16 hex digits.
fn new_id() -> String {
    let mut bytes = [0u8; 8];
    let _ = getrandom::getrandom(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// "Salon, Bureau ou Chambre Léa".
fn list_fr(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} ou {last}", rest.join(", ")),
    }
}

/// What the user said by tapping an answer.
fn said(pending: &Pending, answer: PendingAnswer) -> String {
    match (pending, answer) {
        (Pending::Choose(choices), PendingAnswer::Choice(i)) => choices
            .get(i)
            .map_or_else(|| format!("choix {}", i + 1), |(label, _)| label.clone()),
        (_, PendingAnswer::Yes) => "Oui".into(),
        (_, PendingAnswer::No) => "Non".into(),
        (_, PendingAnswer::Choice(i)) => format!("choix {}", i + 1),
    }
}

/// The plan an answer to a pending question leads to.
fn answered(pending: &Pending, answer: PendingAnswer, house: &House) -> Plan {
    if answer == PendingAnswer::No {
        return Plan::Answer {
            reply: "D'accord, je ne fais rien.".into(),
            trace: Default::default(),
        };
    }
    match pending.resolve(answer) {
        Some(request) => plan_request(&request, house),
        None => Plan::Answer {
            reply: "Ce choix n'existe pas.".into(),
            trace: Default::default(),
        },
    }
}

fn what(action: &Action) -> (String, String) {
    match action {
        Action::Light { entity, command } => {
            let what = match command {
                Command::TurnOff => "éteindre",
                _ => "allumer",
            };
            (entity.clone(), what.into())
        }
        Action::Setpoint { zone, setpoint } => {
            (zone.clone(), format!("consigne {}", temp(*setpoint)))
        }
    }
}

fn result_text(result: &ActionResult, simulate: bool) -> String {
    match (result, simulate) {
        (_, true) => "simulated".into(),
        (ActionResult::Ok, _) => "ok".into(),
        (ActionResult::Skipped, _) => "skipped".into(),
        (ActionResult::Failed(why), _) => format!("failed: {why}"),
    }
}

impl Hub {
    /// Answers one request from the assistant page: plans it, executes it unless simulated,
    /// and logs the turn.
    pub async fn assist(self: &Arc<Self>, req: AssistRequest) -> Result<AssistTurn, HubError> {
        self.assist_at(req, clock::now()).await
    }

    /// [`Hub::assist`] at a given time (Unix seconds), for the pending question's expiry.
    pub async fn assist_at(
        self: &Arc<Self>,
        req: AssistRequest,
        now: u64,
    ) -> Result<AssistTurn, HubError> {
        let started = Instant::now();
        let client = req.client.trim().to_owned();
        if client.is_empty() {
            return Err(HubError::Rejected("client is required".into()));
        }
        let (areas, entities) = {
            let reg = self.read();
            (area_views(&reg), entity_views(&reg))
        };
        let climate = self.climate_view().ok();
        let zones = climate
            .as_ref()
            .map(|c| c.zones.clone())
            .unwrap_or_default();
        let (outdoor, stale_after_min) = climate
            .map(|c| (c.house.outdoor_entity, c.house.stale_after_min))
            .unwrap_or((None, 60));
        let (min, max) = self.home_settings().assist_range();
        let house = House {
            areas: &areas,
            entities: &entities,
            zones: &zones,
            outdoor_entity: outdoor.as_deref(),
            stale_after_min,
            range: HeatRange { min, max },
            now,
        };
        let text = req.text.as_deref().map(str::trim).filter(|t| !t.is_empty());
        let (said, plan) = match (&req.reply, text) {
            (Some(reply), _) => {
                let (id, answer) = match reply {
                    PendingReply::Choice { pending, choice } => {
                        (pending, PendingAnswer::Choice(*choice))
                    }
                    PendingReply::Yes { pending } => (pending, PendingAnswer::Yes),
                    PendingReply::No { pending } => (pending, PendingAnswer::No),
                };
                let pending = self
                    .assist
                    .take(&client, id, now)
                    .ok_or_else(|| HubError::Rejected("question expired".into()))?;
                (said(&pending, answer), answered(&pending, answer, &house))
            }
            (None, Some(text)) => {
                let open = self.assist.current(&client, now);
                // An answer or not, a new sentence closes the question.
                self.assist.forget(&client);
                let plan = match open.and_then(|p| answer_text(text, &p).map(|a| (p, a))) {
                    Some((pending, answer)) => answered(&pending, answer, &house),
                    None => plan(text, &house),
                };
                (text.to_owned(), plan)
            }
            (None, None) => return Err(HubError::Rejected("text or reply is required".into())),
        };
        let turn = self
            .run_plan(plan, &client, said, req.simulate, &house, now)
            .await;
        let mut turn = turn;
        turn.trace.elapsed_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.assist.turns().push(turn.clone());
        self.log_turn(&turn).await;
        Ok(turn)
    }

    async fn run_plan(
        self: &Arc<Self>,
        plan: Plan,
        client: &str,
        said: String,
        simulate: bool,
        house: &House<'_>,
        now: u64,
    ) -> AssistTurn {
        let (outcome, mut text, question, trace) = match plan {
            Plan::Act {
                actions,
                done,
                mut trace,
            } => {
                let results = if simulate {
                    vec![ActionResult::Ok; actions.len()]
                } else {
                    self.execute(&actions, house, now).await
                };
                trace.actions = actions
                    .iter()
                    .zip(&results)
                    .map(|(action, result)| {
                        let (target, what) = what(action);
                        ActionTrace {
                            target,
                            what,
                            result: result_text(result, simulate),
                        }
                    })
                    .collect();
                let failed = !results.is_empty()
                    && results.iter().all(|r| matches!(r, ActionResult::Failed(_)));
                let outcome = if failed {
                    OutcomeKind::Failed
                } else {
                    OutcomeKind::Acted
                };
                (
                    outcome,
                    reply::done(&done, &results, house, simulate),
                    None,
                    trace,
                )
            }
            Plan::Answer { reply, trace } => (OutcomeKind::Answered, reply, None, trace),
            Plan::Ask {
                question,
                choices,
                trace,
            } => {
                let labels: Vec<String> = choices.iter().map(|(l, _)| l.clone()).collect();
                let id = self.assist.put(client, Pending::Choose(choices), now);
                let text = format!("{question} {}.", list_fr(&labels));
                let question = Question {
                    pending: id,
                    kind: QuestionKind::Choice,
                    choices: labels,
                };
                (OutcomeKind::Asked, text, Some(question), trace)
            }
            Plan::Confirm {
                question,
                request,
                trace,
            } => {
                let id = self.assist.put(client, Pending::Confirm(request), now);
                let q = Question {
                    pending: id,
                    kind: QuestionKind::Confirm,
                    choices: Vec::new(),
                };
                (OutcomeKind::Confirming, question, Some(q), trace)
            }
            Plan::NotUnderstood {
                reply,
                examples,
                trace,
            } => {
                let text = if examples.is_empty() {
                    reply
                } else {
                    let quoted: Vec<String> = examples.iter().map(|e| format!("« {e} »")).collect();
                    format!("{reply} Essaie par exemple : {}.", quoted.join(", "))
                };
                (OutcomeKind::NotUnderstood, text, None, trace)
            }
        };
        if simulate && !text.starts_with("Simulation :") {
            text = format!("Simulation : {text}");
        }
        AssistTurn {
            id: new_id(),
            at: now,
            client: client.to_owned(),
            text: said,
            simulate,
            outcome,
            reply: text,
            question,
            trace,
            wrong: false,
        }
    }

    /// Runs the actions, lights together; one result per action, in order. A light already
    /// in the wanted state, or offline, is skipped.
    async fn execute(
        self: &Arc<Self>,
        actions: &[Action],
        house: &House<'_>,
        now: u64,
    ) -> Vec<ActionResult> {
        join_all(actions.iter().map(|action| async move {
            match action {
                Action::Light { entity, command } => {
                    let Some(view) = house.entity(entity) else {
                        return ActionResult::Failed("inconnue".into());
                    };
                    if view.state.availability == Availability::Offline
                        || already_there(view, command)
                    {
                        return ActionResult::Skipped;
                    }
                    match self.send_command(entity, command.clone()).await {
                        Ok(response) if response.accepted => ActionResult::Ok,
                        Ok(response) => ActionResult::Failed(
                            response.reason.unwrap_or_else(|| "pas de réponse".into()),
                        ),
                        Err(err) => ActionResult::Failed(err.to_string()),
                    }
                }
                Action::Setpoint { zone, setpoint } => {
                    let ov = Override {
                        entity: None,
                        setpoint: Some(Setpoint::Temp(*setpoint)),
                        mode: None,
                        until: self.hand_hold_until(zone, now),
                        origin: OverrideOrigin::User,
                    };
                    match self.set_climate_override(zone, ov).await {
                        Ok(()) => ActionResult::Ok,
                        Err(err) => ActionResult::Failed(format!("refusé par le hub ({err})")),
                    }
                }
            }
        }))
        .await
    }

    /// The history database, when one is configured.
    fn assist_db(&self) -> Option<crate::history::db::Db> {
        self.history().and_then(|h| h.db().ok()).cloned()
    }

    /// Writes the turn to the database when there is one; a failure is logged, never
    /// returned: the memory log already has the turn.
    async fn log_turn(&self, turn: &AssistTurn) {
        let Some(db) = self.assist_db() else { return };
        match timeout(DB_TIMEOUT, db.insert_assist_turn(turn)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(%error, "could not log the assistant turn"),
            Err(_) => tracing::warn!("logging the assistant turn timed out"),
        }
    }

    /// Up to `limit` recent turns, newest first: from the database when it answers
    /// (`kept`), else from the memory log.
    pub async fn assist_turns(&self, limit: usize) -> AssistTurns {
        if let Some(db) = self.assist_db() {
            let wanted = i64::try_from(limit).unwrap_or(i64::MAX);
            match timeout(DB_TIMEOUT, db.assist_turns(wanted)).await {
                Ok(Ok(turns)) => return AssistTurns { turns, kept: true },
                Ok(Err(error)) => tracing::warn!(%error, "could not read the assistant turns"),
                Err(_) => tracing::warn!("reading the assistant turns timed out"),
            }
        }
        AssistTurns {
            turns: self.assist.turns().newest(limit),
            kept: false,
        }
    }

    /// Flags a turn as wrongly understood, in memory and in the database.
    pub async fn assist_mark_wrong(&self, id: &str) -> Result<(), HubError> {
        let mut found = self.assist.turns().mark_wrong(id);
        if let Some(db) = self.assist_db() {
            match timeout(DB_TIMEOUT, db.mark_assist_wrong(id)).await {
                Ok(Ok(marked)) => found |= marked,
                Ok(Err(error)) => tracing::warn!(%error, "could not mark the assistant turn"),
                Err(_) => tracing::warn!("marking the assistant turn timed out"),
            }
        }
        if found {
            Ok(())
        } else {
            Err(HubError::UnknownTurn(id.to_owned()))
        }
    }
}
