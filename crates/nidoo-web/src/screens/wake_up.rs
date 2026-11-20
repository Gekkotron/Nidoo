//! The Wake-up page: the list of music alarms and the editor of one. Each alarm is a managed
//! flow (`wake-up-<n>`, label `wake-up`) rewritten from its settings on every save.

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::NavigateOptions;
use leptos_router::components::A;
use leptos_router::hooks::{use_navigate, use_params_map};
use nidoo_api::{FlowView, LabelEdit};
use nidoo_core::flow::wake_up::{FADE_CHOICES, LABEL, next_id, read, to_flow, validate};
use nidoo_core::model::{MediaPlayerState, StateValue};

use crate::api;
use crate::components::toast::Toasts;
use crate::house::House;
use crate::i18n::wake_up::{WakeText, next_alarm, percent, unreadable_body, wake_text};
use crate::i18n::{Lang, use_lang};
use crate::nav::BackLink;
use crate::playlist_fields::{fade_label, output_options, playlist_options};
use crate::schedule::week_chips;
use crate::screens::flow_schedule::tick_text;
use crate::screens::sound_settings::satellites;
use crate::screens::wake_up_logic::{Draft, NIDOO_SOURCE, summary, time_text};
use crate::screens::wake_up_settings::pinson_player;

/// How long the editor waits after the last change before asking the hub for the next ring.
const PREVIEW_PAUSE_MS: u32 = 300;

/// The state the Pinson media player reports, empty while none is connected.
fn player_state(house: House) -> Memo<MediaPlayerState> {
    Memo::new(move |_| {
        house.entities.with(|e| match pinson_player(e) {
            Some(p) => match p.state.value {
                StateValue::MediaPlayer(s) => s,
                _ => MediaPlayerState::default(),
            },
            None => MediaPlayerState::default(),
        })
    })
}

/// A flow changed on the hub: replace it in the house list, or add it.
fn store_flow(house: House, flow: FlowView) {
    house.flows.update(|list| {
        match list
            .iter_mut()
            .find(|f| f.definition.id == flow.definition.id)
        {
            Some(slot) => *slot = flow,
            None => list.push(flow),
        }
    });
}

/// What stands in for the page when it cannot work.
#[component]
fn Empty(
    title: WakeText,
    body: WakeText,
    #[prop(into, optional)] link: Option<(&'static str, WakeText)>,
) -> impl IntoView {
    let lang = use_lang();
    view! {
        <div class="wake-empty">
            <span class="wake-empty__glyph" inner_html=crate::icons::ALARM_CLOCK></span>
            <div class="wake-empty__title">{move || wake_text(lang(), title)}</div>
            <p class="wake-empty__body">{move || wake_text(lang(), body)}</p>
            {link
                .map(|(href, label)| {
                    view! {
                        <A href=href attr:class="btn btn--primary wake-empty__link">
                            {move || wake_text(lang(), label)}
                        </A>
                    }
                })}
        </div>
    }
}

/// Shows `children` while Wake-up is on and a Pinson player is connected, else says why not.
#[component]
fn Gate(children: ChildrenFn) -> impl IntoView {
    let house = expect_context::<House>();
    let on = Memo::new(move |_| house.home_settings.with(|s| s.wake_up));
    let has_player = Memo::new(move |_| house.entities.with(|e| pinson_player(e).is_some()));
    move || {
        if !on.get() {
            view! {
                <Empty
                    title=WakeText::Disabled
                    body=WakeText::DisabledBody
                    link=("/settings/wake-up", WakeText::OpenSettings)
                />
            }
            .into_any()
        } else if !has_player.get() {
            view! { <Empty title=WakeText::NoPinson body=WakeText::NoPinsonBody /> }.into_any()
        } else {
            children().into_any()
        }
    }
}

