//! The picker's screens: [`PickerBody`] (search, chips, rooms, the list) and [`EntityPicker`],
//! the modal `<dialog>` that holds it.

use std::sync::atomic::{AtomicUsize, Ordering};

use leptos::prelude::*;
use web_sys::{KeyboardEvent, ScrollIntoViewOptions, ScrollLogicalPosition};

use super::model::{Chip, Filter, Listing, Row, listing};
use super::{Keep, Suggest};
use crate::alerts::relative_time;
use crate::components::back_close::BackClose;
use crate::components::now::Now;
use crate::format::display_value;
use crate::house::House;
use crate::i18n::picker::{PickerText, capped_label, done_label, use_picker_text};
use crate::i18n::use_lang;
use nidoo_core::model::Availability;

/// Whether a pick chooses one entity (and closes) or toggles entities in a set.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Single,
    Multi,
}

/// The top row above the list: "None" / "Automatic · <entity>". Picking it reports `None`.
#[derive(Clone)]
pub struct Pinned {
    pub text: Signal<String>,
}

/// What the list holds between the pinned and missing rows: a room heading or an entity.
#[derive(Clone, PartialEq)]
enum Item {
    Title { area: String, room: String },
    Row(Row),
}

impl Item {
    /// Rows re-render when what they show changes, so the key carries the title and device.
    fn key(&self) -> String {
        match self {
            Item::Title { area, room } => format!("t:{area}:{room}"),
            Item::Row(r) => format!("{}|{}|{:?}", r.id, r.title, r.device),
        }
    }
}

fn items(l: &Listing) -> Vec<Item> {
    let mut out = Vec::new();
    for s in &l.sections {
        out.push(Item::Title {
            area: s.area.clone(),
            room: s.room.clone(),
        });
        out.extend(s.rows.iter().cloned().map(Item::Row));
    }
    out
}

