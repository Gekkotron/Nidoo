//! `/assistant`: type a French sentence, read the hub's reply, answer its question with a tap,
//! and see how the sentence was understood. The replies are French and written by the hub;
//! the page chrome follows the language setting.

use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_assist::pending::PENDING_TTL_S;
use nidoo_assist::view::{AssistRequest, AssistTurn, PendingReply, Question, QuestionKind, Trace};

use crate::api::assist::{get_turns, mark_wrong, post_assist};
use crate::components::toast::Toasts;
use crate::i18n::{AssistantText, Lang, assistant_text, use_lang};

/// The id this browser gives the hub, so its pending question is its own.
const CLIENT_KEY: &str = "nidoo.assist.client";
/// Whether the Simulation switch is on, per browser.
const SIMULATE_KEY: &str = "nidoo.assist.simulate";
/// How many past turns the page shows when it opens.
const LIMIT: u32 = 50;

fn storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
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

    spawn_local(async move {
        if let Ok(list) = get_turns(LIMIT).await {
            let mut list_turns = list.turns;
            list_turns.reverse();
            let _ = turns.try_set(list_turns);
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
        text.set(String::new());
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
                <p class="assist__hint">{move || t(AssistantText::Hint)}</p>
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
                                    <p class="assist__said">{turn.text.clone()}</p>
                                    <p class="assist__reply">{turn.reply.clone()}</p>
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
                        on:input=move |ev| text.set(event_target_value(&ev))
                        on:keydown=move |ev: web_sys::KeyboardEvent| {
                            if ev.key() == "Enter" && !ev.is_composing() {
                                ev.prevent_default();
                                send_text();
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
}
