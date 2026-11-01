//! The top bar of the Flows page: search, sort, grouping, view toggle and the filter chips.

use leptos::prelude::*;

use crate::flow_list::{
    FlowListPrefs, Grouping, Sort, StatusCounts, StatusFilter, TriggerKind, ViewMode,
};
use crate::i18n::flows::{FlowText, use_flow_text};

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
    let trigger_chips = TRIGGER_CHIPS
        .into_iter()
        .map(|(kind, label)| {
            let on = move || prefs.with(|p| p.triggers.contains(&kind));
            view! {
                <button
                    type="button"
                    class="chip"
                    class:chip--selected=on
                    aria-pressed=move || on().to_string()
                    on:click=move |_| {
                        prefs.update(|p| {
                            if !p.triggers.remove(&kind) {
                                p.triggers.insert(kind);
                            }
                        });
                    }
                >
                    {move || t(label)}
                </button>
            }
        })
        .collect_view();
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
                options=vec![(Grouping::Room, FlowText::GroupRoom), (Grouping::None, FlowText::GroupNone)]
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
            <div
                class="flow-chips__set"
                role="group"
                aria-label=move || t(FlowText::TriggerType)
            >
                {trigger_chips}
            </div>
        </div>
    }
}
