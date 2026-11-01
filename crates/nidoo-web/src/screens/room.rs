//! A room's page: its title and the cards Home shows there. The overview stacks these sections.

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::hooks::use_params_map;
use nidoo_api::OverridePatch;
use nidoo_core::model::UNASSIGNED_AREA;
use serde_json::json;

use crate::api::{encode_id, rename_area, set_override};
use crate::components::card_place::{CardPlace, provide_card_place};
use crate::components::room_title::{RoomEdit, RoomTitle};
use crate::components::toast::Toasts;
use crate::format::area_name;
use crate::groups::{HomeItem, home_items};
use crate::house::House;
use crate::i18n::devices::{DeviceText, device_text};
use crate::i18n::settings::{SettingsText, settings_text};
use crate::i18n::{DashText, Key, current_lang, dash_text, use_lang, use_text};
use crate::icons::GRIP;
use crate::nav::use_back_anchor;
use crate::room_order::{arranged, move_by, move_onto, order_changes};
use crate::rooms::{FAVORITES, use_group_refresh, use_room_chips};
use crate::screens::home::card_for;

/// Pointer travel in CSS pixels above which a press on a card starts a drag.
const DRAG_THRESHOLD: f64 = 6.0;

/// A press on a card or its grip, until it is released.
#[derive(Clone)]
struct Press {
    id: String,
    pointer: i32,
    x: f64,
    y: f64,
    /// Set once the pointer travelled past [`DRAG_THRESHOLD`].
    dragging: bool,
}

/// Drag-to-reorder state shared by the cards of one room while it is edited. A card dropped on
/// another takes its place (see [`move_onto`]).
#[derive(Clone, Copy)]
struct Reorder {
    press: StoredValue<Option<Press>>,
    /// The id of the card being dragged, for its lifted look.
    dragging: RwSignal<Option<String>>,
    /// How far the dragged card has followed the pointer.
    shift: RwSignal<(f64, f64)>,
    /// The card it would take the place of.
    target: RwSignal<Option<String>>,
}

impl Reorder {
    fn new() -> Self {
        Self {
            press: StoredValue::new(None),
            dragging: RwSignal::new(None),
            shift: RwSignal::new((0.0, 0.0)),
            target: RwSignal::new(None),
        }
    }

    fn down(&self, ev: &web_sys::PointerEvent, id: &str) {
        if ev.pointer_type() == "mouse" && ev.button() != 0 {
            return;
        }
        self.press.set_value(Some(Press {
            id: id.to_owned(),
            pointer: ev.pointer_id(),
            x: f64::from(ev.client_x()),
            y: f64::from(ev.client_y()),
            dragging: false,
        }));
    }

    /// Follows the pointer on the window: the dragged card takes no pointer events, so the
    /// card under it can be found.
    fn moved(&self, ev: &web_sys::PointerEvent) {
        let Some(mut press) = self.press.get_value() else {
            return;
        };
        if press.pointer != ev.pointer_id() {
            return;
        }
        let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
        if !press.dragging {
            if (x - press.x).hypot(y - press.y) < DRAG_THRESHOLD {
                return;
            }
            press.dragging = true;
            self.dragging.set(Some(press.id.clone()));
        }
        ev.prevent_default();
        self.shift.set((x - press.x, y - press.y));
        let target = document()
            .element_from_point(x as f32, y as f32)
            .and_then(|el| el.closest("[data-card-id]").ok().flatten())
            .and_then(|el| el.get_attribute("data-card-id"))
            .filter(|hovered| *hovered != press.id);
        if self.target.with_untracked(|t| *t != target) {
            self.target.set(target);
        }
        self.press.set_value(Some(press));
    }

    /// Ends a press; a drag released on another card moves there.
    fn up(&self, ev: &web_sys::PointerEvent, order: RwSignal<Vec<String>>) {
        let Some(press) = self.press.get_value() else {
            return;
        };
        if press.pointer != ev.pointer_id() {
            return;
        }
        let target = self.target.get_untracked();
        self.reset();
        if let Some(onto) = target.filter(|_| press.dragging) {
            order.update(|o| move_onto(o, &press.id, &onto));
        }
    }

    fn reset(&self) {
        self.press.set_value(None);
        self.dragging.set(None);
        self.target.set(None);
        self.shift.set((0.0, 0.0));
    }
}

/// The placeholder cards shown until the house has said anything.
#[component]
pub(crate) fn RoomsSkeleton() -> impl IntoView {
    view! {
        <div class="grid grid--home" aria-busy="true">
            <div class="card card--skeleton"></div>
            <div class="card card--skeleton"></div>
            <div class="card card--skeleton"></div>
            <div class="card card--skeleton"></div>
        </div>
    }
}

