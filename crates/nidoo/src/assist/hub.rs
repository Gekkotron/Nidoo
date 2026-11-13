//! Planning a request against the live house and executing the plan.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use nidoo_api::{already_there, area_views, entity_views};
use nidoo_assist::plan::plan_heard;
use nidoo_assist::plan::{Action, Plan};
use nidoo_assist::reply::{self, ActionResult, temp};
use nidoo_assist::view::{
    ActionTrace, AssistRequest, AssistTurn, AssistTurns, OutcomeKind, PendingReply, Question,
    QuestionKind,
};
use nidoo_assist::{HeatRange, House, Pending, PendingAnswer, answer_text_in, plan_request};
use nidoo_climate::doc::{Override, OverrideOrigin, Setpoint};
use nidoo_core::flow::Lang;
use nidoo_core::model::{Availability, Command, StateValue};
use tokio::time::timeout;

use super::log::TurnLog;
use crate::clock;
use crate::hub::{Hub, HubError};

/// How long a database read or write of the turn log may take.
const DB_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a question waits for its answer, in seconds.
pub use nidoo_assist::pending::PENDING_TTL_S;

/// The longest client id the hub accepts (the page makes 16 characters).
pub const MAX_CLIENT_LEN: usize = 64;

/// How many pending questions the hub keeps at once; the oldest goes first.
const MAX_SLOTS: usize = 256;

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
    /// Asked in Simulation: its answer is simulated too, whatever the switch says then.
    simulate: bool,
}

/// What tapping an answer to question `id` finds.
#[derive(Debug)]
pub(crate) enum Taken {
    /// The question was open: it is now closed, and this is it (and whether it was simulated).
    Open(Box<Pending>, bool),
    /// The client has no open question, or it expired.
    Gone,
    /// The client's open question is another one; it is left as it was.
    Superseded,
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

    /// Leaves `pending` as the client's one question; returns its id. Expired questions of
    /// any client are dropped, and past [`MAX_SLOTS`] the oldest questions go.
    pub(crate) fn put(&self, client: &str, pending: Pending, now: u64, simulate: bool) -> String {
        let id = new_id();
        let mut slots = self.slots();
        slots.retain(|_, s| now < s.expires_at);
        slots.insert(
            client.to_owned(),
            Slot {
                id: id.clone(),
                pending,
                expires_at: now + PENDING_TTL_S,
                simulate,
            },
        );
        while slots.len() > MAX_SLOTS {
            let oldest = slots
                .iter()
                .filter(|(k, _)| k.as_str() != client)
                .min_by_key(|(_, s)| s.expires_at)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(key) => slots.remove(&key),
                None => break,
            };
        }
        id
    }

    /// Takes the client's question `id` when it is still open. A tap on another question
    /// (a stale button) leaves the client's current one alone.
    pub(crate) fn take(&self, client: &str, id: &str, now: u64) -> Taken {
        let mut slots = self.slots();
        slots.retain(|_, s| now < s.expires_at);
        match slots.get(client) {
            None => Taken::Gone,
            Some(slot) if slot.id != id => Taken::Superseded,
            Some(_) => match slots.remove(client) {
                Some(slot) => Taken::Open(Box::new(slot.pending), slot.simulate),
                None => Taken::Gone,
            },
        }
    }

    /// A new sentence closes the client's question: takes it, in one step, when it is still
    /// open, so that a tap and a typed answer never both get it.
    pub(crate) fn close(&self, client: &str, now: u64) -> Option<(Pending, bool)> {
        let mut slots = self.slots();
        slots.retain(|_, s| now < s.expires_at);
        slots.remove(client).map(|s| (s.pending, s.simulate))
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.slots().len()
    }
}

