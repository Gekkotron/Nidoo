//! The floating bars over the canvas: top-left the back link, the flow's name and its mode
//! (a text field and a single/restart select while editing);
//! top-right the enable switch, Run, Edit and Runs (Tidy, Undo, Cancel and Save while editing).

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::NavigateOptions;
use leptos_router::components::A;
use leptos_router::hooks::use_navigate;
use nidoo_api::FlowView;
use nidoo_core::flow::{Flow, FlowMode};
use nidoo_core::model::UNASSIGNED_AREA;

use super::{Editor, Panel, Selection};
use crate::api;
use crate::components::flow_switch::FlowSwitch;
use crate::components::toast::Toasts;
use crate::flow_edit::{flow_uuid, stamp_notify_lang};
use crate::flow_layout::{tidy, with_resolved_positions};
use crate::flow_list::clear_stale_area;
use crate::house::House;
use crate::i18n::flows::{FlowText, save_failed, use_flow_text};
use crate::i18n::use_lang;
use crate::icons::{CHECK, PENCIL, PLAY, X_MARK};
use crate::nav::use_back;
use crate::screens::flow_schedule::SaveBlocked;

/// 16 bytes from the browser's `crypto.getRandomValues`, which (unlike `randomUUID`) also
/// works on a plain-HTTP LAN address.
fn random_bytes() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    web_sys::window()
        .and_then(|w| w.crypto().ok())
        .and_then(|c| c.get_random_values_with_u8_array(&mut bytes).ok())
        .expect("crypto.getRandomValues is available in every supported browser");
    bytes
}

#[component]
fn EditControls(
    editing: RwSignal<bool>,
    editor: Editor,
    saving: RwSignal<bool>,
    original: Signal<Flow>,
) -> impl IntoView {
    let toasts = expect_context::<Toasts>();
    let lang = use_lang();
    let t = use_flow_text();
    let blocked = use_context::<SaveBlocked>();
    let is_blocked = move || blocked.is_some_and(|b| b.0.get());
    let house = expect_context::<House>();
    let navigate = use_navigate();
    let go_back = navigate.clone();
    // An empty id marks a flow that was never saved: there is nothing to go back to.
    let cancel = move |_| {
        if let Some(b) = blocked {
            b.0.set(false);
        }
        let original = original.get_untracked();
        if original.id.is_empty() {
            go_back("/flows", NavigateOptions::default());
            return;
        }
        editor.reset(original);
        editing.set(false);
    };
    let save = move |_| {
        if saving.get_untracked() || blocked.is_some_and(|b| b.0.get_untracked()) {
            return;
        }
        saving.set(true);
        let mut snapshot = editor.draft.get_untracked();
        house
            .areas
            .with_untracked(|areas| clear_stale_area(&mut snapshot, areas));
        stamp_notify_lang(&mut snapshot, lang().core());
        let is_new = snapshot.id.is_empty();
        if is_new {
            snapshot.id = flow_uuid(random_bytes());
        }
        let navigate = navigate.clone();
        spawn_local(async move {
            match api::put_flow(&snapshot).await {
                // Saved: back to the view, which the hub's echo updates. A new flow moves to
                // its own address, and the screen there finds it in the house.
                Ok((view, warning)) => {
                    if is_new {
                        let saved = view.clone();
                        house.flows.update(|flows| {
                            if !flows.iter().any(|f| f.definition.id == saved.definition.id) {
                                flows.push(saved);
                            }
                        });
                        navigate(
                            &format!("/flows/{}", api::encode_id(&view.definition.id)),
                            NavigateOptions {
                                replace: true,
                                ..Default::default()
                            },
                        );
                    } else {
                        editor.reset(view.definition);
                        editing.set(false);
                    }
                    if let Some(warning) = warning {
                        toasts.push(warning);
                    }
                    house.refresh_flows_config();
                }
                Err(e) => toasts.push(save_failed(untrack(lang), &e.error)),
            }
            saving.set(false);
        });
    };
    view! {
        <button
            type="button"
            class="btn btn--ghost"
            aria-label=move || t(FlowText::Tidy)
            title=move || t(FlowText::TidyTitle)
            on:click=move |_| editor.edit(|f| *f = tidy(f))
        >
            {move || t(FlowText::Tidy)}
        </button>
        <button
            type="button"
            class="btn btn--ghost btn--icon flow-toolbar__undo"
            aria-label=move || t(FlowText::Undo)
            title=move || t(FlowText::Undo)
            disabled=move || !editor.can_undo()
            on:click=move |_| {
                editor.step(false);
            }
        >
            "↶"
        </button>
        <button
            type="button"
            class="btn btn--ghost btn--icon"
            aria-label=move || t(FlowText::CancelEditing)
            title=move || t(FlowText::Cancel)
            disabled=move || saving.get()
            on:click=cancel
            inner_html=X_MARK
        ></button>
        <button
            type="button"
            class="btn btn--icon"
            aria-label=move || {
                if saving.get() { t(FlowText::Saving) } else { t(FlowText::Save) }
            }
            title=move || {
                if saving.get() { t(FlowText::SavingEllipsis) } else { t(FlowText::Save) }
            }
            disabled=move || saving.get() || is_blocked()
            on:click=save
            inner_html=CHECK
        ></button>
    }
}