/// The ids of the flows the page manages, soonest alarm first, the unreadable ones last.
fn alarm_ids(flows: &[FlowView]) -> Vec<String> {
    let mut rows: Vec<(Option<(u8, u8)>, String)> = flows
        .iter()
        .filter_map(|f| {
            let at = match read(&f.definition)? {
                Ok(w) => Some((w.hour, w.minute)),
                Err(_) => None,
            };
            Some((at, f.definition.id.clone()))
        })
        .collect();
    // `None` (unreadable) sorts before `Some`, so flip the key to put it last.
    rows.sort_by(|a, b| match (a.0, b.0) {
        (Some(x), Some(y)) => x.cmp(&y).then_with(|| a.1.cmp(&b.1)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.1.cmp(&b.1),
    });
    rows.into_iter().map(|(_, id)| id).collect()
}

/// The list: one row per alarm, and the button that adds one.
#[component]
pub fn WakeUpScreen() -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let ids = Memo::new(move |_| house.flows.with(|f| alarm_ids(f)));
    view! {
        <section class="wake-page">
            <Gate>
                <p class="wake-page__lede">{move || wake_text(lang(), WakeText::PageLede)}</p>
                <div class="wake-alarms">
                    <For each=move || ids.get() key=|id| id.clone() let:id>
                        <AlarmRow id=id />
                    </For>
                    <div>
                        <A href="/wake-up/new" attr:class="chip chip--add wake-add">
                            <span class="wake-add__glyph" inner_html=crate::icons::PLUS></span>
                            {move || wake_text(lang(), WakeText::AddAlarm)}
                        </A>
                    </div>
                </div>
            </Gate>
        </section>
    }
}

/// Deletes alarm `id` and drops it from the house list.
fn delete_alarm(house: House, toasts: Option<Toasts>, id: String, then: Option<Callback<()>>) {
    spawn_local(async move {
        match api::delete_flow(&id).await {
            Ok(()) => {
                house
                    .flows
                    .try_update(|list| list.retain(|f| f.definition.id != id));
                if let Some(then) = then {
                    then.run(());
                }
            }
            Err(e) => {
                if let Some(toasts) = toasts {
                    toasts.push(e.error);
                }
            }
        }
    });
}

/// One alarm of the list: its summary and switch, or the unreadable stand-in with Delete.
#[component]
fn AlarmRow(id: String) -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let toasts = use_context::<Toasts>();
    let flow_id = id.clone();
    let alarm = Memo::new(move |_| {
        house.flows.with(|list| {
            list.iter().find(|f| f.definition.id == flow_id).map(|f| {
                (
                    f.definition.name.clone(),
                    f.definition.enabled,
                    read(&f.definition),
                )
            })
        })
    });
    let state = player_state(house);
    let sats = Memo::new(move |_| house.entities.with(satellites));
    let next = RwSignal::new(None::<String>);
    // The next ring is asked again only when the schedule itself changes.
    let cron = Memo::new(move |_| {
        alarm.with(|a| match a {
            Some((_, _, Some(Ok(w)))) => Some(nidoo_core::flow::wake_up::cron(w)),
            _ => None,
        })
    });
    Effect::new(move |_| {
        let Some(cron) = cron.get() else { return };
        spawn_local(async move {
            let first = api::schedule_preview(&cron)
                .await
                .ok()
                .and_then(|p| p.next.into_iter().next());
            next.try_set(first);
        });
    });
    let toggle_id = id.clone();
    let toggle = move |_| {
        let Some((_, enabled, _)) = alarm.get_untracked() else {
            return;
        };
        let id = toggle_id.clone();
        spawn_local(async move {
            match api::set_flow_enabled(&id, !enabled).await {
                Ok(flow) => store_flow(house, flow),
                Err(e) => {
                    if let Some(toasts) = toasts {
                        toasts.push(e.error);
                    }
                }
            }
        });
    };
    let delete_id = id.clone();
    let href = format!("/wake-up/{}", api::encode_id(&id));
    move || {
        let (name, enabled, settings) = alarm.get()?;
        Some(match settings {
            Some(Ok(w)) => {
                let s = state.with(|st| sats.with(|sats| summary(lang(), &w, st, sats)));
                let days = s
                    .days
                    .into_iter()
                    .map(|(text, on)| {
                        view! { <span class:on=on>{text}</span> }
                    })
                    .collect_view();
                let label = name.clone();
                let mut meta = vec![s.playlist];
                meta.extend(s.speakers);
                meta.push(s.fade);
                let toggle = toggle.clone();
                view! {
                    <div class="wake-alarm" class:wake-alarm--off=!enabled>
                        <A href=href.clone() attr:class="wake-alarm__body">
                            <span class="wake-alarm__time">{s.time}</span>
                            <span class="wake-alarm__text">
                                <span class="wake-alarm__name">{name}</span>
                                <span class="wake-alarm__meta">
                                    <span class="wake-days-mini">{days}</span>
                                    {meta
                                        .into_iter()
                                        .map(|m| view! { <span>{m}</span> })
                                        .collect_view()}
                                </span>
                                <span class="wake-alarm__next">
                                    {move || {
                                        if enabled {
                                            next.get()
                                                .map(|t| next_alarm(lang(), &tick_text(lang(), &t)))
                                                .unwrap_or_default()
                                        } else {
                                            wake_text(lang(), WakeText::OffLabel).to_owned()
                                        }
                                    }}
                                </span>
                            </span>
                        </A>
                        <button
                            type="button"
                            role="switch"
                            class="switch wake-alarm__switch"
                            class:switch--on=enabled
                            aria-checked=enabled.to_string()
                            aria-label=label
                            on:click=toggle
                        >
                            <span class="switch__knob"></span>
                        </button>
                    </div>
                }
                .into_any()
            }
            // The settings do not parse (edited by hand): it can only be deleted.
            _ => {
                let delete_id = delete_id.clone();
                view! {
                    <div class="wake-alarm wake-alarm--bad">
                        <span
                            class="wake-alarm__time wake-alarm__time--bad"
                            inner_html=crate::icons::ALARM_CLOCK
                        ></span>
                        <span class="wake-alarm__text">
                            <span class="wake-alarm__name">
                                {move || wake_text(lang(), WakeText::Unreadable)}
                            </span>
                            <span class="wake-alarm__meta">
                                {move || unreadable_body(lang(), &delete_id)}
                            </span>
                        </span>
                        <button
                            type="button"
                            class="btn btn--danger wake-alarm__delete"
                            on:click={
                                let id = id.clone();
                                move |_| delete_alarm(house, toasts, id.clone(), None)
                            }
                        >
                            {move || wake_text(lang(), WakeText::Delete)}
                        </button>
                    </div>
                }
                .into_any()
            }
        })
    }
}

