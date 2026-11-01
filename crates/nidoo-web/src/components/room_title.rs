//! The Home room heading. Its pencil opens the room's edit mode (see `RoomSection`): the name
//! turns into a field, saved with the card order by one Save.

use leptos::prelude::*;

use crate::i18n::devices::{DeviceText, use_device_text};
use crate::icons;

/// The heading's edit mode, owned by the room section, which also arranges the cards.
#[derive(Clone, Copy)]
pub struct RoomEdit {
    pub editing: RwSignal<bool>,
    pub busy: RwSignal<bool>,
    /// The name being typed.
    pub draft: RwSignal<String>,
    /// Opens the edit mode with the room's current name and card order.
    pub start: Callback<()>,
    pub save: Callback<()>,
    pub cancel: Callback<()>,
}

/// The room heading. `id` is `None` for a chip that is not a room (Favorites, Unassigned): it
/// has no name field. `edit` is `None` where nothing can be edited (Favorites): no pencil.
#[component]
pub fn RoomTitle(
    #[prop(into)] id: Signal<Option<String>>,
    #[prop(into)] name: Signal<String>,
    edit: Option<RoomEdit>,
) -> impl IntoView {
    let t = use_device_text();
    let input = NodeRef::<leptos::html::Input>::new();
    let editing = move || edit.is_some_and(|e| e.editing.get());

    Effect::new(move |_| {
        if editing()
            && let Some(input) = input.get()
        {
            let _ = input.focus();
            input.select();
        }
    });

    let start = move |_| {
        if let Some(e) = edit {
            e.start.run(());
        }
    };
    let busy = move || edit.is_some_and(|e| e.busy.get());
    let cancel = move || {
        if let Some(e) = edit {
            e.cancel.run(());
        }
    };

    view! {
        <Show
            when=editing
            fallback=move || {
                view! {
                    <div class="room__heading">
                        <h1 class="room__title">{move || name.get()}</h1>
                        <Show when=move || edit.is_some()>
                            <button
                                type="button"
                                class="btn btn--round btn--icon room__rename"
                                aria-label=move || t(DeviceText::EditRoom)
                                title=move || t(DeviceText::EditRoom)
                                on:click=start
                                inner_html=icons::PENCIL
                            ></button>
                        </Show>
                    </div>
                }
            }
        >
            <form
                class="room__heading room__form"
                on:submit=move |ev| {
                    ev.prevent_default();
                    if let Some(e) = edit {
                        e.save.run(());
                    }
                }
            >
                <Show
                    when=move || id.with(Option::is_some)
                    fallback=move || view! { <h1 class="room__title">{move || name.get()}</h1> }
                >
                    <input
                        node_ref=input
                        class="field room__field"
                        type="text"
                        aria-label=move || t(DeviceText::RoomName)
                        prop:value=move || edit.map(|e| e.draft.get()).unwrap_or_default()
                        prop:disabled=busy
                        on:input=move |ev| {
                            if let Some(e) = edit {
                                e.draft.set(event_target_value(&ev));
                            }
                        }
                        on:keydown=move |ev| {
                            if ev.key() == "Escape" {
                                cancel();
                            }
                        }
                    />
                </Show>
                <span class="room__hint">{move || t(DeviceText::DragToReorder)}</span>
                <button type="submit" class="btn room__action" disabled=busy>
                    {move || t(DeviceText::Save)}
                </button>
                <button type="button" class="btn room__action" disabled=busy on:click=move |_| cancel()>
                    {move || t(DeviceText::Cancel)}
                </button>
            </form>
        </Show>
    }
}