#[component]
fn ReadControls(
    view: Signal<FlowView>,
    editing: RwSignal<bool>,
    editor: Editor,
    selection: RwSignal<Option<Selection>>,
) -> impl IntoView {
    let toasts = expect_context::<Toasts>();
    let t = use_flow_text();
    let invalid = Memo::new(move |_| view.with(|v| v.invalid.is_some()));
    let busy = Memo::new(move |_| view.with(|v| v.active_run.is_some()));
    // The switch takes plain values: it is rebuilt when one of them changes.
    let switch = Memo::new(move |_| {
        view.with(|v| {
            (
                v.definition.id.clone(),
                v.definition.name.clone(),
                v.definition.enabled,
                v.invalid.is_some(),
            )
        })
    });
    let run_now = move |_| {
        let id = view.with_untracked(|v| v.definition.id.clone());
        spawn_local(async move {
            if let Err(e) = api::run_flow(&id).await {
                toasts.push(e.error);
            }
        });
    };
    // The Edit button opens the library: a card ringed in view mode is not carried over.
    let enter_edit = move |_| {
        editor.reset(view.with_untracked(|v| with_resolved_positions(&v.definition)));
        selection.set(None);
        editing.set(true);
    };
    view! {
        {move || {
            let (id, name, enabled, invalid) = switch.get();
            view! { <FlowSwitch id=id name=name enabled=enabled invalid=invalid /> }
        }}
        <button
            type="button"
            class="btn btn--icon"
            aria-label=move || t(FlowText::RunNow)
            title=move || t(FlowText::RunNow)
            disabled=move || invalid.get() || busy.get()
            on:click=run_now
            inner_html=PLAY
        ></button>
        <button
            type="button"
            class="btn btn--ghost btn--icon flow-toolbar__edit"
            aria-label=move || t(FlowText::EditLayout)
            title=move || t(FlowText::EditLayout)
            disabled=move || invalid.get()
            on:click=enter_edit
            inner_html=PENCIL
        ></button>
    }
}

