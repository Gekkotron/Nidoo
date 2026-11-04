//! `/assistant`: type a French sentence, read the hub's reply, answer its question with a tap,
//! and see how the sentence was understood. The replies are French and written by the hub;
//! the page chrome follows the language setting. When the hub has a speech model in use, each
//! new reply is read aloud (a header switch turns that off, per browser) and ▶ reads one again.

use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_api::SpeechRequest;
use nidoo_assist::pending::PENDING_TTL_S;
use nidoo_assist::view::{AssistRequest, AssistTurn, PendingReply, Question, QuestionKind, Trace};
use wasm_bindgen_futures::JsFuture;

use crate::api::assist::{get_turns, mark_wrong, post_assist};
use crate::api::tts;
use crate::components::toast::Toasts;
use crate::i18n::{
    AssistSettingsText, AssistantText, Lang, assist_settings_text, assistant_text, use_lang,
};
use crate::icons::{GEAR, PLAY, SPEAKER, SPEAKER_OFF};

/// The id this browser gives the hub, so its pending question is its own.
/// The language the Assistant's replies are written in, and so spoken in.
const REPLY_LANGUAGE: &str = "fr";

const CLIENT_KEY: &str = "nidoo.assist.client";
/// Whether the Simulation switch is on, per browser.
const SIMULATE_KEY: &str = "nidoo.assist.simulate";
/// How many past turns the page shows when it opens.
const LIMIT: u32 = 50;
/// The sentences sent from this browser, oldest first, for the up arrow.
const HISTORY_KEY: &str = "nidoo.assist.history";
/// How many sent sentences the up arrow can reach.
const HISTORY_MAX: usize = 50;
/// Where this browser last cleared the conversation (a [`ClearMark`]).
const CLEARED_KEY: &str = "nidoo.assist.cleared";
/// Whether replies are read aloud, per browser: `"0"` when turned off, on otherwise.
const SPEAK_KEY: &str = "nidoo.assist.speak";
/// A tenth of a second of silence. Played on the click or key that sends a sentence, it lets
/// Safari play the reply that arrives later on the same element.
const SILENCE: &str = "data:audio/wav;base64,UklGRnQAAABXQVZFZm10IBAAAAABAAEAQB8AAEAfAAABAAgAZGF0YVAAAACAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgA==";

/// The sentences sent, browsed with the arrows as in a shell: up for older, down for newer,
/// and down past the newest gives back the sentence being typed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    /// Oldest first.
    sent: Vec<String>,
    /// The sentence shown while browsing, and the one typed before browsing began.
    browsing: Option<(usize, String)>,
}

impl History {
    pub fn new(sent: Vec<String>) -> Self {
        let mut h = Self::default();
        for s in sent {
            h.push(s);
        }
        h
    }

    pub fn sent(&self) -> &[String] {
        &self.sent
    }

    /// Keeps a sent sentence (once when said twice in a row) and stops browsing.
    pub fn push(&mut self, sentence: String) {
        self.browsing = None;
        if sentence.is_empty() || self.sent.last() == Some(&sentence) {
            return;
        }
        self.sent.push(sentence);
        if self.sent.len() > HISTORY_MAX {
            self.sent.remove(0);
        }
    }

    /// The older sentence to show, `typed` being the field's text; `None` when there is none.
    pub fn up(&mut self, typed: &str) -> Option<String> {
        let at = match &self.browsing {
            None if self.sent.is_empty() => return None,
            None => self.sent.len() - 1,
            Some((0, _)) => return None,
            Some((at, _)) => at - 1,
        };
        let draft = self
            .browsing
            .take()
            .map_or_else(|| typed.to_owned(), |(_, d)| d);
        self.browsing = Some((at, draft));
        Some(self.sent[at].clone())
    }

    /// The newer sentence to show, or the one typed before browsing; `None` when not browsing.
    pub fn down(&mut self) -> Option<String> {
        let (at, draft) = self.browsing.take()?;
        if at + 1 < self.sent.len() {
            self.browsing = Some((at + 1, draft));
            Some(self.sent[at + 1].clone())
        } else {
            Some(draft)
        }
    }