/// A flow is saved with the `wake-up` label, which is created (and named) on the first save.
async fn ensure_label(house: House, lang: Lang) -> Result<(), String> {
    let view = api::flow_labels::get_flow_labels()
        .await
        .map_err(|e| e.error)?;
    if view.catalog.labels.iter().any(|l| l.id == LABEL) {
        house.set_flow_labels(view);
        return Ok(());
    }
    api::flow_labels::edit_flow_labels(&LabelEdit::CreateLabel {
        name: LABEL.to_owned(),
        choice: None,
    })
    .await
    .map_err(|e| e.error)?;
    let named = api::flow_labels::edit_flow_labels(&LabelEdit::RenameLabel {
        id: LABEL.to_owned(),
        name: wake_text(lang, WakeText::PageTitle).to_owned(),
    })
    .await
    .map_err(|e| e.error)?;
    house.set_flow_labels(named);
    Ok(())
}

/// What the editor opens on: a new alarm, or the saved one with its name and switch.
#[derive(Clone)]
enum Opening {
    New,
    Saved {
        name: String,
        enabled: bool,
        settings: nidoo_core::flow::wake_up::WakeUp,
    },
}

/// The editor at `/wake-up/:id` (`new` for a new alarm).
#[component]
pub fn WakeUpEditorScreen() -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let params = use_params_map();
    let id = Memo::new(move |_| params.with(|p| p.get("id").unwrap_or_default()));
    // The alarm exists once its flow is in the house list; the editor is built then and kept,
    // so a push about another flow does not reset what is being typed.
    let found = Memo::new(move |_| {
        let id = id.get();
        if id == "new" {
            return Some(true);
        }
        house.flows.with(|list| {
            list.iter()
                .find(|f| f.definition.id == id)
                .map(|f| matches!(read(&f.definition), Some(Ok(_))))
        })
    });
    view! {
        <section class="wake-page">
            <BackLink />
            <Gate>
                {move || match found.get() {
                    Some(true) => {
                        let opening = if id.get_untracked() == "new" {
                            Opening::New
                        } else {
                            house
                                .flows
                                .with_untracked(|list| {
                                    list.iter()
                                        .find(|f| f.definition.id == id.get_untracked())
                                        .and_then(|f| match read(&f.definition) {
                                            Some(Ok(settings)) => {
                                                Some(Opening::Saved {
                                                    name: f.definition.name.clone(),
                                                    enabled: f.definition.enabled,
                                                    settings,
                                                })
                                            }
                                            _ => None,
                                        })
                                })
                                .unwrap_or(Opening::New)
                        };
                        view! { <Editor id=id.get_untracked() opening=opening /> }.into_any()
                    }
                    _ => {
                        view! {
                            <p class="wake-page__lede">{move || wake_text(lang(), WakeText::NotFound)}</p>
                        }
                            .into_any()
                    }
                }}
            </Gate>
        </section>
    }
}

