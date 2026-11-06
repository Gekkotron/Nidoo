//! Entity cards, one component per kind.

pub mod alarm;
pub mod binary_sensor;
pub mod button;
pub mod chime;
pub mod climate;
pub mod cover;
pub mod door;
pub mod generic;
pub mod group;
pub mod light;
pub mod lock;
pub mod media_player;
pub mod number;
pub mod satellite;
pub mod select;
pub mod sensor;
pub mod siren;
pub mod switch;
pub mod text;
pub mod vacuum;
pub mod water_heater;

use leptos::prelude::*;
use nidoo_core::model::{Command, EntityState};

use crate::commands::{Pending, fire_command};
use crate::components::toast::Toasts;
use crate::format::{choice_word, entity_value, word};
use crate::house::House;
use crate::i18n::{Key, use_lang, use_text};
use nidoo_api::EntityView;

/// The contexts a card needs to send commands, captured while the card is built so event
/// handlers can use them later.
#[derive(Clone, Copy)]
pub struct Sender {
    house: House,
    toasts: Toasts,
    pending: Pending,
}

impl Sender {
    pub fn from_context() -> Self {
        Self {
            house: expect_context(),
            toasts: expect_context(),
            pending: expect_context(),
        }
    }

    /// See [`fire_command`].
    pub fn send(&self, id: &str, cmd: Command, optimistic: Option<EntityState>) {
        fire_command(
            self.house,
            id.to_owned(),
            cmd,
            self.toasts,
            self.pending,
            optimistic,
        );
    }

    /// Sends `cmd` to every one of `ids` at once, with no optimistic state: a group card shows
    /// each member's own echo.
    pub fn send_group(&self, ids: &[String], cmd: Command) {
        for id in ids {
            self.send(id, cmd.clone(), None);
        }
    }

    /// True while `id` has a command in flight.
    pub fn pending(&self, id: &str) -> Signal<bool> {
        self.pending.signal(id)
    }

    /// True while any of `ids` has a command in flight (tracked: one read of the set).
    pub fn any_pending<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> bool {
        self.pending.any(ids)
    }
}

/// The value line of `entity` ("21,5 °C", "Ouvert · 40 %"), following the language toggle.
pub fn live_value(entity: &EntityView) -> Signal<String> {
    let lang = crate::i18n::use_lang();
    let entity = entity.clone();
    Signal::derive(move || entity_value(lang(), &entity))
}

/// The option `raw` of a mode or choice list, in the language: a climate mode by its name, else
/// the shared choice words, else its words.
pub fn option_word(lang: crate::i18n::Lang, raw: &str) -> String {
    let mode = text::mode_label(lang, raw);
    if mode == word(raw) {
        choice_word(lang, raw)
    } else {
        mode
    }
}

/// The pill toggle with a sliding knob. The whole row is the tap target.
#[component]
pub fn PowerToggle(
    on: bool,
    #[prop(into)] label: Signal<String>,
    on_toggle: Callback<()>,
    /// The accessible name when the visible label does not say what the switch is for; empty
    /// leaves the label to name it.
    #[prop(optional, into)]
    aria_label: Signal<String>,
    /// Greys the switch out and ignores taps (a group with no member online).
    #[prop(optional, into)]
    disabled: Signal<bool>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class="power"
            role="switch"
            aria-label=move || Some(aria_label.get()).filter(|l| !l.is_empty())
            aria-checked=on.to_string()
            disabled=move || disabled.get()
            on:click=move |_| on_toggle.run(())
        >
            <span class="power__label">{move || label.get()}</span>
            <span class="switch" class:switch--on=on>
                <span class="switch__knob"></span>
            </span>
        </button>
    }
}

/// The bare pill toggle a card shows at its top right; the entity's name is its accessible name.
#[component]
pub fn ToggleSwitch(
    on: bool,
    #[prop(into)] aria_label: Signal<String>,
    on_toggle: Callback<()>,
    /// Greys the switch out and ignores taps.
    #[prop(optional, into)]
    disabled: Signal<bool>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class="power power--compact"
            role="switch"
            aria-label=move || aria_label.get()
            aria-checked=on.to_string()
            disabled=move || disabled.get()
            on:click=move |_| on_toggle.run(())
        >
            <span class="switch" class:switch--on=on>
                <span class="switch__knob"></span>
            </span>
        </button>
    }
}

/// A value readout between `−` and `+` step buttons. The buttons are disabled when there is
/// nothing to step from; `on_step` receives `-1.0` or `1.0`. `caption` names the value above it.
#[component]
pub fn Stepper(
    #[prop(into)] label: Signal<String>,
    can_step: bool,
    on_step: Callback<f64>,
    /// Empty draws no caption.
    #[prop(optional, into)]
    caption: Signal<String>,
) -> impl IntoView {
    let t = use_text();
    view! {
        <div class="stepper">
            <button
                type="button"
                class="btn btn--round"
                aria-label=move || t(Key::Decrease)
                disabled=!can_step
                on:click=move |_| on_step.run(-1.0)
            >
                "−"
            </button>
            <span class="stepper__value">
                {move || {
                    let c = caption.get();
                    (!c.is_empty()).then(|| view! { <span class="stepper__caption">{c}</span> })
                }}
                {move || label.get()}
            </span>
            <button
                type="button"
                class="btn btn--round"
                aria-label=move || t(Key::Increase)
                disabled=!can_step
                on:click=move |_| on_step.run(1.0)
            >
                "+"
            </button>
        </div>
    }
}

/// A labelled `<select>` over `options`; `on_select` receives the picked option. `captioned`
/// shows the label above the value, inside the pill, instead of leaving it to screen readers.
/// The option words follow the language; the values sent stay the raw ones.
#[component]
pub fn ModeSelect(
    #[prop(into)] label: Signal<String>,
    options: Vec<String>,
    selected: Option<String>,
    on_select: Callback<String>,
    #[prop(optional)] captioned: bool,
    /// True for climate modes (heat, cool, fan modes…), which have shared words. A device's
    /// own options (a select, a tone, a fan speed) keep the words the device announced.
    #[prop(optional)]
    modes: bool,
) -> impl IntoView {
    let lang = use_lang();
    let items = options
        .into_iter()
        .map(|option| {
            let is_selected = selected.as_deref() == Some(option.as_str());
            let raw = option.clone();
            view! {
                <option value=option selected=is_selected>
                    {move || if modes { option_word(lang(), &raw) } else { word(&raw) }}
                </option>
            }
        })
        .collect_view();
    let on_change = move |ev| on_select.run(event_target_value(&ev));
    if captioned {
        return view! {
            <label class="pick">
                <span class="pick__caption">{move || label.get()}</span>
                <select class="pick__select" on:change=on_change>
                    {items}
                </select>
            </label>
        }
        .into_any();
    }
    view! {
        <select class="select" aria-label=move || label.get() on:change=on_change>
            {items}
        </select>
    }
    .into_any()
}
