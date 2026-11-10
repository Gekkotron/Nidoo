//! One flow as a list row: icon, name, trigger, last run, switch and a slot for the ⋯ menu.

use leptos::prelude::*;
use leptos_router::components::A;
use nidoo_api::{FlowView, RunStatus};
use nidoo_core::flow::FlowMode;

use crate::api;
use crate::components::flow_switch::FlowSwitch;
use crate::components::now::Now;
use crate::flow_list::{ListRow, label_color_name};
use crate::flows::status_word;
use crate::i18n::flows::{FlowText, flow_text};
use crate::i18n::format::{ago, date_time, local_date_time};
use crate::i18n::{Lang, use_lang};
use crate::icons::trigger_icon;

/// The tooltip of a run time: the exact date and time, in the language and the local zone.
pub(super) fn run_title(lang: Lang, secs: u64) -> String {
    date_time(lang, local_date_time(secs))
}

/// The last run as "Done · 2 min ago", or "No runs yet".
pub(super) fn last_run(lang: Lang, flow: &FlowView, now: u64) -> String {
    flow.runs.first().map_or_else(
        || flow_text(lang, FlowText::NoRunsYet).to_owned(),
        |run| {
            format!(
                "{} · {}",
                status_word(lang, run.status),
                ago(lang, now.saturating_sub(run.started_at))
            )
        },
    )
}

/// Whether the newest finished run failed (running and skipped ones do not count).
pub(super) fn last_run_failed(flow: &FlowView) -> bool {
    flow.runs
        .iter()
        .find(|r| matches!(r.status, RunStatus::Done | RunStatus::Failed))
        .is_some_and(|r| r.status == RunStatus::Failed)
}

/// The icon cell: the first trigger kind's glyph, or ⚠ for an invalid flow.
pub(super) fn icon_view(row: &ListRow) -> impl IntoView + use<> {
    match row.kinds.iter().next() {
        Some(kind) => {
            view! { <span class="flow-icon" inner_html=trigger_icon(*kind)></span> }.into_any()
        }
        None => view! { <span class="flow-icon flow-icon--warn">"⚠"</span> }.into_any(),
    }
}

pub(super) fn is_restart(flow: &FlowView) -> bool {
    flow.definition.mode == FlowMode::Restart
}

/// The red "Invalid" tag with the reason as tooltip, when the flow is invalid.
pub(super) fn invalid_badge(flow: &FlowView) -> Option<impl IntoView + use<>> {
    let lang = use_lang();
    flow.invalid.clone().map(|reason| {
        view! {
            <span class="badge badge--invalid" title=reason>
                {move || flow_text(lang(), FlowText::Invalid)}
            </span>
        }
    })
}

/// The row's labels as coloured chips, in catalog order; nothing when it carries none.
pub(super) fn label_chips(row: &ListRow) -> impl IntoView + use<> {
    row.labels
        .iter()
        .map(|label| {
            let class = format!("label-chip label-chip--{}", label_color_name(label.color));
            view! { <span class=class>{label.name.clone()}</span> }
        })
        .collect_view()
}

/// A row of the list view. `menu` is the slot at the row end for the ⋯ menu.
#[component]
pub fn FlowRow(row: ListRow, #[prop(optional)] menu: Option<AnyView>) -> impl IntoView {
    let now = expect_context::<Now>();
    let lang = use_lang();
    let flow = row.view.clone();
    let id = flow.definition.id.clone();
    let href = format!("/flows/{}", api::encode_id(&id));
    let name = flow.definition.name.clone();
    let enabled = flow.definition.enabled;
    let invalid = flow.invalid.is_some();
    let started = flow.runs.first().map(|r| r.started_at);
    let failed = last_run_failed(&flow);
    let restart = is_restart(&flow);
    let badge = invalid_badge(&flow);
    let icon = icon_view(&row);
    let trigger = row.trigger_text.clone();
    let name_for_switch = name.clone();
    let run_flow = flow.clone();
    view! {
        <li class="flow-row" class:flow-row--off=!enabled class:flow-row--invalid=invalid>
            {icon}
            <span class="flow-row__name">
                <A href=href attr:class="flow-row__link">{name}</A>
                {restart
                    .then(|| {
                        view! {
                            <span class="flow-tag">
                                {move || flow_text(lang(), FlowText::ModeRestart)}
                            </span>
                        }
                    })}
                {badge}
                {label_chips(&row)}
            </span>
            <span class="flow-row__sub">
                <span class="flow-row__trigger">{trigger}</span>
                <span class="flow-row__dot" aria-hidden="true">" · "</span>
                <span class="flow-row__run" class:flow-row__run--failed=failed title=move || started.map(|s| run_title(lang(), s))>
                    {move || last_run(lang(), &run_flow, now.0.get())}
                </span>
            </span>
            <span class="flow-row__switch">
                <FlowSwitch id=id name=name_for_switch enabled=enabled invalid=invalid />
            </span>
            <span class="flow-row__menu">{menu}</span>
        </li>
    }
}
