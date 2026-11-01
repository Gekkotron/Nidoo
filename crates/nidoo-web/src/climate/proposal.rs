//! The proposal panel and the approved-rules panel of a zone card.
//!
//! Both show the rules on a week timeline. A proposal is reviewed as learned, or edited first
//! and approved as edited; a direct edit of the approved rules saves through the zone settings
//! and counts as approval (spec §4.4). Edits are local until Approve or Save, and live in
//! signals the card owns, so a refetch of the zone leaves them alone.

use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_api::ApiError;
use nidoo_climate::doc::{Day, Mode, ModeRules, Proposal, ProposalKind, Rules, Setpoint};
use nidoo_climate::learn::diff;
use nidoo_climate::view::ZoneView;

use super::timeline::{NEW_BASE, WeekTimeline};
use crate::api::climate as api;
use crate::components::toast::Toasts;
use crate::i18n::climate::{ClimateText, could_not_save, use_climate_text};
use crate::i18n::{Key, use_lang, use_text};

fn present(rules: &Rules) -> Vec<Mode> {
    [Mode::Heat, Mode::Cool]
        .into_iter()
        .filter(|m| rules.get(*m).is_some())
        .collect()
}

/// `rules` with the schedule of `mode` replaced.
fn with_mode(rules: &Rules, mode: Mode, next: ModeRules) -> Rules {
    let mut out = rules.clone();
    match mode {
        Mode::Heat => out.heat = Some(next),
        Mode::Cool => out.cool = Some(next),
    }
    out
}

/// A timeline of `rules`, one tab per mode when both exist.
#[component]
fn RulesTimeline(
    rules: Signal<Rules>,
    editable: Signal<bool>,
    on_change: Callback<Rules>,
    now: Signal<(Day, u16)>,
) -> impl IntoView {
    let lang = use_lang();
    let t = use_text();
    let modes = Memo::new(move |_| rules.with(present));
    let mode = RwSignal::new(modes.with_untracked(|m| m.first().copied().unwrap_or(Mode::Heat)));
    let tabs = move || {
        let list = modes.get();
        (list.len() > 1).then(|| {
            let buttons = list
                .into_iter()
                .map(|m| {
                    view! {
                        <button
                            type="button"
                            class="chip segmented__option"
                            class:chip--selected=move || mode.get() == m
                            aria-pressed=move || (mode.get() == m).to_string()
                            on:click=move |_| mode.set(m)
                        >
                            {move || super::mode_word(lang(), Some(m))}
                        </button>
                    }
                })
                .collect_view();
            view! { <div class="segmented" role="group" aria-label=move || t(Key::Mode)>{buttons}</div> }
        })
    };
    // Rebuilt only when the mode tab or the edit toggle changes, never while dragging.
    let body = move || {
        let m = mode.get();
        let editable = editable.get();
        let current = Signal::derive(move || {
            rules.with(|r| {
                r.get(m).cloned().unwrap_or(ModeRules {
                    base: Setpoint::Off,
                    blocks: Vec::new(),
                })
            })
        });
        let change = Callback::new(move |next: ModeRules| {
            on_change.run(with_mode(&rules.get_untracked(), m, next));
        });
        rules.with_untracked(|r| r.get(m).is_some()).then(
            || view! { <WeekTimeline rules=current editable=editable on_change=change now=now /> },
        )
    };
    view! {
        {tabs}
        {body}
    }
}

fn report(toasts: Toasts, result: Result<(), ApiError>, on_changed: Callback<()>) {
    match result {
        Ok(()) => on_changed.run(()),
        Err(e) => toasts.push(e.error),
    }
}

/// A learned proposal: its changes, the proposed week, Edit, Approve and Reject.
#[component]
pub fn ProposalPanel(
    zone: String,
    proposal: Proposal,
    approved: Signal<Rules>,
    /// Called after an approval or rejection so the screen refetches.
    on_changed: Callback<()>,
    /// Today and the minute of the day in the house time zone.
    now: Signal<(Day, u16)>,
) -> impl IntoView {
    let toasts = expect_context::<Toasts>();
    let t = use_climate_text();
    let lang = use_lang();
    let text = use_text();
    let learned = proposal.rules.clone();
    let edited = RwSignal::new(proposal.rules.clone());
    let editing = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let initial = proposal.kind == ProposalKind::Initial;
    let title = move || {
        t(if initial {
            ClimateText::ProposedSchedule
        } else {
            ClimateText::ProposedChanges
        })
    };
    let dirty = {
        let learned = learned.clone();
        move || edited.with(|e| *e != learned)
    };
    let changes = {
        let learned = proposal.changes.clone();
        move || {
            if editing.get() {
                approved.with_untracked(|a| edited.with(|e| diff(Some(a), e)))
            } else {
                learned.clone()
            }
        }
    };
    let changes_view = move || {
        let lines = changes();
        if lines.is_empty() {
            view! { <p class="climate-proposal__none">{move || t(ClimateText::NoChange)}</p> }
                .into_any()
        } else {
            let items = lines
                .into_iter()
                .map(|l| view! { <li>{l}</li> })
                .collect_view();
            view! { <ul class="climate-proposal__changes">{items}</ul> }.into_any()
        }
    };

    let approve_zone = zone.clone();
    let approve_dirty = dirty.clone();
    let approve = move |_| {
        if busy.get_untracked() {
            return;
        }
        let rules = edited.get_untracked();
        let send_edited = approve_dirty();
        if send_edited && let Err(e) = rules.validate() {
            toasts.push(could_not_save(untrack(lang), &e));
            return;
        }
        busy.set(true);
        let zone = approve_zone.clone();
        spawn_local(async move {
            let result = api::approve(&zone, send_edited.then_some(&rules)).await;
            busy.try_set(false);
            report(toasts, result, on_changed);
        });
    };
    let reject_zone = zone;
    let reject = move |_| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        let zone = reject_zone.clone();
        spawn_local(async move {
            let result = api::reject(&zone).await;
            busy.try_set(false);
            report(toasts, result, on_changed);
        });
    };
    let discard = {
        let learned = learned.clone();
        move |_| {
            edited.set(learned.clone());
            editing.set(false);
        }
    };

    view! {
        <section class="climate-proposal" aria-label=title>
            <h3 class="climate-proposal__title">{title}</h3>
            // The learned changes read above the week; while editing, the live diff sits under it.
            {
                let learned = changes_view.clone();
                move || (!editing.get()).then(&learned)
            }
            <RulesTimeline
                rules=edited.into()
                editable=editing.into()
                on_change=Callback::new(move |next| edited.set(next))
                now=now
            />
            {move || editing.get().then(&changes_view)}
            <div class="climate-proposal__actions">
                {move || {
                    if editing.get() {
                        view! {
                            <button type="button" class="btn btn--compact" on:click=discard.clone()>
                                {move || t(ClimateText::DiscardEdits)}
                            </button>
                        }
                            .into_any()
                    } else {
                        view! {
                            <button
                                type="button"
                                class="btn btn--compact"
                                on:click=move |_| editing.set(true)
                            >
                                {move || text(Key::Edit)}
                            </button>
                        }
                            .into_any()
                    }
                }}
                <button
                    type="button"
                    class="btn btn--primary"
                    disabled=move || busy.get()
                    on:click=approve
                >
                    {
                        let dirty = dirty.clone();
                        move || {
                            t(if dirty() { ClimateText::ApproveEdited } else { ClimateText::Approve })
                        }
                    }
                </button>
                <button type="button" class="btn" disabled=move || busy.get() on:click=reject>
                    {move || t(ClimateText::Reject)}
                </button>
            </div>
        </section>
    }
}