/// A random id, 16 hex digits.
fn new_id() -> String {
    let mut bytes = [0u8; 8];
    let _ = getrandom::getrandom(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The French text, or the English one.
fn tr<'a>(lang: Lang, fr: &'a str, en: &'a str) -> &'a str {
    match lang {
        Lang::Fr => fr,
        Lang::En => en,
    }
}

/// "Salon, Bureau ou Chambre Léa", "Kitchen, Office or Tom's room".
fn list_or(items: &[String], lang: Lang) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} {} {last}", rest.join(", "), tr(lang, "ou", "or")),
    }
}

/// What the user said by tapping an answer.
fn said(pending: &Pending, answer: PendingAnswer, lang: Lang) -> String {
    match (pending, answer) {
        (Pending::Choose(choices), PendingAnswer::Choice(i)) => choices
            .get(i)
            .map_or_else(|| tapped(answer, lang), |(label, _)| label.clone()),
        _ => tapped(answer, lang),
    }
}

/// What a tap said when its question is no longer known.
fn tapped(answer: PendingAnswer, lang: Lang) -> String {
    match answer {
        PendingAnswer::Yes => tr(lang, "Oui", "Yes").into(),
        PendingAnswer::No => tr(lang, "Non", "No").into(),
        PendingAnswer::Choice(i) => format!("{} {}", tr(lang, "choix", "choice"), i + 1),
    }
}

/// The plan an answer to a pending question leads to.
fn answered(pending: &Pending, answer: PendingAnswer, house: &House) -> Plan {
    if answer == PendingAnswer::No {
        return Plan::Answer {
            reply: tr(
                house.lang,
                "D'accord, je ne fais rien.",
                "OK, I won't do anything.",
            )
            .into(),
            trace: Default::default(),
        };
    }
    match pending.resolve(answer) {
        Some(request) => plan_request(&request, house),
        None => Plan::Answer {
            reply: tr(
                house.lang,
                "Ce choix n'existe pas.",
                "That choice doesn't exist.",
            )
            .into(),
            trace: Default::default(),
        },
    }
}

/// What an action does, for the trace, in `lang`.
fn what(action: &Action, lang: Lang) -> (String, String) {
    let (target, fr) = what_fr(action);
    if lang == Lang::Fr {
        return (target, fr);
    }
    let en = match action {
        Action::Light { command, .. } => match command {
            Command::TurnOff => "turn off".into(),
            Command::TurnOn {
                color, brightness, ..
            } => {
                let mut what = match color {
                    Some(hs) => format!("turn on in colour (hue {} °)", hs.h.round()),
                    None => "turn on".into(),
                };
                if let Some(b) = brightness {
                    what.push_str(&format!(" at {b} of 255"));
                }
                what
            }
            _ => "turn on".into(),
        },
        Action::Switch { command, .. } => match command {
            Command::TurnOff => "turn off".into(),
            _ => "turn on".into(),
        },
        Action::Setpoint { setpoint, .. } => {
            format!("setpoint {} °C", (setpoint * 10.0).round() / 10.0)
        }
        Action::Media { command, .. } => match command {
            Command::PlayMedia { query } => format!("play \"{query}\""),
            Command::MediaAction { action } => match action.as_str() {
                "pause" => "pause".into(),
                "next" => "next".into(),
                "previous" => "previous".into(),
                _ => "play".into(),
            },
            _ => "play".into(),
        },
        Action::Speaker { command, .. } => match command {
            Command::SetNumber { value } => format!("volume {} %", value.round()),
            Command::TurnOff => "unmute".into(),
            _ => "mute".into(),
        },
        Action::Cover { command, .. } => match command {
            Command::SetPosition { position } => format!("open to {position} %"),
            Command::Close => "close".into(),
            _ => "open".into(),
        },
    };
    (target, en)
}