/// The form of one alarm.
#[component]
fn Editor(id: String, opening: Opening) -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let navigate = use_navigate();
    let toasts = use_context::<Toasts>();
    let state = player_state(house);
    let is_new = id == "new";
    let draft = RwSignal::new(match &opening {
        Opening::New => state
            .with_untracked(|s| Draft::new_alarm(s, &house.entities.with_untracked(satellites))),
        Opening::Saved {
            name,
            enabled,
            settings,
        } => Draft::from_saved(
            name,
            *enabled,
            settings,
            &house.entities.with_untracked(satellites),
        ),
    });
    let saving = RwSignal::new(false);
    let no_day = Memo::new(move |_| draft.with(|d| d.days.is_empty()));

    // The next ring, asked again after a short pause; a reply for a stale schedule is dropped.
    let cron = Memo::new(move |_| {
        draft.with(|d| {
            (!d.days.is_empty()).then(|| nidoo_core::flow::wake_up::cron(&d.to_wake_up("")))
        })
    });
    let next = RwSignal::new(None::<String>);
    let generation = RwSignal::new(0_u32);
    Effect::new(move |_| {
        let Some(cron) = cron.get() else {
            next.set(None);
            return;
        };
        let ticket = generation.get_untracked() + 1;
        generation.set(ticket);
        spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(PREVIEW_PAUSE_MS).await;
            if generation.try_get_untracked() != Some(ticket) {
                return;
            }
            let first = api::schedule_preview(&cron)
                .await
                .ok()
                .and_then(|p| p.next.into_iter().next());
            if generation.try_get_untracked() == Some(ticket) {
                next.try_set(first);
            }
        });
    });

    let save_id = id.clone();
    let save_nav = navigate.clone();
    let save = move |_| {
        let Some(player) = house.entities.with_untracked(pinson_player) else {
            return;
        };
        let l = lang();
        let (name, enabled, settings) =
            draft.with_untracked(|d| (d.display_name(l), d.enabled, d.to_wake_up(&player.id)));
        if let Err(e) = validate(&settings) {
            if let Some(toasts) = toasts {
                toasts.push(e.to_string());
            }
            return;
        }
        let flow_id = if is_new {
            house.flows.with_untracked(|list| {
                let taken: Vec<&str> = list.iter().map(|f| f.definition.id.as_str()).collect();
                next_id(&taken)
            })
        } else {
            save_id.clone()
        };
        let navigate = save_nav.clone();
        saving.set(true);
        spawn_local(async move {
            let result = match ensure_label(house, l).await {
                Ok(()) => api::put_flow(&to_flow(&flow_id, &name, enabled, &settings))
                    .await
                    .map_err(|e| e.error),
                Err(e) => Err(e),
            };
            match result {
                Ok((flow, warning)) => {
                    store_flow(house, flow);
                    if let (Some(warning), Some(toasts)) = (warning, toasts) {
                        toasts.push(warning);
                    }
                    navigate("/wake-up", NavigateOptions::default());
                }
                Err(e) => {
                    saving.try_set(false);
                    if let Some(toasts) = toasts {
                        toasts.push(e);
                    }
                }
            }
        });
    };

    let delete_nav = navigate.clone();
    let delete = {
        let id = id.clone();
        move |_| {
            let navigate = delete_nav.clone();
            let then = Callback::new(move |()| navigate("/wake-up", NavigateOptions::default()));
            delete_alarm(house, toasts, id.clone(), Some(then));
        }
    };
    let cancel_nav = navigate;

    let chips = move || {
        week_chips(lang())
            .into_iter()
            .map(|(day, label)| {
                let on = move || draft.with(|d| d.days.contains(&day));
                view! {
                    <button
                        type="button"
                        class="chip wake-day"
                        class:chip--selected=on
                        aria-pressed=move || on().to_string()
                        on:click=move |_| draft.update(|d| d.toggle_day(day))
                    >
                        {label}
                    </button>
                }
            })
            .collect_view()
    };

    let number_input = move |label: WakeText, max: u8, minute: bool| {
        let current = move || draft.with(|d| if minute { d.minute } else { d.hour });
        view! {
            <input
                type="text"
                inputmode="numeric"
                maxlength="2"
                class=if minute { "wake-time__input wake-minute" } else { "wake-time__input wake-hour" }
                aria-label=move || wake_text(lang(), label)
                prop:value=format!("{:02}", current())
                on:input=move |ev| {
                    if let Ok(v) = event_target_value(&ev).trim().parse::<u8>() {
                        let v = v.min(max);
                        draft
                            .update(|d| {
                                if minute {
                                    d.minute = v;
                                } else {
                                    d.hour = v;
                                }
                            });
                    }
                }
                on:change=move |ev| {
                    event_target::<web_sys::HtmlInputElement>(&ev)
                        .set_value(&format!("{:02}", current()));
                }
            />
        }
    };

    let playlists = Memo::new(move |_| {
        let saved = draft.with(|d| d.playlist.clone());
        state.with(|s| playlist_options(lang(), s, &saved))
    });
    let outputs = Memo::new(move |_| {
        let saved = draft.with(|d| d.source.clone());
        state.with(|s| output_options(lang(), s, saved.as_deref()))
    });
    let fades = Memo::new(move |_| {
        let current = draft.with(|d| d.fade_in_s);
        let mut list = FADE_CHOICES.to_vec();
        if !list.contains(&current) {
            list.push(current);
        }
        list
    });
    let on_nidoo = Memo::new(move |_| draft.with(|d| d.source.as_deref() == Some(NIDOO_SOURCE)));
    let speaker_rows = move || {
        (0..draft.with_untracked(|d| d.speakers.len()))
            .map(|i| {
                let on = move || draft.with(|d| d.speakers[i].on);
                let volume = move || draft.with(|d| d.speakers[i].volume);
                let name = draft.with_untracked(|d| d.speakers[i].name.clone());
                view! {
                    <div class="wake-speaker" class:wake-speaker--off=move || !on()>
                        <input
                            type="checkbox"
                            class="wake-speaker__check"
                            aria-label=name.clone()
                            prop:checked=on
                            on:change=move |ev| {
                                let checked = event_target_checked(&ev);
                                draft.update(|d| d.speakers[i].on = checked);
                            }
                        />
                        <span class="wake-speaker__name">{name}</span>
                        {move || {
                            if on() {
                                view! {
                                    <input
                                        type="range"
                                        class="wake-speaker__range"
                                        min="0"
                                        max="100"
                                        aria-label=move || wake_text(lang(), WakeText::Volume)
                                        prop:value=volume().to_string()
                                        on:input=move |ev| {
                                            if let Ok(v) = event_target_value(&ev).parse::<f64>() {
                                                draft.update(|d| d.speakers[i].volume = v);
                                            }
                                        }
                                    />
                                    <span class="wake-speaker__volume">
                                        {move || percent(lang(), volume())}
                                    </span>
                                }
                                    .into_any()
                            } else {
                                view! {
                                    <span class="wake-speaker__muted">
                                        {move || wake_text(lang(), WakeText::MutedNote)}
                                    </span>
                                }
                                    .into_any()
                            }
                        }}
                    </div>
                }
            })
            .collect_view()
    };

    view! {
        <div class="wake-editor">
            <div class="wake-editor__head">
                <input
                    type="text"
                    class="wake-editor__name"
                    aria-label=move || wake_text(lang(), WakeText::Name)
                    prop:value=move || draft.with(|d| d.display_name(lang()))
                    on:input=move |ev| {
                        let v = event_target_value(&ev);
                        draft.update(|d| d.name = Some(v));
                    }
                />
                <span class="wake-editor__state">
                    {move || {
                        wake_text(
                            lang(),
                            if draft.with(|d| d.enabled) { WakeText::OnLabel } else { WakeText::OffLabel },
                        )
                    }}
                </span>
                <button
                    type="button"
                    role="switch"
                    class="switch wake-editor__switch"
                    class:switch--on=move || draft.with(|d| d.enabled)
                    aria-checked=move || draft.with(|d| d.enabled).to_string()
                    aria-label=move || wake_text(lang(), WakeText::AlarmOn)
                    on:click=move |_| draft.update(|d| d.enabled = !d.enabled)
                >
                    <span class="switch__knob"></span>
                </button>
            </div>
            <div class="wake-time">
                {number_input(WakeText::Hour, 23, false)} <span class="wake-time__colon">":"</span>
                {number_input(WakeText::Minute, 59, true)}
            </div>
            <div class="wake-days">
                <div class="wake-days__chips">{chips}</div>
                <Show when=move || no_day.get()>
                    <div class="wake-days__error">{move || wake_text(lang(), WakeText::PickDay)}</div>
                </Show>
            </div>
            <div class="wake-row">
                <div class="wake-row__label">{move || wake_text(lang(), WakeText::Music)}</div>
                <div class="wake-row__body">
                    <select
                        class="wake-select wake-playlist"
                        aria-label=move || wake_text(lang(), WakeText::Playlist)
                        on:change=move |ev| {
                            let v = event_target_value(&ev);
                            draft.update(|d| d.playlist = v);
                        }
                    >
                        {move || {
                            let current = draft.with(|d| d.playlist.clone());
                            playlists
                                .get()
                                .into_iter()
                                .map(|o| {
                                    let selected = o.id == current;
                                    view! { <option value=o.id selected=selected>{o.label}</option> }
                                })
                                .collect_view()
                        }}
                    </select>
                    <span>{move || wake_text(lang(), WakeText::Shuffle)}</span>
                    <button
                        type="button"
                        role="switch"
                        class="switch wake-shuffle"
                        class:switch--on=move || draft.with(|d| d.shuffle)
                        aria-checked=move || draft.with(|d| d.shuffle).to_string()
                        aria-label=move || wake_text(lang(), WakeText::Shuffle)
                        on:click=move |_| draft.update(|d| d.shuffle = !d.shuffle)
                    >
                        <span class="switch__knob"></span>
                    </button>
                </div>
            </div>
            <div class="wake-row">
                <div class="wake-row__label">{move || wake_text(lang(), WakeText::Output)}</div>
                <div class="wake-row__body">
                    <select
                        class="wake-select wake-output"
                        aria-label=move || wake_text(lang(), WakeText::Output)
                        on:change=move |ev| {
                            let v = event_target_value(&ev);
                            draft.update(|d| d.source = (!v.is_empty()).then_some(v));
                        }
                    >
                        {move || {
                            let current = draft.with(|d| d.source.clone().unwrap_or_default());
                            outputs
                                .get()
                                .into_iter()
                                .map(|o| {
                                    let selected = o.id == current;
                                    view! { <option value=o.id selected=selected>{o.label}</option> }
                                })
                                .collect_view()
                        }}
                    </select>
                </div>
            </div>
            <Show when=move || on_nidoo.get()>
                <div class="wake-row">
                    <div class="wake-row__label">{move || wake_text(lang(), WakeText::Speakers)}</div>
                    <div class="wake-speakers">{speaker_rows()}</div>
                </div>
            </Show>
            <div class="wake-row">
                <div class="wake-row__label">{move || wake_text(lang(), WakeText::Rise)}</div>
                <div class="wake-row__body">
                    {move || {
                        let current = draft.with(|d| d.fade_in_s);
                        fades
                            .get()
                            .into_iter()
                            .map(|secs| {
                                view! {
                                    <button
                                        type="button"
                                        class="chip wake-fade"
                                        class:chip--selected=secs == current
                                        aria-pressed=(secs == current).to_string()
                                        on:click=move |_| draft.update(|d| d.fade_in_s = secs)
                                    >
                                        {fade_label(lang(), secs)}
                                    </button>
                                }
                            })
                            .collect_view()
                    }} <span class="wake-hint">{move || wake_text(lang(), WakeText::RiseHint)}</span>
                </div>
            </div>
            <div class="wake-next">
                <span class="wake-next__glyph" inner_html=crate::icons::ALARM_CLOCK></span>
                <span class="wake-next__text">
                    {move || {
                        if no_day.get() {
                            wake_text(lang(), WakeText::NoDayNext).to_owned()
                        } else {
                            next.get()
                                .map(|t| next_alarm(lang(), &tick_text(lang(), &t)))
                                .unwrap_or_else(|| {
                                    draft.with(|d| time_text(d.hour, d.minute))
                                })
                        }
                    }}
                </span>
            </div>
            <div class="wake-actions">
                {(!is_new)
                    .then(|| {
                        view! {
                            <button
                                type="button"
                                class="btn btn--danger wake-delete"
                                on:click=delete.clone()
                            >
                                {move || wake_text(lang(), WakeText::Delete)}
                            </button>
                        }
                    })} <span class="wake-actions__spacer"></span>
                <button
                    type="button"
                    class="btn wake-cancel"
                    on:click=move |_| cancel_nav("/wake-up", NavigateOptions::default())
                >
                    {move || wake_text(lang(), WakeText::Cancel)}
                </button>
                <button
                    type="button"
                    class="btn btn--primary wake-save"
                    disabled=move || no_day.get() || saving.get()
                    on:click=save
                >
                    {move || wake_text(lang(), WakeText::Save)}
                </button>
            </div>
        </div>
    }
}