/// One room (or the Favorites chip): a renamable title and its cards. On the overview
/// (`anchored`) the section carries the id `room-<encoded id>`, and a click inside it makes Back
/// from the sheet it opens return to this room rather than the top of the page.
#[component]
pub(crate) fn RoomSection(id: String, #[prop(optional)] anchored: bool) -> impl IntoView {
    let house = expect_context::<House>();
    let anchor = anchored.then(|| format!("room-{}", encode_id(&id)));
    let mark = use_back_anchor();
    let on_click = {
        let anchor = anchor.clone();
        // Runs before the back stack records the sheet: that waits for an effect.
        move |_| {
            if let Some(anchor) = &anchor {
                mark(anchor);
            }
        }
    };
    // Under its own heading a card does not repeat the room; Favorites mixes rooms.
    if id != FAVORITES {
        provide_card_place(CardPlace::Room);
    }
    let lang = use_lang();
    let chip = id.clone();
    let items = Memo::new(move |_| {
        house.areas.with(|a| {
            house
                .entities
                .with(|m| house.groups.with(|g| home_items(m.values(), a, &chip, g)))
        })
    });
    let title = {
        let id = id.clone();
        Signal::derive(move || {
            if id == FAVORITES {
                return settings_text(lang(), SettingsText::Favorites).to_owned();
            }
            house.areas.with(|a| {
                a.iter()
                    .find(|a| a.id == id)
                    .map(|a| area_name(lang(), &a.id, &a.name))
                    .unwrap_or_default()
            })
        })
    };
    // Favorites mixes rooms and has no order of its own: it cannot be edited. Unassigned can be
    // reordered, not renamed.
    let editable = id != FAVORITES;
    let renamable = (editable && id != UNASSIGNED_AREA).then(|| id.clone());

    let toasts = expect_context::<Toasts>();
    let editing = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let draft_name = RwSignal::new(String::new());
    let draft_order = RwSignal::new(Vec::<String>::new());
    let live_ids = move || {
        items.with(|v| {
            v.iter()
                .map(|HomeItem::Entity(id)| id.clone())
                .collect::<Vec<_>>()
        })
    };
    // The cards in the order on screen: the draft while editing, Home order otherwise.
    let shown_ids = Memo::new(move |_| {
        let live = live_ids();
        if editing.get() {
            draft_order.with(|d| arranged(d, &live))
        } else {
            live
        }
    });
    let reorder = Reorder::new();
    if editable {
        let moved = window_event_listener(leptos::ev::pointermove, move |ev| reorder.moved(&ev));
        let up = window_event_listener(leptos::ev::pointerup, move |ev| {
            reorder.up(&ev, draft_order)
        });
        let cancel = window_event_listener(leptos::ev::pointercancel, move |_| reorder.reset());
        let escape = window_event_listener(leptos::ev::keydown, move |ev| {
            if ev.key() == "Escape" && reorder.dragging.with_untracked(Option::is_some) {
                reorder.reset();
            }
        });
        on_cleanup(move || {
            moved.remove();
            up.remove();
            cancel.remove();
            escape.remove();
        });
    }

    let start = Callback::new(move |()| {
        draft_name.set(title.get_untracked());
        draft_order.set(live_ids());
        editing.set(true);
    });
    let cancel = Callback::new(move |()| {
        reorder.reset();
        editing.set(false);
    });
    // One Save for the name and the order. Each card gets its place (1, 2, 3…) in its
    // retained override, so the order holds on every client until it is moved again; only the
    // places that changed are written. A failure keeps the edit open, and a second Save sends
    // what is still missing.
    let save = {
        let renamable = renamable.clone();
        Callback::new(move |()| {
            if busy.get_untracked() {
                return;
            }
            let lang = current_lang();
            let name = draft_name.get_untracked().trim().to_owned();
            let rename = match &renamable {
                Some(_) if name.is_empty() => {
                    toasts.push(device_text(lang, DeviceText::RoomNeedsName));
                    return;
                }
                Some(id) if name != title.get_untracked() => Some((id.clone(), name)),
                _ => None,
            };
            let ids = shown_ids.get_untracked();
            let changes = house
                .entities
                .with_untracked(|m| order_changes(&ids, |id| m.get(id).and_then(|e| e.ui.order)));
            if rename.is_none() && changes.is_empty() {
                editing.set(false);
                return;
            }
            busy.set(true);
            spawn_local(async move {
                if let Some((id, name)) = rename {
                    match rename_area(&id, &name).await {
                        Ok(areas) => house.areas.set(areas),
                        Err(e) => {
                            toasts.push(e.error);
                            busy.set(false);
                            return;
                        }
                    }
                }
                for (id, order) in changes {
                    let mut patch = OverridePatch::new();
                    patch.insert("order".to_owned(), json!(order));
                    match set_override(&id, patch).await {
                        Ok(view) => house.entities.update(|m| {
                            m.insert(view.id.clone(), view);
                        }),
                        Err(e) => {
                            toasts.push(e.error);
                            busy.set(false);
                            return;
                        }
                    }
                }
                busy.set(false);
                editing.set(false);
            });
        })
    };
    let edit = editable.then_some(RoomEdit {
        editing,
        busy,
        draft: draft_name,
        start,
        save,
        cancel,
    });

    view! {
        <section class="room" class:room--editing=move || editing.get() id=anchor on:click=on_click>
            <RoomTitle id=renamable name=title edit=edit />
            <Show
                when=move || items.with(|v| !v.is_empty())
                fallback=move || {
                    view! {
                        <p class="room__empty">
                            {move || settings_text(lang(), SettingsText::NothingHere)}
                        </p>
                    }
                }
            >
                <div
                    class="grid grid--home"
                    class:grid--dragging=move || reorder.dragging.with(Option::is_some)
                >
                    <For
                        each=move || shown_ids.get()
                        key=|id| id.clone()
                        children=move |id| {
                            let card = id.clone();
                            // One memo per card: it only changes when this entity does.
                            let entity = Memo::new(move |_| {
                                house.entities.with(|m| m.get(&card).cloned())
                            });
                            view! {
                                <RoomCard id=id reorder=reorder editing=editing order=draft_order>
                                    {move || entity.get().map(card_for)}
                                </RoomCard>
                            }
                        }
                    />
                </div>
            </Show>
        </section>
    }
}