/// A prefix no other picker on the page shares, so ids and the ARIA references to them stay unique.
fn next_prefix() -> String {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    format!("picker{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// The DOM id of the row for `id` (`None` is the pinned row) in the picker with `prefix`.
fn dom_id(prefix: &str, id: Option<&str>) -> String {
    id.map_or_else(
        || format!("{prefix}-pin"),
        |id| format!("{prefix}-opt-{id}"),
    )
}

fn scroll_to(dom: &str) {
    if let Some(el) = document().get_element_by_id(dom) {
        let o = ScrollIntoViewOptions::new();
        o.set_block(ScrollLogicalPosition::Nearest);
        el.scroll_into_view_with_scroll_into_view_options(&o);
    }
}

/// One entity row. The value and age read the entity by id, so a state echo touches only them.
#[component]
fn PickerRow(
    row: Row,
    prefix: String,
    mode: Mode,
    current: Signal<Vec<String>>,
    active: Memo<Option<String>>,
    on_pick: Callback<Option<String>>,
) -> impl IntoView {
    let house = expect_context::<House>();
    let t = use_picker_text();
    let id = row.id.clone();
    let dom = dom_id(&prefix, Some(&id));
    let off = Memo::new({
        let id = id.clone();
        move |_| {
            house.entities.with(|m| {
                m.get(&id)
                    .is_some_and(|e| e.state.availability == Availability::Offline)
            })
        }
    });
    let selected = Memo::new({
        let id = id.clone();
        move |_| current.with(|c| c.contains(&id))
    });
    let is_active = {
        let dom = dom.clone();
        move || active.with(|a| a.as_deref() == Some(dom.as_str()))
    };
    let click_id = id.clone();
    view! {
        <div
            id=dom
            role="option"
            class="picker__row"
            class:picker__row--off=move || off.get()
            class:picker__row--active=is_active
            aria-selected=move || selected.get().to_string()
            on:click=move |_| on_pick.run(Some(click_id.clone()))
        >
            {(mode == Mode::Multi)
                .then(|| {
                    view! {
                        <input
                            type="checkbox"
                            class="picker__check"
                            tabindex="-1"
                            aria-hidden="true"
                            prop:checked=move || selected.get()
                            on:click=|ev| ev.prevent_default()
                        />
                    }
                })}
            <span class="picker__icon" inner_html=row.icon></span>
            <span class="picker__row-main">
                <span class="picker__row-title">{row.title}</span>
                <span class="picker__row-sub">
                    {row.device.map(|d| view! { <span class="picker__row-device">{d}</span> })}
                    {row.room.map(|r| view! { <span class="picker__row-room">{r}</span> })}
                    {move || {
                        off.get()
                            .then(|| {
                                view! {
                                    <span class="picker__row-off">
                                        {t(PickerText::Unavailable)}
                                    </span>
                                }
                            })
                    }}
                </span>
            </span>
            <RowSide id=id />
        </div>
    }
}

/// The value and age of entity `id`, read by id so a state echo touches only these two spans.
#[component]
pub(super) fn RowSide(id: String) -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let now = use_context::<Now>();
    let value = Memo::new({
        let id = id.clone();
        move |_| {
            let lang = lang();
            house.entities.with(|m| {
                m.get(&id)
                    .map(|e| display_value(lang, &e.state, e.unit.as_deref()))
                    .unwrap_or_default()
            })
        }
    });
    let stamp = Memo::new(move |_| {
        house
            .entities
            .with(|m| m.get(&id).and_then(|e| e.state.last_update))
    });
    let age = move || {
        stamp.get().map(|at| {
            let n = now.map_or(at, |n| n.0.get());
            relative_time(lang(), at, n)
        })
    };
    view! {
        <span class="picker__row-side">
            <span class="picker__row-value">{move || value.get()}</span>
            <span class="picker__row-time">{age}</span>
        </span>
    }
}

/// The picker without its frame: search box, chips, room column and the list.
#[component]
pub fn PickerBody(
    mode: Mode,
    #[prop(into)] current: Signal<Vec<String>>,
    #[prop(optional_no_strip)] pool: Option<Keep>,
    #[prop(optional_no_strip)] suggest: Option<Suggest>,
    #[prop(optional_no_strip)] pinned: Option<Pinned>,
    /// Names listed first, after the pinned row (a flow's math results); a pick reports
    /// `Some(name)` like an entity does. They narrow with the search box and hide under a room.
    #[prop(optional_no_strip)]
    results: Option<Signal<Vec<String>>>,
    /// Single: the pick (`None` is the pinned row). Multi: `Some(id)` toggles `id`.
    on_pick: Callback<Option<String>>,
    /// No room column at any width (the group wizard).
    #[prop(optional)]
    inline: bool,
) -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let t = use_picker_text();
    let prefix = next_prefix();
    let list_id = format!("{prefix}-list");
    let pin_id = dom_id(&prefix, None);
    let query = RwSignal::new(String::new());
    let room = RwSignal::new(None::<String>);
    let chip = RwSignal::new(if suggest.is_some() {
        Chip::Suggested
    } else {
        Chip::All
    });
    let diagnostic = RwSignal::new(false);
    let hi = RwSignal::new(None::<usize>);
    let suggest_keep = suggest.as_ref().map(|s| s.keep.clone());
    let suggest_text = suggest.as_ref().map(|s| s.text);
    let has_pinned = pinned.is_some();
    let named = Memo::new(move |_| {
        let q = query.get().trim().to_lowercase();
        if room.with(Option::is_some) {
            return Vec::new();
        }
        results
            .map(|r| r.with(|r| r.iter().filter(|n| n.contains(&q)).cloned().collect()))
            .unwrap_or_default()
    });

    // Equal listings do not redraw: a state echo changes no row.
    let list = Memo::new(move |_| {
        let lang = lang();
        let cur = current.get();
        let q = query.get();
        let r = room.get();
        let (c, d) = (chip.get(), diagnostic.get());
        house.entities.with(|ents| {
            house.areas.with(|areas| {
                listing(
                    lang,
                    ents,
                    areas,
                    &Filter {
                        pool: pool.as_ref(),
                        suggest: suggest_keep.as_ref(),
                        chip: c,
                        diagnostic: d,
                        query: &q,
                        room: r.as_deref(),
                        current: &cur,
                    },
                )
            })
        })
    });
    let list_items = Memo::new(move |_| list.with(items));
    // What the arrow keys walk: the pinned row (`None`), the missing rows, then the rows.
    let flat = Memo::new(move |_| {
        let mut out: Vec<Option<String>> = Vec::new();
        if has_pinned {
            out.push(None);
        }
        named.with(|n| out.extend(n.iter().cloned().map(Some)));
        list.with(|l| {
            out.extend(l.missing.iter().cloned().map(Some));
            for s in &l.sections {
                out.extend(s.rows.iter().map(|r| Some(r.id.clone())));
            }
        });
        out
    });
    let active = Memo::new({
        let prefix = prefix.clone();
        move |_| {
            hi.get()
                .and_then(|i| flat.with(|f| f.get(i).map(|id| dom_id(&prefix, id.as_deref()))))
        }
    });
    // A narrowing starts the highlight over.
    Effect::new(move |_| {
        query.track();
        room.track();
        chip.track();
        diagnostic.track();
        hi.set(None);
    });
    Effect::new(move |_| {
        if let Some(dom) = active.get() {
            scroll_to(&dom);
        }
    });
    let search = NodeRef::<leptos::html::Input>::new();
    Effect::new(move |_| {
        if let Some(input) = search.get() {
            let _ = input.focus();
        }
    });

    let on_key = move |ev: KeyboardEvent| match ev.key().to_lowercase().as_str() {
        "arrowdown" => {
            ev.prevent_default();
            let n = flat.with(Vec::len);
            if n > 0 {
                hi.update(|h| *h = Some(h.map_or(0, |i| (i + 1).min(n - 1))));
            }
        }
        "arrowup" => {
            ev.prevent_default();
            if flat.with(Vec::len) > 0 {
                hi.update(|h| *h = Some(h.map_or(0, |i| i.saturating_sub(1))));
            }
        }
        "enter" => {
            ev.prevent_default();
            let id = hi
                .get_untracked()
                .and_then(|i| flat.with_untracked(|f| f.get(i).cloned()));
            if let Some(id) = id {
                on_pick.run(id);
            }
        }
        _ => {}
    };

    let empty = move || {
        named.with(Vec::is_empty) && list.with(|l| l.missing.is_empty() && l.sections.is_empty())
    };
    let can_widen = suggest.is_some();
    let rooms = move || list.with(|l| l.rooms.clone());
    let missing = move || list.with(|l| l.missing.clone());
    let pinned_text = pinned.map(|p| p.text);

    view! {
        <div
            class="picker-body"
            class:picker-body--inline=inline
            class:picker-body--room=move || room.with(Option::is_some)
        >
            <input
                node_ref=search
                type="search"
                class="picker__search"
                role="combobox"
                autocomplete="off"
                aria-controls=list_id.clone()
                aria-expanded="true"
                aria-autocomplete="list"
                aria-activedescendant=move || active.get()
                placeholder=move || t(PickerText::Search)
                prop:value=move || query.get()
                on:input=move |ev| {
                    query.set(event_target_value(&ev));
                    room.set(None);
                }
                on:keydown=on_key
            />
            <div class="picker__chips">
                {suggest_text
                    .map(|text| {
                        view! {
                            <button
                                type="button"
                                class="chip"
                                class:chip--selected=move || chip.get() == Chip::Suggested
                                on:click=move |_| chip.set(Chip::Suggested)
                            >
                                {move || t(text)}
                            </button>
                        }
                    })}
                <button
                    type="button"
                    class="chip"
                    class:chip--selected=move || chip.get() == Chip::All
                    on:click=move |_| chip.set(Chip::All)
                >
                    {move || t(PickerText::All)}
                </button>
                <button
                    type="button"
                    class="chip"
                    class:chip--selected=move || diagnostic.get()
                    aria-pressed=move || diagnostic.get().to_string()
                    on:click=move |_| diagnostic.update(|d| *d = !*d)
                >
                    {move || t(PickerText::Diagnostic)}
                </button>
            </div>
            <div class="picker__cols">
                <nav class="picker__rooms">
                    <button
                        type="button"
                        class="picker__room"
                        class:picker__room--on=move || room.with(Option::is_none)
                        on:click=move |_| room.set(None)
                    >
                        <span>{move || t(PickerText::AllRooms)}</span>
                        <span class="picker__count">{move || list.with(|l| l.total)}</span>
                    </button>
                    <For
                        each=rooms
                        key=|r| format!("{}:{}:{}", r.area, r.room, r.count)
                        children=move |r| {
                            let area = r.area.clone();
                            let on = {
                                let area = area.clone();
                                move || room.with(|c| c.as_deref() == Some(area.as_str()))
                            };
                            view! {
                                <button
                                    type="button"
                                    class="picker__room"
                                    class:picker__room--on=on
                                    on:click=move |_| room.set(Some(area.clone()))
                                >
                                    <span>{r.room}</span>
                                    <span class="picker__count">{r.count}</span>
                                </button>
                            }
                        }
                    />
                </nav>
                <div class="picker__list" id=list_id role="listbox">
                    {pinned_text
                        .map(|text| {
                            let pin_dom = pin_id.clone();
                            let pin_active = pin_id.clone();
                            view! {
                                <div
                                    id=pin_dom
                                    role="option"
                                    class="picker__row picker__row--pinned"
                                    class:picker__row--active=move || {
                                        active.with(|a| a.as_deref() == Some(pin_active.as_str()))
                                    }
                                    aria-selected=move || current.with(Vec::is_empty).to_string()
                                    on:click=move |_| on_pick.run(None)
                                >
                                    <span class="picker__row-main">
                                        <span class="picker__row-title">{move || text.get()}</span>
                                    </span>
                                </div>
                            }
                        })}
                    <For
                        each=move || named.get()
                        key=|name| name.clone()
                        children={
                            let prefix = prefix.clone();
                            move |name| {
                                let dom = dom_id(&prefix, Some(&name));
                                let selected = {
                                    let name = name.clone();
                                    move || current.with(|c| c.contains(&name)).to_string()
                                };
                                let is_active = {
                                    let dom = dom.clone();
                                    move || active.with(|a| a.as_deref() == Some(dom.as_str()))
                                };
                                let click = name.clone();
                                view! {
                                    <div
                                        id=dom
                                        role="option"
                                        class="picker__row picker__row--result"
                                        class:picker__row--active=is_active
                                        aria-selected=selected
                                        on:click=move |_| on_pick.run(Some(click.clone()))
                                    >
                                        <span class="picker__row-main">
                                            <span class="picker__row-title">{name}</span>
                                        </span>
                                    </div>
                                }
                            }
                        }
                    />
                    <For
                        each=missing
                        key=|id| id.clone()
                        children={
                            let prefix = prefix.clone();
                            move |id| {
                            let dom = dom_id(&prefix, Some(&id));
                            let selected = {
                                let id = id.clone();
                                move || current.with(|c| c.contains(&id)).to_string()
                            };
                            let is_active = {
                                let dom = dom.clone();
                                move || active.with(|a| a.as_deref() == Some(dom.as_str()))
                            };
                            let click = id.clone();
                            view! {
                                <div
                                    id=dom
                                    role="option"
                                    class="picker__row picker__row--missing"
                                    class:picker__row--active=is_active
                                    aria-selected=selected
                                    on:click=move |_| on_pick.run(Some(click.clone()))
                                >
                                    <span class="picker__row-main">
                                        <span class="picker__row-title">{id}</span>
                                        <span class="picker__row-sub">
                                            <span class="picker__row-off">
                                                {move || t(PickerText::Missing)}
                                            </span>
                                        </span>
                                    </span>
                                </div>
                            }
                            }
                        }
                    />
                    <For
                        each=move || list_items.get()
                        key=Item::key
                        children=move |item| match item {
                            Item::Title { room, .. } => {
                                view! { <div class="picker__section-title">{room}</div> }.into_any()
                            }
                            Item::Row(r) => {
                                view! {
                                    <PickerRow
                                        row=r
                                        prefix=prefix.clone()
                                        mode=mode
                                        current=current
                                        active=active
                                        on_pick=on_pick
                                    />
                                }
                                    .into_any()
                            }
                        }
                    />
                    {move || {
                        list.with(|l| {
                            (l.shown < l.total
                                && query.with(|q| q.trim().is_empty())
                                && room.with(Option::is_none))
                                .then(|| {
                                    view! {
                                        <p class="picker__capped">
                                            {capped_label(lang(), l.shown, l.total)}
                                        </p>
                                    }
                                })
                        })
                    }}
                    <Show when=empty>
                        <div class="picker__empty">
                            <p>{move || t(PickerText::NoMatch)}</p>
                            <Show when=move || can_widen && chip.get() == Chip::Suggested>
                                <button
                                    type="button"
                                    class="chip"
                                    on:click=move |_| {
                                        chip.set(Chip::All);
                                        room.set(None);
                                    }
                                >
                                    {move || t(PickerText::SearchAll)}
                                </button>
                            </Show>
                        </div>
                    </Show>
                </div>
            </div>
        </div>
    }
}