    /// The field was edited: the shown sentence becomes the one being typed.
    pub fn edited(&mut self) {
        self.browsing = None;
    }
}

fn stored_history() -> History {
    let sent = storage()
        .and_then(|s| s.get_item(HISTORY_KEY).ok().flatten())
        .and_then(|v| serde_json::from_str::<Vec<String>>(&v).ok())
        .unwrap_or_default();
    History::new(sent)
}

fn store_history(h: &History) {
    if let (Some(s), Ok(v)) = (storage(), serde_json::to_string(h.sent())) {
        let _ = s.set_item(HISTORY_KEY, &v);
    }
}

/// The newest turn when the conversation was cleared: it and the older ones stay hidden in this
/// browser. The hub keeps them all, and the up arrow still reaches the sentences.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClearMark {
    pub id: String,
    /// Hub Unix seconds.
    pub at: u64,
}

impl ClearMark {
    /// The mark for clearing `turns` (oldest first); `None` when there is nothing to clear.
    pub fn of(turns: &[AssistTurn]) -> Option<Self> {
        turns.last().map(|t| Self {
            id: t.id.clone(),
            at: t.at,
        })
    }
}

/// The turns (oldest first) said after the clear `mark`: those after the marked turn, or, once
/// it is no longer listed, those from a later second.
pub fn after_clear(mut turns: Vec<AssistTurn>, mark: Option<&ClearMark>) -> Vec<AssistTurn> {
    let Some(mark) = mark else {
        return turns;
    };
    match turns.iter().position(|t| t.id == mark.id) {
        Some(at) => turns.split_off(at + 1),
        None => {
            turns.retain(|t| t.at > mark.at);
            turns
        }
    }
}

fn stored_clear_mark() -> Option<ClearMark> {
    storage()
        .and_then(|s| s.get_item(CLEARED_KEY).ok().flatten())
        .and_then(|v| serde_json::from_str(&v).ok())
}