/// A card of the room's grid. Outside edit mode the wrapper takes no box (`display: contents`),
/// so the card lays out as before. While editing, a layer over the card keeps its own controls
/// from firing and takes a mouse or pen drag; the grip on its top edge drags with touch too (a
/// touch on the card still scrolls the page) and moves the card one place with the arrow keys.
#[component]
fn RoomCard(
    id: String,
    reorder: Reorder,
    editing: RwSignal<bool>,
    order: RwSignal<Vec<String>>,
    children: Children,
) -> impl IntoView {
    let lang = use_lang();
    let is_dragged = {
        let id = id.clone();
        move || reorder.dragging.with(|d| d.as_deref() == Some(id.as_str()))
    };
    let is_target = {
        let id = id.clone();
        move || reorder.target.with(|t| t.as_deref() == Some(id.as_str()))
    };
    let style = {
        let is_dragged = is_dragged.clone();
        move || {
            is_dragged().then(|| {
                let (dx, dy) = reorder.shift.get();
                format!("translate:{dx}px {dy}px")
            })
        }
    };
    let overlay_id = id.clone();
    let grip_id = id.clone();
    let key_id = id.clone();
    view! {
        <div
            class="room-card"
            class:room-card--dragging=is_dragged
            class:room-card--drop-target=is_target
            style=style
            data-card-id=id
        >
            {children()}
            <Show when=move || editing.get()>
                <div
                    class="room-card__overlay"
                    on:pointerdown={
                        let overlay_id = overlay_id.clone();
                        move |ev: web_sys::PointerEvent| {
                            if ev.pointer_type() != "touch" {
                                reorder.down(&ev, &overlay_id);
                            }
                        }
                    }
                    on:click=|ev| ev.stop_propagation()
                ></div>
                <button
                    type="button"
                    class="room-card__grip"
                    aria-label=move || dash_text(lang(), DashText::Move)
                    title=move || dash_text(lang(), DashText::Move)
                    inner_html=GRIP
                    on:pointerdown={
                        let grip_id = grip_id.clone();
                        move |ev: web_sys::PointerEvent| reorder.down(&ev, &grip_id)
                    }
                    on:click=|ev| ev.stop_propagation()
                    on:keydown={
                        let key_id = key_id.clone();
                        move |ev: web_sys::KeyboardEvent| {
                            let step = match ev.key().as_str() {
                                "ArrowLeft" | "ArrowUp" => -1,
                                "ArrowRight" | "ArrowDown" => 1,
                                _ => return,
                            };
                            ev.prevent_default();
                            order.update(|o| move_by(o, &key_id, step));
                        }
                    }
                ></button>
            </Show>
        </div>
    }
}

/// `/rooms/:id`: one room, or a note that it is gone once the areas have loaded.
#[component]
pub fn RoomScreen() -> impl IntoView {
    let t = use_text();
    let params = use_params_map();
    let id = Memo::new(move |_| params.read().get("id").unwrap_or_default());
    let chips = use_room_chips();
    let house = expect_context::<House>();
    // The room pages wait for the areas: until they arrive, no id can be called gone.
    let loaded = Memo::new(move |_| house.areas.with(|a| !a.is_empty()));
    use_group_refresh();
    // The room on screen: `Some(id)` for any area the hub knows (even one with nothing on Home)
    // or a pseudo-room with a chip (Favorites), `None` once it is gone. One memo carrying the id,
    // so a room-to-room link rebuilds the page (a bare known/gone flag stays `true` and the page
    // kept the first room), yet a card's state echo never does.
    let room = Memo::new(move |_| {
        let id = id.get();
        let known = house.areas.with(|a| a.iter().any(|a| a.id == id))
            || chips.with(|list| list.iter().any(|c| c.id == id));
        known.then_some(id)
    });

    view! {
        <Show when=move || loaded.get() fallback=|| view! { <RoomsSkeleton /> }>
            {move || match room.get() {
                Some(id) => view! { <RoomSection id=id /> }.into_any(),
                None => view! { <p class="room__gone">{move || t(Key::RoomGone)}</p> }.into_any(),
            }}
        </Show>
    }
}