fn what_fr(action: &Action) -> (String, String) {
    match action {
        Action::Light { entity, command } => {
            let what = match command {
                Command::TurnOff => "éteindre".into(),
                Command::TurnOn {
                    color, brightness, ..
                } => {
                    let mut what = match color {
                        Some(hs) => {
                            format!("allumer en couleur (teinte {} °)", reply::number(hs.h))
                        }
                        None => "allumer".into(),
                    };
                    if let Some(b) = brightness {
                        what.push_str(&format!(" à {b} sur 255"));
                    }
                    what
                }
                _ => "allumer".into(),
            };
            (entity.clone(), what)
        }
        Action::Switch { entity, command } => {
            let what = match command {
                Command::TurnOff => "éteindre",
                _ => "allumer",
            };
            (entity.clone(), what.into())
        }
        Action::Setpoint { zone, setpoint } => {
            (zone.clone(), format!("consigne {}", temp(*setpoint)))
        }
        Action::Media { entity, command } => {
            let what = match command {
                Command::PlayMedia { query } => format!("jouer « {query} »"),
                Command::MediaAction { action } => match action.as_str() {
                    "pause" => "pause".into(),
                    "next" => "suivante".into(),
                    "previous" => "précédente".into(),
                    _ => "lecture".into(),
                },
                _ => "lecture".into(),
            };
            (entity.clone(), what)
        }
        Action::Speaker { entity, command } => {
            let what = match command {
                Command::SetNumber { value } => format!("volume {} %", reply::number(*value)),
                Command::TurnOff => "remettre le son".into(),
                _ => "couper le son".into(),
            };
            (entity.clone(), what)
        }
        Action::Cover { entity, command } => {
            let what = match command {
                Command::SetPosition { position } => format!("ouvrir à {position} %"),
                Command::Close => "fermer".into(),
                _ => "ouvrir".into(),
            };
            (entity.clone(), what)
        }
    }
}

/// One action's result, and the English detail of a failure (kept for the trace and the log;
/// the result itself carries French for the reply).
type Executed = (ActionResult, Option<String>);

fn result_text(result: &Executed, simulate: bool) -> String {
    let base = match result {
        (ActionResult::Ok | ActionResult::Echo(_), _) => "ok".to_owned(),
        (ActionResult::Skipped, _) => "skipped".to_owned(),
        (ActionResult::Offline, _) => "offline".to_owned(),
        (ActionResult::Failed(fr), detail) => {
            format!("failed: {}", detail.as_deref().unwrap_or(fr))
        }
    };
    if simulate {
        format!("simulated {base}")
    } else {
        base
    }
}

/// A short reason in `lang` for a refusal of the device-command layer.
pub(crate) fn reason(english: &str, lang: Lang) -> &'static str {
    match lang {
        Lang::Fr => reason_fr(english),
        Lang::En => reason_en(english),
    }
}

/// [`reason_fr`] in English: the same short reasons.
fn reason_en(english: &str) -> &'static str {
    let e = english.to_lowercase();
    let has = |needle: &str| e.contains(needle);
    if has("no state change") || has("no echo") {
        reply::NO_ANSWER_EN
    } else if has("groups disabled") {
        "groups are disabled"
    } else if has("no reachable member") {
        "no device can be reached"
    } else if has("unknown group") || has("unknown entity") {
        "unknown device"
    } else if has("publish") || has("broker") || has("disconnected") {
        "the link to the broker is down"
    } else if has("shutting down") {
        "the hub is shutting down"
    } else if has("not active") {
        "the zone is not active"
    } else if has("offline") || has("unavailable") {
        "offline"
    } else {
        "refused by the hub"
    }
}

/// A short French reason for a refusal of the device-command layer, which words them in
/// English. Anything unknown gets a generic text; the English stays in the trace.
pub(crate) fn reason_fr(english: &str) -> &'static str {
    let e = english.to_lowercase();
    let has = |needle: &str| e.contains(needle);
    if has("no state change") || has("no echo") {
        "pas de réponse de l'appareil"
    } else if has("groups disabled") {
        "les groupes sont désactivés"
    } else if has("no reachable member") {
        "aucun appareil joignable"
    } else if has("unknown group") || has("unknown entity") {
        "appareil inconnu"
    } else if has("publish") || has("broker") || has("disconnected") {
        "le lien avec le broker est coupé"
    } else if has("shutting down") {
        "le hub s'arrête"
    } else if has("not active") {
        "la zone n'est pas active"
    } else if has("offline") || has("unavailable") {
        "hors ligne"
    } else {
        "refusé par le hub"
    }
}

