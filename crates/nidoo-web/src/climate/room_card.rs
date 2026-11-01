//! A room's card on the Climate screen: the reading, the target with − / +, why the engine chose
//! it, the live holds, the room's state and its eco. Settings set once live behind the ⚙.

use chrono_tz::Tz;
use leptos::prelude::*;
use nidoo_climate::doc::{Day, Override, OverrideOrigin, ZoneState};
use nidoo_climate::view::ZoneView;

use crate::api::climate as api;
use crate::climate::eco_pill::EcoPill;
use crate::climate::target_stepper::TargetStepper;
use crate::climate::{
    act, age_text, degrees_text, mode_word, setpoint_text, source_word, until_text,
};
use crate::components::now::Now;
use crate::components::toast::Toasts;
use crate::i18n::Lang;
use crate::i18n::climate::{ClimateText, hold_until, now_with_age, use_climate_text};
use crate::i18n::use_lang;

/// A thermometer reading this old shows its age under the reading (15 min).
const THERMOMETER_AGE_SHOWN_S: u64 = 15 * 60;

const STATES: [(ZoneState, ClimateText); 3] = [
    (ZoneState::Off, ClimateText::ZoneOff),
    (ZoneState::Pending, ClimateText::ZonePending),
    (ZoneState::Active, ClimateText::ZoneActive),
];

fn override_text(lang: Lang, o: &Override, now: u64, tz: Tz) -> String {
    let what = match (o.setpoint, o.mode) {
        (Some(sp), Some(m)) => format!(
            "{} {}",
            mode_word(lang, Some(m)),
            setpoint_text(lang, Some(sp))
        ),
        (Some(sp), None) => setpoint_text(lang, Some(sp)),
        (None, m) => mode_word(lang, m),
    };
    let by = match o.origin {
        OverrideOrigin::Hand => ClimateText::SetOnDevice,
        OverrideOrigin::User => ClimateText::SetHere,
    };
    hold_until(lang, &what, &until_text(lang, o.until, now, tz), by)
}

#[component]
pub fn RoomCard(
    zone: Signal<ZoneView>,
    tz: Signal<Tz>,
    now: Signal<(Day, u16)>,
    on_changed: Callback<()>,
    /// The ⚙ was pressed.
    on_settings: Callback<()>,
) -> impl IntoView {
    let toasts = expect_context::<Toasts>();
    let lang = use_lang();
    let t = use_climate_text();
    let clock = expect_context::<Now>().0;

    let reading = move || {
        zone.with(|z| {
            z.reading.map_or_else(
                || t(ClimateText::NoReading).to_owned(),
                |r| degrees_text(lang(), r),
            )
        })
    };
    let reading_label = move || {
        zone.with(|z| match (z.reading, z.reading_age_s) {
            (Some(_), Some(age)) => now_with_age(lang(), &age_text(lang(), age)),
            _ => t(ClimateText::Now).to_owned(),
        })
    };
    // The room thermometer's own value and age, when it is older than the reading's freshness.
    let thermometer_line = move || {
        zone.with(|z| {
            z.thermometer
                .as_ref()
                .filter(|th| th.age_s >= THERMOMETER_AGE_SHOWN_S)
                .map(|th| {
                    format!(
                        "{} · {}",
                        degrees_text(lang(), th.value),
                        age_text(lang(), th.age_s)
                    )
                })
        })
        .map(|line| view! { <span class="climate-status__label">{line}</span> })
    };
    // "Target · Heat", or "Target" while the engine has not decided.
    let target_label = move || {
        zone.with(|z| match z.decision.as_ref().and_then(|d| d.mode) {
            Some(mode) => format!(
                "{} {}",
                t(ClimateText::TargetIn),
                mode_word(lang(), Some(mode))
            ),
            None => t(ClimateText::Target).to_owned(),
        })
    };
    let why = move || {
        zone.with(|z| match z.decision.as_ref() {
            Some(d) if d.reason.is_empty() => source_word(lang(), d.source).to_owned(),
            Some(d) => format!("{} · {}", source_word(lang(), d.source), d.reason),
            None => t(ClimateText::NoDecision).to_owned(),
        })
    };

    // State switch: a group of toggle buttons, the current one pressed.
    let state_buttons = STATES
        .into_iter()
        .map(|(state, word)| {
            let selected = move || zone.with(|z| z.settings.state == state);
            view! {
                <button
                    type="button"
                    class="chip segmented__option"
                    class:chip--selected=selected
                    aria-pressed=move || selected().to_string()
                    on:click=move |_| {
                        if selected() {
                            return;
                        }
                        let (id, mut next) = zone.with_untracked(|z| (z.zone.clone(), z.settings.clone()));
                        next.state = state;
                        act(toasts, on_changed, async move { api::put_zone(&id, &next).await });
                    }
                >
                    {move || t(word)}
                </button>
            }
        })
        .collect_view();

    // Holds, redrawn on the clock's tick so "until" stays right and an ended hold goes.
    let holds = move || {
        let now = clock.get();
        let mut list = zone.with(|z| z.overrides.clone());
        list.retain(|o| o.until > now);
        let tz = tz.get();
        let lang = lang();
        (!list.is_empty()).then(|| {
            let chips = list
                .iter()
                .map(|o| view! { <span class="chip climate-card__hold">{override_text(lang, o, now, tz)}</span> })
                .collect_view();
            view! {
                <div class="climate-card__holds">
                    {chips}
                    <button
                        type="button"
                        class="climate-card__clear"
                        aria-label=move || t(ClimateText::Clear)
                        on:click=move |_| {
                            let id = zone.with_untracked(|z| z.zone.clone());
                            act(toasts, on_changed, async move { api::clear_override(&id).await });
                        }
                    >
                        "✕"
                    </button>
                </div>
            }
        })
    };

    view! {
        <div class="card climate-status">
            <header class="climate-status__header">
                <h2 class="climate-status__name">{move || zone.with(|z| z.name.clone())}</h2>
                {move || {
                    zone.with(|z| z.orphan)
                        .then(|| view! { <span class="badge">{move || t(ClimateText::NoClimateDevice)}</span> })
                }}
                <button
                    type="button"
                    class="climate-card__gear"
                    aria-label=move || t(ClimateText::RoomSettings)
                    on:click=move |_| on_settings.run(())
                >
                    "⚙"
                </button>
            </header>
            {move || {
                zone.with(|z| z.settings_invalid.clone())
                    .map(|e| view! { <p class="badge climate-status__invalid">{e}</p> })
            }}
            <div class="climate-status__figures">
                <div class="climate-status__figure">
                    <span class="climate-status__label">{reading_label}</span>
                    <strong class="climate-status__reading">{reading}</strong>
                    {thermometer_line}
                </div>
                <div class="climate-status__figure climate-card__target">
                    <span class="climate-status__label">{target_label}</span>
                    <TargetStepper zone=zone now=now on_changed=on_changed />
                </div>
            </div>
            <p class="climate-card__why">{why}</p>
            {holds}
            <div class="climate-card__actions">
                <div class="segmented climate-status__state" role="group" aria-label=move || t(ClimateText::ZoneStateLabel)>
                    {state_buttons}
                </div>
                <EcoPill zone=zone tz=tz on_changed=on_changed />
            </div>
        </div>
    }
}
