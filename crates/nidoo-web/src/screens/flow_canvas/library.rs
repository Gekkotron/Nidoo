//! The card library in the side panel while editing: a search box over the cards a flow can
//! gain, grouped by role, each row shaped like the card it adds.

use leptos::ev;
use leptos::prelude::*;
use nidoo_core::flow::NodeKind;

use super::Editor;
use crate::flow_picker::{Entry, Group, entries};
use crate::flow_text::{Ctx, Role, role};
use crate::house::House;
use crate::i18n::flows::{FlowText, flow_text};
use crate::icons::node_icon;

/// The card a library row is being dragged to the canvas as, while a drag is on. Provided by
/// the canvas, which adds it where the row is dropped.
#[derive(Clone, Copy)]
pub struct Dragging(pub StoredValue<Option<NodeKind>>);

/// The id of the card just added from the library, which glows for a moment so it can be
/// found on the canvas. Provided by the canvas.
#[derive(Clone, Copy)]
pub struct Fresh(pub RwSignal<Option<String>>);

fn header(lang: crate::i18n::Lang, group: Group) -> &'static str {
    flow_text(
        lang,
        match group {
            Group::When => FlowText::SectionWhen,
            Group::And => FlowText::SectionAnd,
            Group::Then => FlowText::SectionThen,
            Group::Wait => FlowText::SectionWait,
            Group::Flow => FlowText::SectionFlow,
        },
    )
}

/// The role colour of a section header.
fn header_role(group: Group) -> Role {
    match group {
        Group::When => Role::Trigger,
        Group::And => Role::Check,
        Group::Then | Group::Flow => Role::Action,
        Group::Wait => Role::Wait,
    }
}

fn role_suffix(role: Role) -> &'static str {
    match role {
        Role::Trigger => "trigger",
        Role::Check => "check",
        Role::Action => "action",
        Role::Wait => "wait",
    }
}

/// The library. Clicking a row, or Enter on the highlighted one while the search box has the
/// focus, calls `on_add`; the library stays as it is so several cards can go in a row. A row
/// can also be dragged onto the canvas (see [`Dragging`]). Esc in the search box clears it.
/// Mod-Z in the search box undoes the flow (Shift-Mod-Z redoes) from an Enter until the next
/// keystroke; otherwise it is the text's own undo. `query` belongs to the canvas, so the search
/// survives the panel swapping to a card's settings and back.
#[component]
pub fn Library(on_add: Callback<NodeKind>, query: RwSignal<String>) -> impl IntoView {
    let house = expect_context::<House>();
    let lang = crate::i18n::use_lang();
    let Dragging(dragging) = expect_context::<Dragging>();
    let editor = expect_context::<Editor>();
    let highlight = RwSignal::new(0_usize);
    // Set by an Enter add, cleared by typing: whether Mod-Z belongs to the flow.
    let added = StoredValue::new(false);

    let results = Memo::new(move |_| {
        let query = query.get();
        let names: Vec<(String, String)> = house.flows.with(|flows| {
            flows
                .iter()
                .map(|f| (f.definition.id.clone(), f.definition.name.clone()))
                .collect()
        });
        house.entities.with(|entities| {
            house.areas.with(|areas| {
                entries(
                    &query,
                    &Ctx {
                        entities,
                        areas,
                        flows: &names,
                        lang: lang(),
                    },
                )
            })
        })
    });

    // A new query starts again from the top.
    Effect::watch(move || query.get(), move |_, _, _| highlight.set(0), false);

    let add = move |entry: Entry| on_add.run(entry.kind);
    let on_key = move |ev: ev::KeyboardEvent| {
        let last = results.with_untracked(Vec::len).saturating_sub(1);
        let modifier = ev.meta_key() || ev.ctrl_key();
        // Key names compared in lower case, so they read as ids, not as words.
        match ev.key().to_ascii_lowercase().as_str() {
            "z" if modifier && added.get_value() => {
                ev.prevent_default();
                editor.step(ev.shift_key());
            }
            "arrowdown" => {
                ev.prevent_default();
                highlight.update(|i| *i = (*i + 1).min(last));
            }
            "arrowup" => {
                ev.prevent_default();
                highlight.update(|i| *i = i.saturating_sub(1));
            }
            "enter" => {
                ev.prevent_default();
                if let Some(entry) =
                    results.with_untracked(|r| r.get(highlight.get_untracked()).cloned())
                {
                    add(entry);
                    added.set_value(true);
                }
            }
            "escape" => query.set(String::new()),
            _ => {}
        }
    };

    let list = move || {
        let mut last = None;
        results
            .get()
            .into_iter()
            .enumerate()
            .map(|(i, entry)| {
                let head = (last != Some(entry.group)).then(|| {
                    let class = format!(
                        "flow-library__header flow-library__header--{}",
                        role_suffix(header_role(entry.group))
                    );
                    view! { <li class=class role="presentation">{move || header(lang(), entry.group)}</li> }
                });
                last = Some(entry.group);
                let class = format!(
                    "flow-library__item flow-library__item--{}",
                    role_suffix(role(&entry.kind))
                );
                let icon = node_icon(&entry.kind);
                let label = entry.label.clone();
                let sub = entry.sub.clone();
                let drag = entry.clone();
                let start_drag = move |ev: ev::DragEvent| {
                    dragging.set_value(Some(drag.kind.clone()));
                    if let Some(data) = ev.data_transfer() {
                        // Some browsers start no drag without data.
                        let _ = data.set_data("text/plain", &drag.label);
                        data.set_effect_allowed("copy");
                    }
                };
                view! {
                    {head}
                    <li
                        class=class
                        class:flow-library__item--on=move || highlight.get() == i
                        role="option"
                        draggable="true"
                        aria-selected=move || (highlight.get() == i).to_string()
                        on:dragstart=start_drag
                        on:dragend=move |_| dragging.set_value(None)
                        on:pointermove=move |_| highlight.set(i)
                        on:click=move |_| add(entry.clone())
                    >
                        <span class="flow-library__icon" inner_html=icon></span>
                        <span class="flow-library__text">
                            <span class="flow-library__title">{label}</span>
                            {sub.map(|s| view! { <span class="flow-library__sub">{s}</span> })}
                        </span>
                        <span class="flow-library__grip" aria-hidden="true">"⋮⋮"</span>
                    </li>
                }
            })
            .collect_view()
    };

    view! {
        <section class="flow-library" aria-label=move || flow_text(lang(), FlowText::AddCard)>
            <header class="flow-drawer__header">
                <h2 class="flow-drawer__title">{move || flow_text(lang(), FlowText::AddCard)}</h2>
            </header>
            <input
                class="field flow-library__search"
                type="search"
                placeholder=move || flow_text(lang(), FlowText::SearchCards)
                aria-label=move || flow_text(lang(), FlowText::SearchCardsLabel)
                prop:value=move || query.get()
                on:input=move |ev| {
                    added.set_value(false);
                    query.set(event_target_value(&ev));
                }
                on:keydown=on_key
            />
            <ul class="flow-library__list" role="listbox">
                {list}
                <Show when=move || results.with(Vec::is_empty)>
                    <li class="flow-library__empty">
                        {move || flow_text(lang(), FlowText::NothingMatches)}
                    </li>
                </Show>
            </ul>
        </section>
    }
}
