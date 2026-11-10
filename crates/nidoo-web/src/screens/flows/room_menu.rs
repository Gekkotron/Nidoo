//! The ⋯ menu of a flow row or card: open the flow, move it to a room, tag it with labels, or
//! delete it.

use leptos::ev;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use nidoo_api::LabelEdit;
use nidoo_core::model::UNASSIGNED_AREA;
use wasm_bindgen::JsCast;

use crate::api;
use crate::api::flow_labels::{edit_flow_labels, set_flow_labels};
use crate::components::toast::Toasts;
use crate::flow_list::{label_by_name, new_label_id, toggled_labels};
use crate::house::House;
use crate::i18n::flows::{FlowText, delete_flow_prompt, use_flow_text};
use crate::i18n::{Key, use_lang, use_text};

/// Which flow's ⋯ menu is open (its id), held by the page rather than by each menu: a row the
/// list rebuilds keeps its menu open, only one menu is ever open, and one pair of window
/// listeners serves them all.
#[derive(Clone, Copy)]
pub struct OpenMenu(pub(super) RwSignal<Option<String>>);

/// Provides [`OpenMenu`] to the page and registers its one Escape / outside-press listener pair,
/// removed when the page goes away.
pub fn provide_open_menu() {
    let open = RwSignal::new(None::<String>);
    provide_context(OpenMenu(open));
    let esc = window_event_listener(ev::keydown, move |ev| {
        if ev.key() == "Escape" && open.with_untracked(Option::is_some) {
            open.set(None);
        }
    });
    let outside = window_event_listener(ev::pointerdown, move |ev| {
        if open.with_untracked(Option::is_none) {
            return;
        }
        let inside = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .is_some_and(|el| el.closest(".room-menu").ok().flatten().is_some());
        if !inside {
            open.set(None);
        }
    });
    on_cleanup(move || {
        esc.remove();
        outside.remove();
    });
}

