//! The ⋯ menu of a flow row or card: open the flow, move it to a room, or delete it.

use leptos::ev;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use nidoo_core::model::UNASSIGNED_AREA;
use wasm_bindgen::JsCast;

use crate::api;
use crate::components::toast::Toasts;
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

/// A ⋯ button over a small popover. `current` is the flow's room (a known area id), `invalid`
/// flows cannot be moved (they can still be deleted). Delete asks first, inside the popover,
/// naming the flow by `name`. Esc or a press outside the menu closes it (see [`OpenMenu`]).
#[component]
pub fn RoomMenu(
    id: String,
    name: String,
    href: String,
    current: Option<String>,
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
