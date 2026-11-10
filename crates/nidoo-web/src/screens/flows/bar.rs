//! The top bar of the Flows page: search, sort, grouping and view toggle, then the status chips
//! and the trigger menu.

use leptos::prelude::*;

use crate::flow_list::{
    FlowListPrefs, Grouping, Sort, StatusCounts, StatusFilter, TriggerKind, ViewMode,
};
use crate::i18n::flows::{FlowText, use_flow_text};

use super::room_menu::OpenMenu;

/// The [`OpenMenu`] id of the trigger menu (flow ids never start with `_`).
const TRIGGER_MENU: &str = "__trigger_menu__";

/// A small segmented control: one button per option, the current one pressed.
#[component]
fn Segmented<T>(
    label: FlowText,
    options: Vec<(T, FlowText)>,
    current: Signal<T>,
    on_pick: Callback<T>,
) -> impl IntoView
where
    T: Copy + PartialEq + Send + Sync + 'static,
{
    let t = use_flow_text();
    let buttons = options
        .into_iter()
        .map(|(value, word)| {
            view! {
                <button
                    type="button"
                    class="seg__btn"
                    class:seg__btn--on=move || current.get() == value
                    aria-pressed=move || (current.get() == value).to_string()
                    on:click=move |_| on_pick.run(value)
                >
                    {move || t(word)}
                </button>
            }
        })
        .collect_view();
    view! {
        <div class="seg" role="group" aria-label=move || t(label)>
            {buttons}
        </div>
    }
}

const TRIGGER_CHIPS: [(TriggerKind, FlowText); 7] = [
    (TriggerKind::State, FlowText::FilterDeviceState),
    (TriggerKind::Schedule, FlowText::Schedule),
    (TriggerKind::Sun, FlowText::NodeSun),
    (TriggerKind::OffPeak, FlowText::NodeOffPeak),
    (TriggerKind::Silent, FlowText::NodeSilent),
    (TriggerKind::Signal, FlowText::FilterButtonEvent),
    (TriggerKind::Manual, FlowText::FilterManual),
];

#[component]
pub fn FlowsBar(
    prefs: RwSignal<FlowListPrefs>,
    query: RwSignal<String>,
    counts: Signal<StatusCounts>,
) -> impl IntoView {
    let t = use_flow_text();
    let status_chip =
        move |status: StatusFilter, label: FlowText, count: fn(&StatusCounts) -> usize| {
            view! {
                <button
                    type="button"
                    class="chip"
                    class:chip--selected=move || prefs.with(|p| p.status == status)
                    aria-pressed=move || prefs.with(|p| p.status == status).to_string()
                    on:click=move |_| prefs.update(|p| p.status = status)
                >
                    {move || t(label)}
                    <span class="chip-badge">{move || counts.with(count)}</span>
                </button>
            }
        };
    view! {
        <div class="flow-bar">
            <input
                type="search"
                class="flow-bar__search"
                placeholder=move || t(FlowText::SearchFlows)
                aria-label=move || t(FlowText::SearchFlowsLabel)
                prop:value=move || query.get()
                on:input=move |ev| query.set(event_target_value(&ev))
            />
            <span class="flow-bar__label">{move || t(FlowText::Sort)}</span>
            <Segmented
                label=FlowText::Sort
                options=vec![(Sort::Name, FlowText::SortName), (Sort::LastRun, FlowText::SortLastRun)]
                current=Signal::derive(move || prefs.with(|p| p.sort))
                on_pick=Callback::new(move |v| prefs.update(|p| p.sort = v))
            />
            <span class="flow-bar__label">{move || t(FlowText::GroupBy)}</span>
            <Segmented
                label=FlowText::GroupBy
                options=vec![
                    (Grouping::Room, FlowText::GroupRoom),
                    (Grouping::Label, FlowText::GroupLabel),
                    (Grouping::None, FlowText::GroupNone),
                ]
                current=Signal::derive(move || prefs.with(|p| p.grouping))
                on_pick=Callback::new(move |v| prefs.update(|p| p.grouping = v))
            />
            <Segmented
                label=FlowText::ViewLabel
                options=vec![(ViewMode::List, FlowText::ViewList), (ViewMode::Cards, FlowText::ViewCards)]
                current=Signal::derive(move || prefs.with(|p| p.view))
                on_pick=Callback::new(move |v| prefs.update(|p| p.view = v))
            />
        </div>
        <div class="flow-chips">
            <div class="flow-chips__set" role="group" aria-label=move || t(FlowText::StatusLabel)>
                {status_chip(StatusFilter::All, FlowText::StatusAll, |c| c.all)}
                {status_chip(StatusFilter::On, FlowText::StatusOn, |c| c.on)}
                {status_chip(StatusFilter::Off, FlowText::StatusOff, |c| c.off)}
                {status_chip(StatusFilter::Attention, FlowText::NeedsAttention, |c| c.attention)}
            </div>
            <TriggerMenu prefs=prefs />
        </div>
    }
}

/// "Trigger ▾" (with "· n" when n kinds are picked) over a popover of the seven trigger kinds as
/// checkboxes; picking none shows every flow. It shares the page's [`OpenMenu`], so Escape or a
/// press outside closes it like a row's ⋯ menu.
#[component]
fn TriggerMenu(prefs: RwSignal<FlowListPrefs>) -> impl IntoView {
    let t = use_flow_text();
    let OpenMenu(open_menu) = expect_context::<OpenMenu>();
    let open = Memo::new(move |_| open_menu.with(|o| o.as_deref() == Some(TRIGGER_MENU)));
    let toggle = move |_| {
        open_menu.update(|o| {
            *o = if o.as_deref() == Some(TRIGGER_MENU) {
                None
            } else {
                Some(TRIGGER_MENU.to_owned())
            };
        });
    };
    let picked = Memo::new(move |_| prefs.with(|p| p.triggers.len()));
    let selected = Signal::derive(move || picked.get() > 0);
    let label = move || match picked.get() {
        0 => format!("{} ▾", t(FlowText::TriggerMenu)),
        n => format!("{} · {n} ▾", t(FlowText::TriggerMenu)),
    };
    let items = move || {
        TRIGGER_CHIPS
            .into_iter()
            .map(|(kind, word)| {
                let on = move || prefs.with(|p| p.triggers.contains(&kind));
                view! {
                    <button
                        type="button"
                        role="menuitemcheckbox"
                        class="room-menu__item"
                        aria-checked=move || on().to_string()
                        on:click=move |_| {
                            prefs.update(|p| {
                                if !p.triggers.remove(&kind) {
                                    p.triggers.insert(kind);
                                }
                            });
                        }
                    >
                        <span class="room-menu__check" aria-hidden="true">
                            {move || on().then_some("✓")}
                        </span>
                        {move || t(word)}
                    </button>
                }
            })
            .collect_view()
    };
    view! {
        <span class="room-menu trigger-menu">
            <button
                type="button"
                class="chip trigger-menu__button"
                class:chip--selected=selected
                aria-haspopup="menu"
                aria-expanded=move || open.get().to_string()
                on:click=toggle
            >
                {label}
            </button>
            <Show when=move || open.get()>
                <div class="room-menu__pop" role="menu" aria-label=move || t(FlowText::TriggerType)>
                    {items}
                </div>
            </Show>
        </span>
    }
}