/// The zone's approved rules on a timeline; "Edit schedule" saves through the zone settings.
#[component]
pub fn RulesPanel(
    zone: Signal<ZoneView>,
    on_changed: Callback<()>,
    /// Today and the minute of the day in the house time zone.
    now: Signal<(Day, u16)>,
) -> impl IntoView {
    let toasts = expect_context::<Toasts>();
    let t = use_climate_text();
    let lang = use_lang();
    let id = zone.with_untracked(|z| z.zone.clone());
    let editing = RwSignal::new(false);
    let draft = RwSignal::new(Rules::default());
    let busy = RwSignal::new(false);
    // The approved rules while viewing, the draft while editing.
    let shown = Signal::derive(move || {
        if editing.get() {
            draft.get()
        } else {
            zone.with(|z| z.settings.rules.clone())
        }
    });
    let empty = move || zone.with(|z| z.settings.rules.is_empty());
    let save = Callback::new(move |()| {
        if busy.get_untracked() {
            return;
        }
        let rules = draft.get_untracked();
        if let Err(e) = rules.validate() {
            toasts.push(could_not_save(untrack(lang), &e));
            return;
        }
        let mut settings = zone.with_untracked(|z| z.settings.clone());
        settings.rules = rules;
        busy.set(true);
        let id = id.clone();
        spawn_local(async move {
            let result = api::put_zone(&id, &settings).await;
            busy.try_set(false);
            if result.is_ok() {
                editing.try_set(false);
            }
            report(toasts, result, on_changed);
        });
    });
    view! {
        <section class="climate-rules" aria-label=move || t(ClimateText::ApprovedRules)>
            <h3 class="climate-proposal__title">{move || t(ClimateText::ThisWeek)}</h3>
            // A room without rules starts an empty heating week here; saving it approves it, and
            // the hub turns a pending room active.
            <Show
                when=move || !empty() || editing.get()
                fallback=move || {
                    view! {
                        <p class="climate-proposal__none">{move || t(ClimateText::NoApprovedSchedule)}</p>
                        <div class="climate-proposal__actions">
                            <button
                                type="button"
                                class="btn btn--compact"
                                on:click=move |_| {
                                    draft.set(Rules {
                                        heat: Some(ModeRules {
                                            base: Setpoint::Temp(NEW_BASE),
                                            blocks: Vec::new(),
                                        }),
                                        cool: None,
                                    });
                                    editing.set(true);
                                }
                            >
                                {move || t(ClimateText::CreateSchedule)}
                            </button>
                        </div>
                    }
                }
            >
                <RulesTimeline
                    rules=shown
                    editable=editing.into()
                    on_change=Callback::new(move |next| draft.set(next))
                    now=now
                />
                <div class="climate-proposal__actions">
                    {move || {
                        if editing.get() {
                            view! {
                                <button
                                    type="button"
                                    class="btn btn--primary"
                                    disabled=move || busy.get()
                                    on:click=move |_| save.run(())
                                >
                                    {move || t(ClimateText::Save)}
                                </button>
                                <button
                                    type="button"
                                    class="btn"
                                    on:click=move |_| editing.set(false)
                                >
                                    {move || t(ClimateText::Cancel)}
                                </button>
                            }
                                .into_any()
                        } else {
                            view! {
                                <button
                                    type="button"
                                    class="btn btn--compact"
                                    on:click=move |_| {
                                        draft.set(zone.with_untracked(|z| z.settings.rules.clone()));
                                        editing.set(true);
                                    }
                                >
                                    {move || t(ClimateText::EditSchedule)}
                                </button>
                            }
                                .into_any()
                        }
                    }}
                </div>
            </Show>
        </section>
    }
}
