//! The Flows page: a top bar (search, sort, grouping, view, filters) over the flows as a list or
//! cards, grouped in foldable room or label sections. A choice's section header picks its
//! label, a standalone label's turns all its flows on or off.

mod bar;
mod card;
mod room_menu;
mod row;

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use nidoo_core::flow::LabelAction;

use crate::components::toast::Toasts;
use crate::flow_list::{
    self, Filters, FlowListPrefs, Grouping, ListRow, Section, SectionHead, ViewMode, decode_prefs,
    encode_prefs,
};
use crate::house::House;
use crate::i18n::flows::{FlowText, flow_text, label_switch_result};
use crate::i18n::{Key, Lang, text, use_lang};
use bar::FlowsBar;
use card::FlowCard;
use room_menu::{RoomMenu, provide_open_menu};
use row::FlowRow;

/// Storage key for the remembered view, grouping, sort, filters and folded sections.
#[cfg(target_arch = "wasm32")]
const STORAGE_KEY: &str = "nidoo.flowList";

/// `localStorage`, or `None` when it is missing, blocked or throws.
#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

fn load_prefs() -> FlowListPrefs {
    #[cfg(target_arch = "wasm32")]
    {
        storage()
            .and_then(|s| s.get_item(STORAGE_KEY).ok().flatten())
            .map(|raw| decode_prefs(&raw))
            .unwrap_or_default()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = decode_prefs;
        FlowListPrefs::default()
    }
}