/// Both floating bars. `drawer` is the runs drawer the Runs button toggles.
#[component]
pub fn Toolbar(
    view: Signal<FlowView>,
    editing: RwSignal<bool>,
    editor: Editor,
    saving: RwSignal<bool>,
    runs_open: RwSignal<bool>,
    panel: Signal<Panel>,
    selection: RwSignal<Option<Selection>>,
) -> impl IntoView {
    let t = use_flow_text();
    let showing_runs = move || panel.get() == Panel::Runs;
    let name = move || view.with(|v| v.definition.name.clone());
    // The first keystroke after focusing the field records one undo step; the rest join it.
    let armed = StoredValue::new(false);
    let rename = move |ev: web_sys::Event| {
        let text = event_target_value(&ev);
        if armed.get_value() {
            armed.set_value(false);
            editor
                .history
                .update_value(|h| h.record(editor.draft.get_untracked()));
        }
        editor.draft.update(|f| f.name = text);
    };
    let mode = move || {
        t(view.with(|v| match v.definition.mode {
            FlowMode::Single => FlowText::ModeSingle,
            FlowMode::Restart => FlowText::ModeRestart,
        }))
    };
    let draft_mode = move || match editor.draft.with(|f| f.mode) {
        FlowMode::Single => "single",
        FlowMode::Restart => "restart",
    };
    let pick_mode = move |ev: web_sys::Event| {
        let mode = match event_target_value(&ev).as_str() {
            "restart" => FlowMode::Restart,
            _ => FlowMode::Single,
        };
        if mode == editor.draft.with_untracked(|f| f.mode) {
            return;
        }
        editor
            .history
            .update_value(|h| h.record(editor.draft.get_untracked()));
        editor.draft.update(|f| f.mode = mode);
    };
    let house = expect_context::<House>();
    // A room the house no longer has reads as "No room" (and is cleared on save).
    let draft_room = move || {
        let area = editor.draft.with(|f| f.area.clone());
        house.areas.with(|areas| {
            area.filter(|a| a != UNASSIGNED_AREA && areas.iter().any(|x| &x.id == a))
                .unwrap_or_default()
        })
    };
    let pick_room = move |ev: web_sys::Event| {
        let value = event_target_value(&ev);
        let area = (!value.is_empty()).then_some(value);
        if area == editor.draft.with_untracked(|f| f.area.clone()) {
            return;
        }
        editor
            .history
            .update_value(|h| h.record(editor.draft.get_untracked()));
        editor.draft.update(|f| f.area = area);
    };
    // The room's name in read mode, when the flow has a known one.
    let room_name = move || {
        view.with(|v| v.definition.area.clone()).and_then(|a| {
            house.areas.with(|areas| {
                areas
                    .iter()
                    .find(|x| x.id == a && x.id != UNASSIGNED_AREA)
                    .map(|x| x.name.clone())
            })
        })
    };
    let original = Signal::derive(move || view.with(|v| v.definition.clone()));
    let back = use_back();
    view! {
        <div class="flow-toolbar flow-toolbar--title">
            <A href=move || back.get() attr:class="flow-toolbar__back" attr:aria-label=move || t(FlowText::Back)>
                "←"
            </A>
            <Show
                when=move || editing.get()
                fallback=move || view! { <h1 class="flow-toolbar__name">{name}</h1> }
            >
                <input
                    type="text"
                    class="flow-toolbar__name flow-toolbar__name--input"
                    aria-label=move || t(FlowText::FlowName)
                    spellcheck="false"
                    autocomplete="off"
                    prop:value=move || editor.draft.with(|f| f.name.clone())
                    on:focus=move |_| armed.set_value(true)
                    on:input=rename
                />
            </Show>
            <Show
                when=move || editing.get()
                fallback=move || view! { <span class="flow-toolbar__mode">{mode}</span> }
            >
                <select
                    class="select flow-toolbar__room"
                    aria-label=move || t(FlowText::Mode)
                    title=move || t(FlowText::ModeHint)
                    prop:value=draft_mode
                    on:change=pick_mode
                >
                    <option value="single" selected=move || draft_mode() == "single">
                        {move || t(FlowText::ModeSingleOption)}
                    </option>
                    <option value="restart" selected=move || draft_mode() == "restart">
                        {move || t(FlowText::ModeRestartOption)}
                    </option>
                </select>
            </Show>
            <Show
                when=move || editing.get()
                fallback=move || {
                    room_name()
                        .map(|name| view! { <span class="flow-toolbar__mode">{name}</span> })
                }
            >
                <select
                    class="select flow-toolbar__room"
                    aria-label=move || t(FlowText::GroupRoom)
                    prop:value=draft_room
                    on:change=pick_room
                >
                    <option value="" selected=move || draft_room().is_empty()>
                        {move || t(FlowText::NoRoom)}
                    </option>
                    {move || {
                        let current = draft_room();
                        house
                            .areas
                            .with(|areas| {
                                areas
                                    .iter()
                                    .filter(|a| a.id != UNASSIGNED_AREA)
                                    .map(|a| {
                                        let selected = a.id == current;
                                        view! {
                                            <option value=a.id.clone() selected=selected>
                                                {a.name.clone()}
                                            </option>
                                        }
                                    })
                                    .collect_view()
                            })
                    }}
                </select>
            </Show>
        </div>
        <div class="flow-toolbar flow-toolbar--actions">
            <Show
                when=move || editing.get()
                fallback=move || view! { <ReadControls view=view editing=editing editor=editor selection=selection /> }
            >
                <EditControls
                    editing=editing
                    editor=editor
                    saving=saving
                    original=original
                />
            </Show>
            <button
                type="button"
                class="btn btn--ghost flow-toolbar__runs"
                class:flow-toolbar__runs--open=showing_runs
                aria-expanded=move || showing_runs().to_string()
                on:click=move |_| {
                    if panel.get_untracked() == Panel::Runs {
                        runs_open.set(false);
                        return;
                    }
                    // The runs take the panel over from a card's settings.
                    if editing.get_untracked()
                        && matches!(selection.get_untracked(), Some(Selection::Node(_)))
                    {
                        selection.set(None);
                    }
                    runs_open.set(true);
                }
            >
                {move || t(FlowText::Runs)}
            </button>
        </div>
    }
}