impl Hub {
    /// Answers one request from the assistant page: plans it, executes it unless simulated,
    /// and logs the turn.
    ///
    /// A turn from a `satellite:` client (speech input) may come from anyone in earshot, or
    /// from a TV: a future security-sensitive intent (alarm, locks, garage or entrance doors,
    /// covers that open the house) must refuse it unless that intent is enabled for voice.
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
        if client.chars().count() > MAX_CLIENT_LEN {
            return Err(HubError::Rejected(format!(
                "client is too long ({MAX_CLIENT_LEN} characters at most)"
            )));
        }
        // A satellite heard the sentence: its room picks the player and the speakers.
        let (areas, entities, speaker_room) = {
            let reg = self.read();
            let speaker_room = client
                .strip_prefix("satellite:")
                .and_then(|sid| crate::sound::room_of(&reg, sid));
            (area_views(&reg), entity_views(&reg), speaker_room)
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
        let lang = req.lang();
        let house = House {
            areas: &areas,
            entities: &entities,
            zones: &zones,
            outdoor_entity: outdoor.as_deref(),
            stale_after_min,
            range: HeatRange { min, max },
            now,
            lang,
        };
        let text = req.text.as_deref().map(str::trim).filter(|t| !t.is_empty());
        let mut simulate = req.simulate;
        let (said, plan) = match (&req.reply, text) {
            (Some(reply), _) => {
                let (id, answer) = match reply {
                    PendingReply::Choice { pending, choice } => {
                        (pending, PendingAnswer::Choice(*choice))
                    }
                    PendingReply::Yes { pending } => (pending, PendingAnswer::Yes),
                    PendingReply::No { pending } => (pending, PendingAnswer::No),
                };
                match self.assist.take(&client, id, now) {
                    Taken::Open(pending, asked_simulated) => {
                        simulate |= asked_simulated;
                        (
                            said(&pending, answer, lang),
                            answered(&pending, answer, &house),
                        )
                    }
                    Taken::Gone => return Err(HubError::Rejected("question expired".into())),
                    // A stale button: the current question stays open.
                    Taken::Superseded => (
                        tapped(answer, lang),
                        Plan::Answer {
                            reply: tr(
                                lang,
                                "Cette question n'est plus d'actualité : réponds à la \
                                 dernière, ou écris une nouvelle demande.",
                                "That question is out of date: answer the latest one, or type \
                                 a new request.",
                            )
                            .into(),
                            trace: Default::default(),
                        },
                    ),
                }
            }
            (None, Some(text)) => {
                // An answer or not, a new sentence closes the question, and takes it in one
                // step so that no tap can answer it too.
                let open = self.assist.close(&client, now);
                let plan = match open
                    .and_then(|(p, sim)| answer_text_in(text, &p, lang).map(|a| (p, a, sim)))
                {
                    Some((pending, answer, asked_simulated)) => {
                        simulate |= asked_simulated;
                        answered(&pending, answer, &house)
                    }
                    None => plan_heard(text, &house, speaker_room.as_deref()),
                };
                (text.to_owned(), plan)
            }
            (None, None) => return Err(HubError::Rejected("text or reply is required".into())),
        };
        let turn = self
            .run_plan(plan, &client, said, simulate, &house, now)
            .await;
        let mut turn = turn;
        turn.trace.elapsed_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.assist.turns().push(turn.clone());
        // The open pages show it as it happens, a turn heard by a satellite included.
        self.broadcast(nidoo_api::WsEvent::AssistTurnAdded {
            id: turn.id.clone(),
        });
        // The database may be slow: the reply never waits for it.
        if self.assist_db().is_some() {
            let hub = Arc::clone(self);
            let logged = turn.clone();
            tokio::spawn(async move { hub.log_turn(&logged).await });
        }
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
                let executed = self.execute(&actions, house, now, simulate).await;
                trace.actions = actions
                    .iter()
                    .zip(&executed)
                    .map(|(action, result)| {
                        let (target, what) = what(action, house.lang);
                        ActionTrace {
                            target,
                            what,
                            result: result_text(result, simulate),
                        }
                    })
                    .collect();
                let results: Vec<ActionResult> = executed.into_iter().map(|(r, _)| r).collect();
                let failed = !results.is_empty()
                    && results
                        .iter()
                        .all(|r| matches!(r, ActionResult::Failed(_) | ActionResult::Offline));
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
                let id = self
                    .assist
                    .put(client, Pending::Choose(choices), now, simulate);
                let text = format!("{question} {}.", list_or(&labels, house.lang));
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
                let id = self
                    .assist
                    .put(client, Pending::Confirm(request), now, simulate);
                let q = Question {
                    pending: id,
                    kind: QuestionKind::Confirm,
                    choices: Vec::new(),
                };
                (OutcomeKind::Confirming, question, Some(q), trace)
            }
            Plan::NotUnderstood { reply, trace } => {
                (OutcomeKind::NotUnderstood, reply, None, trace)
            }
        };
        let prefix = tr(house.lang, "Simulation : ", "Simulation: ");
        if simulate && !text.starts_with(prefix) {
            text = format!("{prefix}{text}");
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
    /// in the wanted state, or offline, is skipped. In Simulation the same checks run and the
    /// results are what a real run would report, but nothing is sent.
    async fn execute(
        self: &Arc<Self>,
        actions: &[Action],
        house: &House<'_>,
        now: u64,
        simulate: bool,
    ) -> Vec<Executed> {
        let failed = |english: String| {
            let said = reason(&english, house.lang).to_owned();
            (ActionResult::Failed(said), Some(english))
        };
        join_all(actions.iter().map(|action| async move {
            match action {
                Action::Light { entity, command }
                | Action::Cover { entity, command }
                | Action::Switch { entity, command }
                | Action::Media { entity, command }
                | Action::Speaker { entity, command } => {
                    let Some(view) = house.entity(entity) else {
                        let unknown = tr(house.lang, "inconnue", "unknown");
                        return (ActionResult::Failed(unknown.into()), None);
                    };
                    if view.state.availability == Availability::Offline {
                        return (ActionResult::Offline, None);
                    }
                    if already_there(view, command) {
                        return (ActionResult::Skipped, None);
                    }
                    if simulate {
                        return (ActionResult::Ok, None);
                    }
                    match self.send_command(entity, command.clone()).await {
                        // A media player's echo carries the song the reply names.
                        Ok(response) if response.accepted => match response.state {
                            Some(state) if matches!(action, Action::Media { .. }) => {
                                match state.value {
                                    StateValue::MediaPlayer(player) => {
                                        (ActionResult::Echo(player), None)
                                    }
                                    _ => (ActionResult::Ok, None),
                                }
                            }
                            _ => (ActionResult::Ok, None),
                        },
                        Ok(response) => {
                            failed(response.reason.unwrap_or_else(|| "no response".into()))
                        }
                        Err(err) => failed(err.to_string()),
                    }
                }
                Action::Setpoint { zone, setpoint } => {
                    if simulate {
                        return (ActionResult::Ok, None);
                    }
                    let ov = Override {
                        entity: None,
                        setpoint: Some(Setpoint::Temp(*setpoint)),
                        mode: None,
                        until: self.hand_hold_until(zone, now),
                        origin: OverrideOrigin::User,
                    };
                    match self.replace_climate_overrides(zone, ov).await {
                        Ok(()) => (ActionResult::Ok, None),
                        Err(err) => failed(err.to_string()),
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
            // The row is written in the background: a "Faux" tapped before it existed
            // updated nothing in the database, so carry the flag over now.
            Ok(Ok(())) if self.assist.turns().is_wrong(&turn.id) => {
                match timeout(DB_TIMEOUT, db.mark_assist_wrong(&turn.id)).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => tracing::warn!(%error, "could not mark the assistant turn"),
                    Err(_) => tracing::warn!("marking the assistant turn timed out"),
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_assist::Request;
    use nidoo_assist::intent::Intent;

    fn confirm() -> Pending {
        Pending::Confirm(Request::new(Intent::ReadTemp {
            room: vec![],
            outdoor: true,
        }))
    }

    #[test]
    fn a_stale_id_leaves_the_current_question() {
        let state = AssistState::default();
        let id = state.put("c", confirm(), 100, false);
        assert!(matches!(state.take("c", "old", 101), Taken::Superseded));
        assert!(matches!(state.take("c", &id, 102), Taken::Open(..)));
        assert!(matches!(state.take("c", &id, 103), Taken::Gone));
    }

    #[test]
    fn a_tap_and_a_typed_answer_have_one_winner() {
        let state = AssistState::default();
        let id = state.put("c", confirm(), 100, false);
        assert!(state.close("c", 101).is_some());
        assert!(matches!(state.take("c", &id, 101), Taken::Gone));
        let id = state.put("c", confirm(), 100, false);
        assert!(matches!(state.take("c", &id, 101), Taken::Open(..)));
        assert!(state.close("c", 101).is_none());
    }

    #[test]
    fn an_expired_question_is_gone() {
        let state = AssistState::default();
        let id = state.put("c", confirm(), 100, false);
        assert!(matches!(
            state.take("c", &id, 100 + PENDING_TTL_S),
            Taken::Gone
        ));
        state.put("c", confirm(), 100, false);
        assert!(state.close("c", 100 + PENDING_TTL_S).is_none());
    }

    #[test]
    fn other_clients_expired_slots_are_pruned() {
        let state = AssistState::default();
        for i in 0..10 {
            state.put(&format!("gone-{i}"), confirm(), 100, false);
        }
        assert_eq!(state.len(), 10);
        state.put("fresh", confirm(), 100 + PENDING_TTL_S, false);
        assert_eq!(state.len(), 1);
        state.put("other", confirm(), 100 + PENDING_TTL_S, false);
        let _ = state.take("fresh", "x", 100 + 2 * PENDING_TTL_S);
        assert_eq!(state.len(), 0);
    }

    #[test]
    fn the_slots_are_capped_and_the_oldest_go() {
        let state = AssistState::default();
        state.put("first", confirm(), 100, false);
        for i in 0..MAX_SLOTS + 10 {
            state.put(&format!("c{i}"), confirm(), 150, false);
        }
        assert_eq!(state.len(), MAX_SLOTS);
        assert!(state.close("first", 151).is_none());
        let last = MAX_SLOTS + 9;
        assert!(state.close(&format!("c{last}"), 151).is_some());
    }

    #[test]
    fn refusals_are_told_in_french() {
        assert_eq!(
            reason_fr("no state change within 50 ms"),
            "pas de réponse de l'appareil"
        );
        assert_eq!(reason_fr("Led: no echo"), "pas de réponse de l'appareil");
        assert_eq!(reason_fr("groups disabled"), "les groupes sont désactivés");
        assert_eq!(
            reason_fr("publish: broker link down"),
            "le lien avec le broker est coupé"
        );
        assert_eq!(
            reason_fr("rejected: zone is not active"),
            "la zone n'est pas active"
        );
        assert_eq!(reason_fr("something new"), "refusé par le hub");
    }
}