fn store_clear_mark(mark: &ClearMark) {
    if let (Some(s), Ok(v)) = (storage(), serde_json::to_string(mark)) {
        let _ = s.set_item(CLEARED_KEY, &v);
    }
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

/// Where a turn came from when a satellite heard it (`satellite:<sid>`): the satellite's name
/// and room, its id when the house does not know it; `None` for a browser.
pub fn turn_source(
    client: &str,
    entities: &std::collections::BTreeMap<String, nidoo_api::EntityView>,
    areas: &[nidoo_api::AreaView],
) -> Option<String> {
    let sid = client.strip_prefix("satellite:")?;
    let volume = format!("{}{sid}_volume", nidoo_core::model::SOUND_DEVICE_PREFIX);
    let Some(entity) = entities.get(&volume) else {
        return Some(sid.to_owned());
    };
    let name = entity
        .device_name
        .clone()
        .unwrap_or_else(|| entity.name.clone());
    Some(match crate::format::room_name(areas, &entity.area) {
        Some(room) => format!("{name} · {room}"),
        None => name,
    })
}

/// This browser's client id, made once and kept in `localStorage` (a fresh one each visit
/// when storage is blocked).
fn client_id() -> String {
    if let Some(id) = storage()
        .and_then(|s| s.get_item(CLIENT_KEY).ok().flatten())
        .filter(|id| !id.is_empty())
    {
        return id;
    }
    let id: String = (0..16)
        .map(|_| char::from_digit((js_sys::Math::random() * 16.0) as u32 % 16, 16).unwrap_or('0'))
        .collect();
    if let Some(s) = storage() {
        let _ = s.set_item(CLIENT_KEY, &id);
    }
    id
}

fn stored_speak() -> bool {
    storage()
        .and_then(|s| s.get_item(SPEAK_KEY).ok().flatten())
        .is_none_or(|v| v != "0")
}

/// Whether a failed `play()` only means a newer clip took the element over.
fn superseded(err: &wasm_bindgen::JsValue) -> bool {
    js_sys::Reflect::get(err, &"name".into())
        .ok()
        .and_then(|name| name.as_string())
        .is_some_and(|name| name == "AbortError")
}

fn stored_simulate() -> bool {
    storage()
        .and_then(|s| s.get_item(SIMULATE_KEY).ok().flatten())
        .is_some_and(|v| v == "1")
}

/// How long the answer buttons of a question asked at `at` (hub Unix seconds) stay worth
/// showing, in milliseconds from `now_ms`: the hub forgets a question after
/// [`PENDING_TTL_S`]. Never more than the full window, whatever the two clocks say.
pub fn chip_window_ms(at: u64, now_ms: f64) -> f64 {
    let window = PENDING_TTL_S as f64 * 1000.0;
    ((at as f64 * 1000.0 + window) - now_ms).clamp(0.0, window)
}

/// The rows of the trace panel: label and value, the empty ones left out.
pub fn trace_rows(lang: Lang, trace: &Trace) -> Vec<(&'static str, String)> {
    let t = |k| assistant_text(lang, k);
    let mut rows = Vec::new();
    let mut push = |k, v: Option<String>| {
        if let Some(v) = v.filter(|v| !v.is_empty()) {
            rows.push((t(k), v));
        }
    };
    push(AssistantText::Words, Some(trace.words.join(" ")));
    push(AssistantText::Intent, trace.intent.clone());
    push(
        AssistantText::Room,
        trace.room.as_ref().map(|room| match &trace.room_how {
            Some(how) => format!("{room} ({how})"),
            None => room.clone(),
        }),
    );
    push(AssistantText::Entities, Some(trace.entities.join(", ")));
    push(AssistantText::Value, trace.value.clone());
    push(AssistantText::Gate, trace.gate.clone());
    push(
        AssistantText::Actions,
        Some(
            trace
                .actions
                .iter()
                .map(|a| format!("{} → {} : {}", a.target, a.what, a.result))
                .collect::<Vec<_>>()
                .join(" · "),
        ),
    );
    push(
        AssistantText::Elapsed,
        Some(format!("{} µs", trace.elapsed_us)),
    );
    rows
}

/// The conversation, the field and the trace of the selected turn.
#[component]
pub fn AssistantScreen() -> impl IntoView {
    let lang = use_lang();
    let t = move |k| assistant_text(lang(), k);
    let toasts = use_context::<Toasts>();
    let client = StoredValue::new(client_id());
    // The satellite a spoken turn came from, by name and room (no house in some tests).
    let house = use_context::<crate::house::House>();
    let source = move |client: &str| {
        let house = house.as_ref()?;
        house.entities.with_untracked(|entities| {
            house
                .areas
                .with_untracked(|areas| turn_source(client, entities, areas))
        })
    };
    // Oldest first, as the conversation reads.
    let turns = RwSignal::new(Vec::<AssistTurn>::new());
    let kept = RwSignal::new(None::<bool>);
    let selected = RwSignal::new(None::<String>);
    let text = RwSignal::new(String::new());
    let simulate = RwSignal::new(stored_simulate());
    let busy = RwSignal::new(false);
    // Questions whose buttons are no longer offered: expired, or refused by the hub.
    let closed = RwSignal::new(Vec::<String>::new());
    // The turn last received from the hub and when (page clock, ms): its buttons are timed
    // from then, so a hub clock that differs from the browser's never hides them.
    let received = RwSignal::new(None::<(String, f64)>);
    let input = NodeRef::<leptos::html::Input>::new();
    let history = StoredValue::new(stored_history());
    // Whether the hub can speak the replies (French, with a model) and whether this browser
    // wants it.
    let can_speak = RwSignal::new(false);
    let speak_on = RwSignal::new(stored_speak());
    let audio = NodeRef::<leptos::html::Audio>::new();

    spawn_local(async move {
        if let Ok(status) = tts::get_status().await {
            let french = status
                .languages
                .iter()
                .any(|l| l.code == REPLY_LANGUAGE && l.model.is_some());
            let _ = can_speak.try_set(status.speech_built && french);
        }
    });
    Effect::new(move |_| {
        let on = speak_on.get();
        if let Some(s) = storage() {
            let _ = s.set_item(SPEAK_KEY, if on { "1" } else { "0" });
        }
    });
    let hush = move || {
        if let Some(el) = audio.get_untracked() {
            let _ = el.pause();
        }
    };
    // Runs inside the click or key press, before any await: Safari plays later only on an
    // element that already played in a user gesture.
    let unlock = move || {
        if !can_speak.get_untracked() {
            return;
        }
        if let Some(el) = audio.get_untracked()
            && el.src().is_empty()
        {
            el.set_src(SILENCE);
            if let Ok(promise) = el.play() {
                spawn_local(async move {
                    let _ = JsFuture::from(promise).await;
                });
            }
        }
    };
    let say = move |text: String| {
        if text.trim().is_empty() {
            return;
        }
        spawn_local(async move {
            let failed = match tts::speak(&SpeechRequest {
                text,
                language: Some(REPLY_LANGUAGE.to_owned()),
                voice: None,
            })
            .await
            {
                Ok(ticket) => match audio.get_untracked() {
                    Some(el) => {
                        el.set_src(&ticket.url);
                        let refused = match el.play() {
                            Ok(promise) => JsFuture::from(promise)
                                .await
                                .err()
                                .filter(|err| !superseded(err)),
                            Err(err) => Some(err),
                        };
                        refused.map(|_| {
                            assist_settings_text(lang(), AssistSettingsText::PlayFailed).to_owned()
                        })
                    }
                    None => None,
                },
                Err(err) => Some(format!("{} {}", t(AssistantText::SpeakFailed), err.error)),
            };
            if let (Some(message), Some(toasts)) = (failed, toasts) {
                toasts.push(message);
            }
        });
    };

    spawn_local(async move {
        if let Ok(list) = get_turns(LIMIT).await {
            let mut list_turns = list.turns;
            list_turns.reverse();
            let _ = turns.try_set(after_clear(list_turns, stored_clear_mark().as_ref()));
            let _ = kept.try_set(Some(list.kept));
        }
    });
    Effect::new(move |_| {
        if let Some(field) = input.get() {
            let _ = field.focus();
        }
    });
    Effect::new(move |_| {
        let on = simulate.get();
        if let Some(s) = storage() {
            let _ = s.set_item(SIMULATE_KEY, if on { "1" } else { "0" });
        }
    });

    let submit = move |req: AssistRequest| {
        busy.set(true);
        let tapped = req.reply.as_ref().map(|reply| match reply {
            PendingReply::Choice { pending, .. }
            | PendingReply::Yes { pending }
            | PendingReply::No { pending } => pending.clone(),
        });
        spawn_local(async move {
            match post_assist(&req).await {
                // `try_`: the page may be gone by the time the hub answers.
                Ok(turn) => {
                    let id = turn.id.clone();
                    if can_speak.get_untracked() && speak_on.get_untracked() {
                        say(turn.reply.clone());
                    }
                    received.try_set(Some((id.clone(), js_sys::Date::now())));
                    turns.try_update(|list| list.push(turn));
                    selected.try_set(Some(id));
                    if kept.try_get_untracked() == Some(None) {
                        kept.try_set(Some(true));
                    }
                }
                Err(err) => {
                    // The hub no longer holds that question: stop offering its buttons.
                    if let Some(id) = tapped {
                        closed.try_update(|ids| ids.push(id));
                    }
                    if let Some(toasts) = toasts {
                        toasts.push(format!("{} {}", t(AssistantText::Failed), err.error));
                    }
                }
            }
            let _ = busy.try_set(false);
        });
    };
    let send_text = move || {
        let sentence = text.get_untracked().trim().to_owned();
        if sentence.is_empty() || busy.get_untracked() {
            return;
        }
        unlock();
        text.set(String::new());
        history.update_value(|h| {
            h.push(sentence.clone());
            store_history(h);
        });
        submit(AssistRequest {
            client: client.get_value(),
            text: Some(sentence),
            reply: None,
            simulate: simulate.get_untracked(),
        });
    };
    let answer = move |reply: PendingReply| {
        if busy.get_untracked() {
            return;
        }
        unlock();
        submit(AssistRequest {
            client: client.get_value(),
            text: None,
            reply: Some(reply),
            simulate: simulate.get_untracked(),
        });
    };

    // Only the latest turn's question can still be open, until it expires or the hub refuses it.
    let open_question = Memo::new(move |_| {
        turns.with(|list| {
            let last = list.last()?;
            let q = last.question.clone()?;
            closed
                .with(|ids| !ids.contains(&q.pending))
                .then_some((q, last.at, last.id.clone()))
        })
    });
    // The question's buttons go away when the hub forgets it.
    Effect::new(move |_| {
        if let Some((q, at, turn)) = open_question.get() {
            let now = js_sys::Date::now();
            let left = match received.get_untracked() {
                Some((id, ms)) if id == turn => chip_window_ms(0, now - ms),
                _ => chip_window_ms(at, now),
            };
            let id = q.pending;
            if left <= 0.0 {
                closed.try_update(|ids| ids.push(id));
            } else {
                set_timeout(
                    move || {
                        closed.try_update(|ids| ids.push(id));
                    },
                    std::time::Duration::from_millis(left as u64),
                );
            }
        }
    });
    let shown = Memo::new(move |_| {
        turns.with(|list| {
            let id = selected.get();
            id.and_then(|id| list.iter().find(|turn| turn.id == id).cloned())
                .or_else(|| list.last().cloned())
        })
    });
    // Hides the conversation in this browser; the hub and the up arrow keep it.
    let clear = move || {
        let Some(mark) = turns.with_untracked(|list| ClearMark::of(list)) else {
            return;
        };
        store_clear_mark(&mark);
        hush();
        turns.set(Vec::new());
        selected.set(None);
        if let Some(field) = input.get_untracked() {
            let _ = field.focus();
        }
    };
    let mark = move |id: String| {
        spawn_local(async move {
            if mark_wrong(&id).await.is_ok() {
                turns.try_update(|list| {
                    if let Some(turn) = list.iter_mut().find(|turn| turn.id == id) {
                        turn.wrong = true;
                    }
                });
            }
        });
    };

    view! {
        <div class="assist">
            <section class="assist__talk">
                <div class="assist__head">
                    <p class="assist__hint">{move || t(AssistantText::Hint)}</p>
                    <div class="assist__tools">
                        <Show when=move || can_speak.get()>
                            <button
                                type="button"
                                class="assist__speak"
                                aria-label=move || t(AssistantText::Speak)
                                aria-pressed=move || if speak_on.get() { "true" } else { "false" }
                                title=move || {
                                    t(if speak_on.get() {
                                        AssistantText::SpeakOn
                                    } else {
                                        AssistantText::SpeakOff
                                    })
                                }
                                inner_html=move || if speak_on.get() { SPEAKER } else { SPEAKER_OFF }
                                on:click=move |_| {
                                    let on = !speak_on.get_untracked();
                                    if on {
                                        unlock();
                                    } else {
                                        hush();
                                    }
                                    speak_on.set(on);
                                }
                            ></button>
                        </Show>
                        <a
                            class="assist__settings"
                            href="/assistant/settings"
                            aria-label=move || assist_settings_text(lang(), AssistSettingsText::OpenSettings)
                            title=move || assist_settings_text(lang(), AssistSettingsText::OpenSettings)
                            inner_html=GEAR
                        ></a>
                        <button
                            type="button"
                            class="assist__clear"
                            title=move || t(AssistantText::ClearHint)
                            disabled=move || turns.with(Vec::is_empty)
                            on:click=move |_| clear()
                        >
                            {move || t(AssistantText::Clear)}
                        </button>
                    </div>
                </div>
                <audio node_ref=audio class="assist__audio" preload="none"></audio>
                <Show when=move || kept.get() == Some(false)>
                    <p class="assist__note">{move || t(AssistantText::NotKept)}</p>
                </Show>
                <ol class="assist__turns">
                    <For
                        each=move || turns.get()
                        key=|turn| (turn.id.clone(), turn.wrong)
                        children=move |turn| {
                            let id = turn.id.clone();
                            let is_selected = {
                                let id = id.clone();
                                move || shown.with(|s| s.as_ref().is_some_and(|s| s.id == id))
                            };
                            view! {
                                <li
                                    class="assist__turn"
                                    class:assist__turn--selected=is_selected
                                    class:assist__turn--wrong=turn.wrong
                                    on:click=move |_| selected.set(Some(id.clone()))
                                >
                                    {source(&turn.client).map(|from| {
                                        view! { <p class="assist__source">{from}</p> }
                                    })}
                                    <p class="assist__said">{turn.text.clone()}</p>
                                    <div class="assist__answer">
                                        <p class="assist__reply">{turn.reply.clone()}</p>
                                        <Show when=move || can_speak.get()>
                                            {
                                                let reply = turn.reply.clone();
                                                view! {
                                                    <button
                                                        type="button"
                                                        class="assist__play"
                                                        aria-label=move || t(AssistantText::Listen)
                                                        title=move || t(AssistantText::Listen)
                                                        inner_html=PLAY
                                                        on:click=move |ev| {
                                                            ev.stop_propagation();
                                                            unlock();
                                                            say(reply.clone());
                                                        }
                                                    ></button>
                                                }
                                            }
                                        </Show>
                                    </div>
                                </li>
                            }
                        }
                    />
                </ol>
                {move || open_question.get().map(|(q, ..)| chips(q, lang(), answer))}
                <div class="assist__bar">
                    <input
                        node_ref=input
                        class="assist__input"
                        type="text"
                        autocomplete="off"
                        placeholder=move || t(AssistantText::Placeholder)
                        aria-label=move || t(AssistantText::Placeholder)
                        prop:value=move || text.get()
                        on:input=move |ev| {
                            history.update_value(History::edited);
                            text.set(event_target_value(&ev));
                        }
                        on:keydown=move |ev: web_sys::KeyboardEvent| {
                            if ev.is_composing() {
                                return;
                            }
                            let recalled = match ev.key().as_str() {
                                "Enter" => {
                                    ev.prevent_default();
                                    send_text();
                                    return;
                                }
                                "ArrowUp" => {
                                    let typed = text.get_untracked();
                                    history.try_update_value(|h| h.up(&typed)).flatten()
                                }
                                "ArrowDown" => history.try_update_value(History::down).flatten(),
                                _ => return,
                            };
                            ev.prevent_default();
                            if let Some(sentence) = recalled {
                                text.set(sentence);
                            }
                        }
                    />
                    <button
                        type="button"
                        class="assist__send"
                        disabled=move || busy.get()
                        on:click=move |_| send_text()
                    >
                        {move || t(AssistantText::Send)}
                    </button>
                </div>
                <label class="assist__simulate" title=move || t(AssistantText::SimulationHint)>
                    <input
                        type="checkbox"
                        prop:checked=move || simulate.get()
                        on:change=move |ev| simulate.set(event_target_checked(&ev))
                    />
                    <span>{move || t(AssistantText::Simulation)}</span>
                </label>
            </section>
            <aside class="assist__trace">
                <h2 class="assist__trace-title">{move || t(AssistantText::HowUnderstood)}</h2>
                {move || match shown.get() {
                    None => view! { <p class="assist__hint">{t(AssistantText::NoTrace)}</p> }
                        .into_any(),
                    Some(turn) => {
                        let rows = trace_rows(lang(), &turn.trace);
                        let id = turn.id.clone();
                        let wrong = turn.wrong;
                        view! {
                            <dl class="assist__rows">
                                {rows
                                    .into_iter()
                                    .map(|(label, value)| view! { <dt>{label}</dt><dd>{value}</dd> })
                                    .collect_view()}
                            </dl>
                            <button
                                type="button"
                                class="assist__wrong"
                                aria-pressed=if wrong { "true" } else { "false" }
                                disabled=wrong
                                on:click=move |_| mark(id.clone())
                            >
                                {if wrong {
                                    t(AssistantText::MarkedWrong)
                                } else {
                                    t(AssistantText::Wrong)
                                }}
                            </button>
                        }
                            .into_any()
                    }
                }}
            </aside>
        </div>
    }
}

/// The answer buttons of a pending question: its choices, or Yes / No.
fn chips(q: Question, lang: Lang, answer: impl Fn(PendingReply) + Copy + 'static) -> impl IntoView {
    let pending = q.pending.clone();
    let buttons: Vec<(String, PendingReply)> = match q.kind {
        QuestionKind::Choice => q
            .choices
            .iter()
            .enumerate()
            .map(|(choice, label)| {
                (
                    label.clone(),
                    PendingReply::Choice {
                        pending: pending.clone(),
                        choice,
                    },
                )
            })
            .collect(),
        QuestionKind::Confirm => vec![
            (
                assistant_text(lang, AssistantText::Yes).to_owned(),
                PendingReply::Yes {
                    pending: pending.clone(),
                },
            ),
            (
                assistant_text(lang, AssistantText::No).to_owned(),
                PendingReply::No { pending },
            ),
        ],
    };
    view! {
        <div class="assist__chips">
            {buttons
                .into_iter()
                .map(|(label, reply)| {
                    view! {
                        <button
                            type="button"
                            class="assist__chip"
                            on:click=move |_| answer(reply.clone())
                        >
                            {label}
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_assist::view::ActionTrace;

    #[test]
    fn chips_live_as_long_as_the_hub_keeps_the_question() {
        let window = PENDING_TTL_S as f64 * 1000.0;
        // Just asked: the whole window, whatever the clocks say.
        assert_eq!(chip_window_ms(1_000, 1_000_000.0), window);
        assert_eq!(chip_window_ms(1_000, 990_000.0), window);
        assert_eq!(chip_window_ms(1_000, 1_030_000.0), window - 30_000.0);
        // Expired, long ago or just now.
        assert_eq!(chip_window_ms(1_000, 1_060_000.0), 0.0);
        assert_eq!(chip_window_ms(1_000, 9_000_000.0), 0.0);
    }

    fn h(sent: &[&str]) -> History {
        History::new(sent.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn up_walks_back_and_down_gives_back_the_typed_sentence() {
        let mut h = h(&["allume le salon", "éteins le salon"]);
        assert_eq!(h.up("mets le"), Some("éteins le salon".into()));
        assert_eq!(h.up("éteins le salon"), Some("allume le salon".into()));
        // The oldest: up stays put.
        assert_eq!(h.up("allume le salon"), None);
        assert_eq!(h.down(), Some("éteins le salon".into()));
        assert_eq!(h.down(), Some("mets le".into()));
        // Not browsing any more.
        assert_eq!(h.down(), None);
    }

    #[test]
    fn nothing_sent_nothing_to_recall() {
        let mut h = h(&[]);
        assert_eq!(h.up(""), None);
        assert_eq!(h.down(), None);
    }

    #[test]
    fn an_edit_stops_browsing() {
        let mut h = h(&["a", "b"]);
        assert_eq!(h.up(""), Some("b".into()));
        h.edited();
        assert_eq!(h.down(), None);
        assert_eq!(h.up("b!"), Some("b".into()));
        assert_eq!(h.down(), Some("b!".into()));
    }

    #[test]
    fn sending_keeps_the_sentence_once_and_caps_the_list() {
        let mut h = h(&["a"]);
        h.push("a".into());
        h.push("b".into());
        assert_eq!(h.sent(), ["a", "b"]);
        for i in 0..HISTORY_MAX + 5 {
            h.push(format!("s{i}"));
        }
        assert_eq!(h.sent().len(), HISTORY_MAX);
        assert_eq!(h.sent().last().map(String::as_str), Some("s54"));
    }

    fn turn(id: &str, at: u64) -> AssistTurn {
        AssistTurn {
            id: id.into(),
            at,
            client: "c".into(),
            text: "allume le salon".into(),
            simulate: false,
            outcome: nidoo_assist::view::OutcomeKind::Acted,
            reply: "C'est fait.".into(),
            question: None,
            trace: Trace::default(),
            wrong: false,
        }
    }

    fn ids(list: &[AssistTurn]) -> Vec<&str> {
        list.iter().map(|t| t.id.as_str()).collect()
    }

    #[test]
    fn a_clear_hides_the_turns_up_to_the_mark() {
        let list = vec![turn("a", 10), turn("b", 20), turn("c", 20), turn("d", 30)];
        assert_eq!(ids(&after_clear(list.clone(), None)), ["a", "b", "c", "d"]);
        // The marked turn is in the list: only what came after it, even within the same second.
        let mark = ClearMark {
            id: "b".into(),
            at: 20,
        };
        assert_eq!(ids(&after_clear(list.clone(), Some(&mark))), ["c", "d"]);
        // The marked turn has aged out of the list or the hub forgot it: go by its time.
        let mark = ClearMark {
            id: "gone".into(),
            at: 20,
        };
        assert_eq!(ids(&after_clear(list.clone(), Some(&mark))), ["d"]);
        let mark = ClearMark {
            id: "d".into(),
            at: 30,
        };
        assert!(after_clear(list, Some(&mark)).is_empty());
    }

    #[test]
    fn the_mark_is_the_newest_turn() {
        assert_eq!(ClearMark::of(&[]), None);
        assert_eq!(
            ClearMark::of(&[turn("a", 10), turn("b", 20)]),
            Some(ClearMark {
                id: "b".into(),
                at: 20
            })
        );
    }

    #[test]
    fn trace_rows_leave_out_empty_fields() {
        let trace = Trace {
            words: vec!["mets".into(), "le".into(), "salon".into()],
            room: Some("salon".into()),
            room_how: Some("mots : salon".into()),
            value: Some("19 − 0,5 = 18,5 °C".into()),
            actions: vec![ActionTrace {
                target: "salon".into(),
                what: "consigne 18,5 °C".into(),
                result: "ok".into(),
            }],
            elapsed_us: 80,
            ..Trace::default()
        };
        let rows = trace_rows(Lang::En, &trace);
        let labels: Vec<&str> = rows.iter().map(|(l, _)| *l).collect();
        assert_eq!(labels, ["Words", "Room", "Value", "Actions", "Time"]);
        assert_eq!(rows[1].1, "salon (mots : salon)");
        assert_eq!(rows[3].1, "salon → consigne 18,5 °C : ok");
    }

    fn volume(device: &str, name: &str, area: &str) -> nidoo_api::EntityView {
        use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
        nidoo_api::EntityView {
            id: format!("{device}_volume"),
            device_id: Some(device.into()),
            name: "Volume".into(),
            device_name: Some(name.into()),
            label: None,
            kind: Kind::Number,
            component: Kind::Number.component().to_string(),
            protocol_kind: Kind::Number,
            device_class: None,
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: area.into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride::default(),
            state: EntityState::default(),
        }
    }

    fn areas() -> Vec<nidoo_api::AreaView> {
        let kitchen = serde_json::json!({
            "id": "kitchen", "name": "Kitchen", "icon": null, "order": null,
            "entity_count": 1, "primary_count": 1
        });
        vec![serde_json::from_value(kitchen).unwrap()]
    }

    #[test]
    fn turn_source_names_the_satellite_and_its_room() {
        let entities: std::collections::BTreeMap<_, _> = [
            volume("nidoo_sound_aaaaaaaaaaaa", "Desk speaker", "kitchen"),
            volume(
                "nidoo_sound_bbbbbbbbbbbb",
                "Loose speaker",
                nidoo_core::model::UNASSIGNED_AREA,
            ),
        ]
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect();
        let areas = areas();
        assert_eq!(
            turn_source("satellite:aaaaaaaaaaaa", &entities, &areas).as_deref(),
            Some("Desk speaker · Kitchen")
        );
        assert_eq!(
            turn_source("satellite:bbbbbbbbbbbb", &entities, &areas).as_deref(),
            Some("Loose speaker")
        );
        assert_eq!(
            turn_source("satellite:cccccccccccc", &entities, &areas).as_deref(),
            Some("cccccccccccc"),
            "unknown satellite"
        );
        assert_eq!(turn_source("k3j2h1", &entities, &areas), None, "a browser");
    }
}
