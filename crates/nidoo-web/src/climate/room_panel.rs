//! One room of the Climate screen: its card (reading, target, holds, state, eco), its week, its
//! proposal, and its settings behind the card's ⚙.
//!
//! The panel is built once per zone and reads its data from a signal, so a refetch updates the
//! text in place and keeps what the person is doing (an open popup, a pending target, an edit).

use chrono_tz::Tz;
use leptos::prelude::*;
use nidoo_climate::view::ZoneView;

use crate::climate::proposal::{ProposalPanel, RulesPanel};
use crate::climate::room_card::RoomCard;
use crate::climate::room_settings::RoomSettings;
use crate::climate::week_minute;
use crate::components::now::Now;

#[component]
pub fn RoomPanel(
    zone: Signal<ZoneView>,
    tz: Signal<Tz>,
    /// Called after any change so the screen refetches.
    on_changed: Callback<()>,
    /// Whether the panel is on screen; a hidden panel leaves its decisions unfetched. Shown when
    /// absent.
    #[prop(optional, into)]
    shown: Option<Signal<bool>>,
) -> impl IntoView {
    let clock = expect_context::<Now>().0;
    // A memo, so the calendars redraw their now line only when the minute or the day changes.
    let now = Memo::new(move |_| week_minute(clock.get(), tz.get()));
    let settings_open = RwSignal::new(false);
    let shown = shown.unwrap_or_else(|| Signal::from(true));

    // The proposal panel is rebuilt only when the proposal itself changes (id or fingerprint),
    // so a refetch keeps an edit in progress.
    let proposal_key = Memo::new(move |_| {
        zone.with(|z| {
            z.proposal
                .as_ref()
                .map(|p| (p.id.clone(), p.fingerprint.clone()))
        })
    });
    let proposal = move || {
        proposal_key.get().and_then(|_| {
            zone.with_untracked(|z| {
                z.proposal.clone().map(|p| {
                    let approved = Signal::derive(move || zone.with(|z| z.settings.rules.clone()));
                    view! { <ProposalPanel zone=z.zone.clone() proposal=p approved=approved on_changed=on_changed now=now.into() /> }
                })
            })
        })
    };

    view! {
        <section class="climate-panel" class:climate-panel--orphan=move || zone.with(|z| z.orphan)>
            <RoomCard
                zone=zone
                tz=tz
                now=now.into()
                on_changed=on_changed
                on_settings=Callback::new(move |()| settings_open.set(true))
            />
            <div class="card climate-week">
                <RulesPanel zone=zone on_changed=on_changed now=now.into() />
                <p class="climate-legend">
                    <span><i class="climate-legend__swatch climate-temp--low"></i>"≤ 16°"</span>
                    <span><i class="climate-legend__swatch climate-temp--mid"></i>"17 – 19°"</span>
                    <span><i class="climate-legend__swatch climate-temp--high"></i>"≥ 20°"</span>
                </p>
            </div>
            {move || proposal().map(|p| view! { <div class="card climate-proposal-card">{p}</div> })}
            <RoomSettings zone=zone tz=tz on_changed=on_changed open=settings_open shown=shown />
        </section>
    }
}