#[cfg_attr(not(target_arch = "wasm32"), allow(unused_variables))]
fn save_prefs(prefs: &FlowListPrefs) {
    #[cfg(target_arch = "wasm32")]
    if let Some(s) = storage() {
        let _ = s.set_item(STORAGE_KEY, &encode_prefs(prefs));
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = encode_prefs;
}

/// The ⋯ menu of a row: open the flow, move it to a room, or delete it.
fn menu_for(row: &ListRow) -> AnyView {
    let id = row.view.definition.id.clone();
    let name = row.view.definition.name.clone();
    let href = format!("/flows/{}", crate::api::encode_id(&id));
    view! {
        <RoomMenu
            id=id
            name=name
            href=href
            current=row.room.clone()
            invalid=row.view.invalid.is_some()
        />
    }
    .into_any()
}

/// The rows of one block, as a list panel or a card grid. Keyed by [`flow_list::row_key`], so a
/// change rebuilds only the rows it touched.
fn block(rows: Signal<Vec<ListRow>>, mode: ViewMode) -> AnyView {
    match mode {
        ViewMode::List => view! {
            <ul class="flow-list">
                <For each=move || rows.get() key=flow_list::row_key let:row>
                    {
                        let menu = menu_for(&row);
                        view! { <FlowRow row=row menu=menu /> }
                    }
                </For>
            </ul>
        }
        .into_any(),
        ViewMode::Cards => view! {
            <ul class="flow-grid">
                <For each=move || rows.get() key=flow_list::row_key let:row>
                    {
                        let menu = menu_for(&row);
                        view! { <FlowCard row=row menu=menu /> }
                    }
                </For>
            </ul>
        }
        .into_any(),
    }
}

/// Switches `label` on the hub, then says what changed in a toast (or why it failed). The flows
/// and the catalog follow over the WebSocket.
fn switch_label(house: House, toasts: Toasts, lang: Lang, label: String, action: LabelAction) {
    spawn_local(async move {
        match crate::api::flow_labels::switch_flow_label(&label, action).await {
            Ok(result) => toasts.push(
                house
                    .flow_labels
                    .with_untracked(|v| label_switch_result(lang, &result, &v.catalog)),
            ),
            Err(e) => toasts.push(e.error),
        }
    });
}

/// The controls at the end of a section header: a choice's label picker (the current label
/// pressed; "Needs two labels" below two), a label's Turn all on / off, nothing for a room or
/// "No label".
fn head_controls(head: SectionHead) -> AnyView {
    let house = expect_context::<House>();
    let toasts = expect_context::<Toasts>();
    let lang = use_lang();
    let switch = move |id: &str, action: LabelAction| {
        let id = id.to_owned();
        move |_| switch_label(house, toasts, lang(), id.clone(), action)
    };
    match head {
        SectionHead::Choice { members, .. } if members.len() < 2 => view! {
            <span class="label-pick__incomplete">
                {move || flow_text(lang(), FlowText::ChoiceIncomplete)}
            </span>
        }
        .into_any(),
        SectionHead::Choice { choice, members } => {
            let buttons = members
                .into_iter()
                .map(|label| {
                    let current = choice.current.as_deref() == Some(label.id.as_str());
                    view! {
                        <button
                            type="button"
                            class="seg__btn"
                            class:seg__btn--on=current
                            aria-pressed=current.to_string()
                            on:click=switch(&label.id, LabelAction::Pick)
                        >
                            {label.name}
                        </button>
                    }
                })
                .collect_view();
            view! {
                <div class="seg label-pick" role="group" aria-label=choice.name>
                    {buttons}
                </div>
            }
            .into_any()
        }
        SectionHead::Label(label) => view! {
            <div class="label-switch">
                <button
                    type="button"
                    class="chip label-switch__btn"
                    on:click=switch(&label.id, LabelAction::On)
                >
                    {move || flow_text(lang(), FlowText::AllOn)}
                </button>
                <button
                    type="button"
                    class="chip label-switch__btn"
                    on:click=switch(&label.id, LabelAction::Off)
                >
                    {move || flow_text(lang(), FlowText::AllOff)}
                </button>
            </div>
        }
        .into_any(),
        SectionHead::Plain | SectionHead::NoLabel => ().into_any(),
    }
}

/// A foldable section: a header (fold button with title · count and ⚠, then the label
/// controls) over its block. It is keyed by
/// the room and reads its title, rows and attention from `sections`, so a run event updates it
/// in place.
fn section_view(
    key: String,
    sections: Memo<Vec<Section>>,
    mode: ViewMode,
    prefs: RwSignal<FlowListPrefs>,
) -> impl IntoView {
    let lang = use_lang();
    let section = {
        let key = key.clone();
        Memo::new(move |_| sections.with(|all| all.iter().find(|s| s.key == key).cloned()))
    };
    let rows = Signal::derive(move || {
        section
            .with(|s| s.as_ref().map(|s| s.rows.clone()))
            .unwrap_or_default()
    });
    let title = move || {
        section
            .with(|s| s.as_ref().map(|s| s.title.to_uppercase()))
            .unwrap_or_default()
    };
    let count = move || section.with(|s| s.as_ref().map_or(0, |s| s.rows.len()));
    let attention = move || section.with(|s| s.as_ref().is_some_and(|s| s.attention));
    let folded = {
        let key = key.clone();
        Memo::new(move |_| prefs.with(|p| p.folded.contains(&key)))
    };
    let toggle = move |_| {
        prefs.update(|p| {
            if !p.folded.remove(&key) {
                p.folded.insert(key.clone());
            }
        });
    };
    let head = Memo::new(move |_| {
        section
            .with(|s| s.as_ref().map(|s| s.head.clone()))
            .unwrap_or(SectionHead::Plain)
    });
    view! {
        <section class="flow-section">
            <div class="flow-section__head">
                <button
                    type="button"
                    class="flow-section__toggle"
                    aria-expanded=move || (!folded.get()).to_string()
                    on:click=toggle
                >
                    <span class="flow-section__caret" class:flow-section__caret--folded=folded>
                        "▾"
                    </span>
                    <span class="flow-section__title">{title}</span>
                    <span class="flow-section__dot" aria-hidden="true">"·"</span>
                    <span class="flow-section__count">{count}</span>
                    {move || {
                        (attention() && folded.get())
                            .then(|| {
                                view! {
                                    <span
                                        class="flow-section__warn"
                                        title=move || flow_text(lang(), FlowText::RoomNeedsAttention)
                                    >
                                        "⚠"
                                    </span>
                                }
                            })
                    }}
                </button>
                {move || head_controls(head.get())}
            </div>
            <Show when=move || !folded.get()>{move || block(rows, mode)}</Show>
        </section>
    }
}

#[component]
pub fn FlowsScreen() -> impl IntoView {
    let house = expect_context::<House>();
    provide_open_menu();
    let prefs = RwSignal::new(load_prefs());
    // The search text is never remembered.
    let query = RwSignal::new(String::new());
    Effect::new(move |_| prefs.with(save_prefs));

    let lang = use_lang();
    let all_rows = Memo::new(move |_| {
        let lang = lang();
        house.flows.with(|flows| {
            house.areas.with(|areas| {
                house.entities.with(|entities| {
                    house.flow_labels.with(|labels| {
                        flow_list::rows(lang, flows, areas, entities, &labels.catalog)
                    })
                })
            })
        })
    });
    let counts = Signal::derive(move || {
        all_rows.with(|rows| {
            query.with(|q| prefs.with(|p| flow_list::status_counts(rows, q, &p.triggers)))
        })
    });
    let shown = Memo::new(move |_| {
        let filters = Filters {
            query: query.get(),
            status: prefs.with(|p| p.status),
            triggers: prefs.with(|p| p.triggers.clone()),
        };
        let mut rows = all_rows.with(|rows| flow_list::filter(rows, &filters));
        flow_list::sort_rows(&mut rows, prefs.with(|p| p.sort));
        rows
    });
    let clear_filters = move |_| {
        query.set(String::new());
        prefs.update(|p| {
            p.status = flow_list::StatusFilter::All;
            p.triggers.clear();
        });
    };
    // `body` reads only the layout and emptiness, so a run event or a fold never rebuilds the
    // list: the keyed sections and rows below update in place.
    let layout = Memo::new(move |_| prefs.with(|p| (p.view, p.grouping)));
    let empty = Memo::new(move |_| shown.with(Vec::is_empty));
    let sections = Memo::new(move |_| {
        let rows = shown.get();
        let lang = lang();
        if prefs.with(|p| p.grouping) == Grouping::Label {
            house
                .flow_labels
                .with(|labels| flow_list::label_sections(lang, rows, &labels.catalog))
        } else {
            house
                .areas
                .with(|areas| flow_list::sections(lang, rows, areas))
        }
    });
    let section_keys = Memo::new(move |_| {
        sections.with(|all| all.iter().map(|s| s.key.clone()).collect::<Vec<_>>())
    });
    let body = move || {
        let (mode, grouping) = layout.get();
        if empty.get() {
            return view! {
                <div class="flow-list__empty">
                    <p>{move || flow_text(lang(), FlowText::NoFlowsMatch)}</p>
                    <button type="button" class="btn" on:click=clear_filters>
                        {move || flow_text(lang(), FlowText::ClearFilters)}
                    </button>
                </div>
            }
            .into_any();
        }
        match grouping {
            Grouping::None => block(shown.into(), mode),
            Grouping::Room | Grouping::Label => view! {
                <For each=move || section_keys.get() key=|key| key.clone() let:key>
                    {section_view(key, sections, mode, prefs)}
                </For>
            }
            .into_any(),
        }
    };
    view! {
        <div class="flows">
            <header class="sheet__header">
                <div class="sheet__titles">
                    <h1 class="sheet__title">{move || flow_text(lang(), FlowText::PageTitle)}</h1>
                </div>
                <A href="/flows/new" attr:class="btn flows-new">
                    {move || text(lang(), Key::NewFlow)}
                </A>
            </header>
            <Show
                when=move || house.flows.with(|f| !f.is_empty())
                fallback=move || {
                    view! {
                        <p class="flow-list__empty">
                            {move || flow_text(lang(), FlowText::NoFlowsYet)}
                        </p>
                    }
                }
            >
                <FlowsBar prefs=prefs query=query counts=counts />
                {body}
            </Show>
        </div>
    }
}
