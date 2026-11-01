//! The fields that open the picker: one entity (or a pinned choice), a list of entities, and
//! the "+ Add…" button the list is built on.

use leptos::prelude::*;

use super::model::{Row, row_of};
use super::view::{EntityPicker, Mode, Pinned, RowSide};
use super::{Keep, Suggest};
use crate::format::room_name;
use crate::house::House;
use crate::i18n::picker::{PickerText, use_picker_text};
use crate::i18n::use_lang;

/// The row of entity `id`, or `None` while the house lacks it. Equal rows do not redraw.
fn use_row(id: Signal<Option<String>>) -> Memo<Option<Row>> {
    let house = expect_context::<House>();
    let lang = use_lang();
    Memo::new(move |_| {
        let id = id.get()?;
        let lang = lang();
        house.entities.with(|ents| {
            let e = ents.get(&id)?;
            let room = house
                .areas
                .with(|areas| room_name(areas, &e.area))
                .filter(|r| !r.is_empty());
            Some(row_of(lang, e, room))
        })
    })
}

/// "device · room" under a title.
#[component]
fn RowSub(device: Option<String>, room: Option<String>) -> impl IntoView {
    view! {
        <span class="picker__row-sub">
            {device.map(|d| view! { <span class="picker__row-device">{d}</span> })}
            {room.map(|r| view! { <span class="picker__row-room">{r}</span> })}
        </span>
    }
}

/// One entity, or the pinned choice. Closed: the chosen entity's full row, or the pinned text.
#[component]
pub fn EntityPickField(
    #[prop(into)] label: Signal<String>,
    #[prop(into)] value: Signal<Option<String>>,
    on_change: Callback<Option<String>>,
    #[prop(optional_no_strip)] pool: Option<Keep>,
    #[prop(optional_no_strip)] suggest: Option<Suggest>,
    /// "None" / "Automatic · …": shown when `value` is None, and offered as the pinned row.
    /// Without it the field is required (no pinned row, closed text = `PickerText::Choose`).
    #[prop(optional_no_strip)]
    pinned: Option<Signal<String>>,
    /// Names offered before the entities (a flow's math results); a `value` that is one of
    /// them shows as the plain name.
    #[prop(optional_no_strip)]
    results: Option<Signal<Vec<String>>>,
    #[prop(optional)] compact: bool,
    /// CSS class of the wrapping element, so a host keeps its own field layout.
    #[prop(optional_no_strip)]
    class: Option<String>,
) -> impl IntoView {
    let t = use_picker_text();
    let open = RwSignal::new(false);
    let row = use_row(value);
    let chosen = move || value.get();
    let current = Signal::derive(move || value.get().into_iter().collect::<Vec<_>>());
    let closed = move || match chosen() {
        None => {
            let text = pinned.map_or_else(|| t(PickerText::Choose).to_owned(), |p| p.get());
            view! { <span class="picker-field__text">{text}</span> }.into_any()
        }
        Some(id) if results.is_some_and(|r| r.with(|r| r.contains(&id))) => view! {
            <span class="picker__row-main">
                <span class="picker__row-title">{id}</span>
            </span>
        }
        .into_any(),
        Some(id) => match row.get() {
            Some(r) => view! {
                {(!compact)
                    .then(|| view! { <span class="picker__icon" inner_html=r.icon></span> })}
                <span class="picker__row-main">
                    <span class="picker__row-title">{r.title}</span>
                    <RowSub device=r.device room=r.room />
                </span>
                {(!compact).then(|| view! { <RowSide id=id /> })}
            }
            .into_any(),
            None => view! {
                <span class="picker__row-main">
                    <span class="picker__row-title">{id}</span>
                    <span class="picker__row-sub">
                        <span class="picker__row-off">{move || t(PickerText::Missing)}</span>
                    </span>
                </span>
            }
            .into_any(),
        },
    };
    view! {
        <div class=class>
            <span class="picker-field__label">{move || label.get()}</span>
            <button
                type="button"
                class="picker-field"
                class:picker-field--compact=compact
                aria-haspopup="dialog"
                on:click=move |_| open.set(true)
            >
                {closed}
                <span class="picker-field__chevron" aria-hidden="true">
                    "▾"
                </span>
            </button>
            <EntityPicker
                open=open
                title=label
                mode=Mode::Single
                current=current
                pool=pool
                suggest=suggest
                pinned=pinned.map(|text| Pinned { text })
                results=results
                on_pick=Callback::new(move |id| on_change.run(id))
            />
        </div>
    }
}