/// A ⋯ button over a small popover. `current` is the flow's room (a known area id), `labels`
/// the label ids it carries; `invalid` flows cannot be moved or tagged (they can still be
/// deleted). Delete asks first, inside the popover,
/// naming the flow by `name`. Esc or a press outside the menu closes it (see [`OpenMenu`]).
#[component]
pub fn RoomMenu(
    id: String,
    name: String,
    href: String,
    current: Option<String>,
    labels: Vec<String>,
    invalid: bool,
) -> impl IntoView {
    let house = expect_context::<House>();
    let toasts = expect_context::<Toasts>();
    let OpenMenu(open_menu) = expect_context::<OpenMenu>();
    let lang = use_lang();
    let t = use_flow_text();
    let key_text = use_text();
    let open = {
        let id = id.clone();
        Memo::new(move |_| open_menu.with(|o| o.as_deref() == Some(id.as_str())))
    };

    // Asking whether to delete; dropped whenever the menu closes.
    let confirming = RwSignal::new(false);
    Effect::new(move |_| {
        if !open.get() {
            confirming.set(false);
        }
    });
    // The swap removes the focused button, so focus follows it: the confirm opens on Cancel
    // and Cancel goes back to Delete flow…. Each returns whether it was confirming.
    let cancel_button = NodeRef::<leptos::html::Button>::new();
    let delete_item = NodeRef::<leptos::html::Button>::new();
    Effect::new(move |was: Option<bool>| {
        let now = confirming.get();
        if now {
            if let Some(button) = cancel_button.get() {
                let _ = button.focus();
            }
        } else if was == Some(true)
            && let Some(button) = delete_item.get()
        {
            let _ = button.focus();
        }
        now
    });

    let id = StoredValue::new(id);
    // The hub's push refreshes the list, like the enable switch.
    let move_to = move |area: Option<String>| {
        open_menu.set(None);
        let id = id.get_value();
        spawn_local(async move {
            if let Err(e) = api::set_flow_area(&id, area.as_deref()).await {
                toasts.push(e.error);
            }
        });
    };

    let item = move |label: Signal<String>, area: Option<String>, checked: bool| {
        let target = area.clone();
        view! {
            <button
                type="button"
                role="menuitemradio"
                class="room-menu__item"
                aria-checked=checked.to_string()
                disabled=invalid
                title=move || invalid.then(|| t(FlowText::FixFlowFirst))
                on:click=move |_| move_to(target.clone())
            >
                <span class="room-menu__check" aria-hidden="true">
                    {checked.then_some("✓")}
                </span>
                {move || label.get()}
            </button>
        }
    };
    let rooms = {
        let current = current.clone();
        move || {
            house.areas.with(|areas| {
                areas
                    .iter()
                    .filter(|a| a.id != UNASSIGNED_AREA)
                    .map(|a| {
                        let checked = current.as_deref() == Some(a.id.as_str());
                        item(Signal::stored(a.name.clone()), Some(a.id.clone()), checked).into_any()
                    })
                    .collect_view()
            })
        }
    };
    let none_checked = current.is_none();

    // The labels as last asked for: a second tick before the hub's push builds on the first.
    let carried = RwSignal::new(labels);
    let toggle = move |label: String| {
        let before = carried.get_untracked();
        let next = toggled_labels(&before, &label);
        carried.set(next.clone());
        let id = id.get_value();
        spawn_local(async move {
            if let Err(e) = set_flow_labels(&id, &next).await {
                carried.try_set(before);
                toasts.push(e.error);
            }
        });
    };
    let label_items = move || {
        house.flow_labels.with(|v| {
            let catalog = &v.catalog;
            catalog
                .labels
                .iter()
                .map(|label| {
                    let id = label.id.clone();
                    let checked = carried.with(|c| c.contains(&id));
                    let choice = label
                        .choice
                        .as_deref()
                        .and_then(|c| catalog.choice(c))
                        .map(|c| format!(" · {}", c.name));
                    view! {
                        <button
                            type="button"
                            role="menuitemcheckbox"
                            class="room-menu__item"
                            aria-checked=checked.to_string()
                            disabled=invalid
                            title=move || invalid.then(|| t(FlowText::FixFlowFirst))
                            on:click=move |_| toggle(id.clone())
                        >
                            <span class="room-menu__check" aria-hidden="true">
                                {checked.then_some("✓")}
                            </span>
                            {label.name.clone()}
                            {choice.map(|c| view! { <span class="room-menu__choice">{c}</span> })}
                        </button>
                    }
                })
                .collect_view()
        })
    };
    // "＋ New label…" turns into a name field; Create makes the label (or finds one of that
    // name) and tags the flow with it.
    let naming = RwSignal::new(false);
    let new_name = RwSignal::new(String::new());
    Effect::new(move |_| {
        if !open.get() {
            naming.set(false);
            new_name.set(String::new());
        }
    });
    let create = move || {
        let name = new_name.get_untracked().trim().to_owned();
        if name.is_empty() {
            return;
        }
        naming.set(false);
        new_name.set(String::new());
        let known = house
            .flow_labels
            .with_untracked(|v| label_by_name(&v.catalog, &name).map(|l| l.id.clone()));
        if let Some(label) = known {
            if !carried.with_untracked(|c| c.contains(&label)) {
                toggle(label);
            }
            return;
        }
        let before = house.flow_labels.with_untracked(|v| v.catalog.clone());
        spawn_local(async move {
            match edit_flow_labels(&LabelEdit::CreateLabel { name, choice: None }).await {
                Ok(view) => {
                    let label = new_label_id(&before, &view.catalog);
                    house.flow_labels.set(view);
                    if let Some(label) = label {
                        toggle(label);
                    }
                }
                Err(e) => toasts.push(e.error),
            }
        });
    };
    let new_label = move || {
        if naming.get() {
            view! {
                <div class="room-menu__new">
                    <input
                        type="text"
                        class="input"
                        aria-label=move || t(FlowText::LabelNameField)
                        placeholder=move || t(FlowText::LabelNameField)
                        autocomplete="off"
                        prop:value=move || new_name.get()
                        on:input=move |ev| new_name.set(event_target_value(&ev))
                        on:keydown=move |ev: web_sys::KeyboardEvent| {
                            if ev.key() == "Enter" {
                                ev.prevent_default();
                                create();
                            }
                        }
                    />
                    <button type="button" class="btn" on:click=move |_| create()>
                        {move || t(FlowText::CreateLabelButton)}
                    </button>
                </div>
            }
            .into_any()
        } else {
            view! {
                <button
                    type="button"
                    role="menuitem"
                    class="room-menu__item"
                    disabled=invalid
                    title=move || invalid.then(|| t(FlowText::FixFlowFirst))
                    on:click=move |_| {
                        naming.set(true);
                        request_animation_frame(|| {
                            let field = web_sys::window()
                                .and_then(|w| w.document())
                                .and_then(|d| d.query_selector(".room-menu__new input").ok().flatten())
                                .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok());
                            if let Some(field) = field {
                                let _ = field.focus();
                            }
                        });
                    }
                >
                    <span class="room-menu__check" aria-hidden="true"></span>
                    "＋ "
                    {move || t(FlowText::NewLabel)}
                </button>
            }
            .into_any()
        }
    };
    // The hub's push drops the row; a refusal shows as a toast and the row stays.
    let delete = move |_| {
        open_menu.set(None);
        let id = id.get_value();
        spawn_local(async move {
            if let Err(e) = api::delete_flow(&id).await {
                toasts.push(e.error);
            }
        });
    };
    let name = StoredValue::new(name);

    view! {
        <span class="room-menu">
            <button
                type="button"
                class="btn btn--ghost btn--icon room-menu__button"
                aria-label=move || t(FlowText::FlowActions)
                title=move || t(FlowText::FlowActions)
                aria-haspopup="menu"
                aria-expanded=move || open.get().to_string()
                on:click=move |_| {
                    let id = id.get_value();
                    open_menu.update(|o| *o = (o.as_deref() != Some(id.as_str())).then_some(id));
                }
            >
                "⋯"
            </button>
            <Show when=move || open.get()>
                <div class="room-menu__pop" role="menu">
                    <Show
                        when=move || confirming.get()
                        fallback={
                            let href = href.clone();
                            let rooms = rooms.clone();
                            move || {
                                view! {
                                    <A
                                        href=href.clone()
                                        attr:class="room-menu__item"
                                        attr:role="menuitem"
                                    >
                                        <span class="room-menu__check" aria-hidden="true"></span>
                                        {move || t(FlowText::OpenFlow)}
                                    </A>
                                    <p class="room-menu__head" role="presentation">
                                        {move || t(FlowText::MoveToRoom)}
                                    </p>
                                    {rooms.clone()}
                                    {item(
                                        Signal::derive(move || t(FlowText::NoRoom).to_owned()),
                                        None,
                                        none_checked,
                                    )}
                                    <p class="room-menu__head" role="presentation">
                                        {move || t(FlowText::Labels)}
                                    </p>
                                    {label_items}
                                    {new_label}
                                    <button
                                        type="button"
                                        role="menuitem"
                                        class="room-menu__item room-menu__item--danger"
                                        node_ref=delete_item
                                        on:click=move |_| confirming.set(true)
                                    >
                                        <span class="room-menu__check" aria-hidden="true"></span>
                                        {move || t(FlowText::DeleteFlow)}
                                    </button>
                                }
                            }
                        }
                    >
                        <div class="room-menu__confirm" role="alertdialog" aria-label=move || t(FlowText::DeleteFlowLabel)>
                            <p class="room-menu__prompt">
                                <strong>{move || delete_flow_prompt(lang(), &name.get_value())}</strong>
                                " "
                                {move || t(FlowText::CantUndo)}
                            </p>
                            <div class="room-menu__actions">
                                <button
                                    type="button"
                                    class="btn btn--ghost"
                                    node_ref=cancel_button
                                    on:click=move |_| confirming.set(false)
                                >
                                    {move || t(FlowText::Cancel)}
                                </button>
                                <button type="button" class="btn btn--danger" on:click=delete>
                                    {move || key_text(Key::Delete)}
                                </button>
                            </div>
                        </div>
                    </Show>
                </div>
            </Show>
        </span>
    }
}