/// The modal picker: a `<dialog>` opened while `open` is true. Esc, the backdrop, the close
/// button and the browser's Back all close it; a Single pick closes it too.
#[component]
pub fn EntityPicker(
    open: RwSignal<bool>,
    #[prop(into)] title: Signal<String>,
    mode: Mode,
    #[prop(into)] current: Signal<Vec<String>>,
    #[prop(optional_no_strip)] pool: Option<Keep>,
    #[prop(optional_no_strip)] suggest: Option<Suggest>,
    #[prop(optional_no_strip)] pinned: Option<Pinned>,
    #[prop(optional_no_strip)] results: Option<Signal<Vec<String>>>,
    on_pick: Callback<Option<String>>,
) -> impl IntoView {
    let lang = use_lang();
    let t = use_picker_text();
    let dialog = NodeRef::<leptos::html::Dialog>::new();
    let title_id = format!("{}-title", next_prefix());
    // The body mounts once the dialog is open, so it can take focus and starts fresh each time.
    let shown = RwSignal::new(false);
    let hook = BackClose::new();
    Effect::new(move |_| {
        let want = open.get();
        let Some(d) = dialog.get() else {
            return;
        };
        if want {
            if !d.open() {
                let _ = d.show_modal();
            }
            if !hook.is_attached() {
                hook.attach(Callback::new(move |()| open.set(false)));
            }
            shown.set(true);
        } else {
            shown.set(false);
            hook.detach();
            if d.open() {
                d.close();
            }
        }
    });
    on_cleanup(move || hook.detach());

    let close = move |_| open.set(false);
    let body_pick = Callback::new(move |id: Option<String>| {
        on_pick.run(id);
        if mode == Mode::Single {
            open.set(false);
        }
    });
    view! {
        <dialog
            node_ref=dialog
            class="picker"
            aria-labelledby=title_id.clone()
            on:cancel=move |ev: web_sys::Event| {
                ev.prevent_default();
                open.set(false);
            }
            on:close=move |_| open.set(false)
            on:keydown=|ev: KeyboardEvent| ev.stop_propagation()
            on:click=move |ev| {
                let target = ev.target();
                let me: Option<web_sys::EventTarget> = dialog.get_untracked().map(Into::into);
                if target.is_some() && target == me {
                    open.set(false);
                }
            }
        >
            <div class="picker__panel">
                <header class="picker__head">
                    <h2 id=title_id.clone() class="picker__title">
                        {move || title.get()}
                    </h2>
                    {(mode == Mode::Multi)
                        .then(|| {
                            view! {
                                <button type="button" class="btn picker__done" on:click=close>
                                    {move || done_label(lang(), current.with(Vec::len))}
                                </button>
                            }
                        })}
                    <button
                        type="button"
                        class="picker__close"
                        aria-label=move || t(PickerText::Close)
                        on:click=close
                    >
                        <span class="picker__close-x">"✕"</span>
                        <span class="picker__close-back">"←"</span>
                    </button>
                </header>
                {move || {
                    shown
                        .get()
                        .then(|| {
                            view! {
                                <PickerBody
                                    mode=mode
                                    current=current
                                    pool=pool.clone()
                                    suggest=suggest.clone()
                                    pinned=pinned.clone()
                                    results=results
                                    on_pick=body_pick
                                />
                            }
                        })
                }}
            </div>
        </dialog>
    }
}