/// `ids` with `id` added when absent, removed when present.
fn toggled(ids: &[String], id: &str) -> Vec<String> {
    if ids.iter().any(|i| i == id) {
        ids.iter().filter(|i| *i != id).cloned().collect()
    } else {
        ids.iter().cloned().chain([id.to_owned()]).collect()
    }
}

/// A removable chip: name and device. Tapping it removes the entity.
#[component]
fn PickChip(id: String, on_remove: Callback<String>) -> impl IntoView {
    let t = use_picker_text();
    let row = use_row(Signal::stored(Some(id.clone())));
    let remove = id.clone();
    let name_id = id.clone();
    view! {
        <button
            type="button"
            class="chip picker-chip"
            aria-label=move || {
                let name = row.get().map_or_else(|| name_id.clone(), |r| r.title);
                format!("{}: {name}", t(PickerText::Remove))
            }
            on:click=move |_| on_remove.run(remove.clone())
        >
            {move || match row.get() {
                Some(r) => {
                    view! {
                        <span>{r.title}</span>
                        <RowSub device=r.device room=None />
                    }
                        .into_any()
                }
                None => {
                    view! {
                        <span>{id.clone()}</span>
                        <span class="picker__row-sub">
                            <span class="picker__row-off">{move || t(PickerText::Missing)}</span>
                        </span>
                    }
                        .into_any()
                }
            }} <span aria-hidden="true">"✕"</span>
        </button>
    }
}

/// A list of entities: removable chips (name + device), and an [`EntityAddButton`].
#[component]
pub fn EntityPickListField(
    #[prop(into)] label: Signal<String>,
    #[prop(into)] value: Signal<Vec<String>>,
    on_change: Callback<Vec<String>>,
    #[prop(optional_no_strip)] pool: Option<Keep>,
    #[prop(optional_no_strip)] suggest: Option<Suggest>,
    #[prop(optional_no_strip)] class: Option<String>,
) -> impl IntoView {
    let toggle = Callback::new(move |id: String| {
        on_change.run(value.with_untracked(|v| toggled(v, &id)));
    });
    view! {
        <div class=class>
            <span class="picker-field__label">{move || label.get()}</span>
            <div class="picker-chips">
                <For
                    each=move || value.get()
                    key=|id| id.clone()
                    children=move |id| view! { <PickChip id=id on_remove=toggle /> }
                />
                <EntityAddButton
                    label=label
                    taken=value
                    on_toggle=toggle
                    pool=pool
                    suggest=suggest
                />
            </div>
        </div>
    }
}

/// A "+ Add…" button that opens the picker in Multi mode over `taken`; each tick reports the id.
#[component]
pub fn EntityAddButton(
    #[prop(into)] label: Signal<String>,
    #[prop(into)] taken: Signal<Vec<String>>,
    on_toggle: Callback<String>,
    #[prop(optional_no_strip)] pool: Option<Keep>,
    #[prop(optional_no_strip)] suggest: Option<Suggest>,
) -> impl IntoView {
    let open = RwSignal::new(false);
    view! {
        <button
            type="button"
            class="chip chip--add"
            aria-haspopup="dialog"
            on:click=move |_| open.set(true)
        >
            {move || format!("+ {}", label.get())}
        </button>
        <EntityPicker
            open=open
            title=label
            mode=Mode::Multi
            current=taken
            pool=pool
            suggest=suggest
            on_pick=Callback::new(move |id: Option<String>| {
                if let Some(id) = id {
                    on_toggle.run(id);
                }
            })
        />
    }
}

#[cfg(test)]
mod tests {
    use super::toggled;

    #[test]
    fn toggled_adds_then_removes() {
        let a = vec!["a".to_string()];
        assert_eq!(toggled(&a, "b"), ["a", "b"]);
        assert_eq!(toggled(&["a".into(), "b".into()], "a"), ["b"]);
    }
}
