//! The climate card, centred: the room temperature large, the target between step buttons, then
//! the mode and fan pickers side by side.

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::model::{ClimateState, Command, EntityState, StateValue};

use super::text::{decimal, mode_label};
use super::{ModeSelect, Sender};
use crate::components::card_frame::{CardFrame, open_entity};
use crate::components::card_place::{CardPlace, use_card_place};
use crate::components::dial::Dial;
use crate::format::entity_title;
use crate::i18n::{Key, use_lang, use_text};

const DEFAULT_STEP: f64 = 0.5;

/// The target after one step up (`+1.0`) or down (`-1.0`) from `current`, kept within the known
/// bounds. `None` when there is no target to step from.
pub(super) fn stepped(
    current: Option<f64>,
    step: Option<f64>,
    direction: f64,
    min: Option<f64>,
    max: Option<f64>,
) -> Option<f64> {
    let step = step.filter(|s| *s > 0.0).unwrap_or(DEFAULT_STEP);
    let next = current? + direction * step;
    let next = min.map_or(next, |m| next.max(m));
    Some(max.map_or(next, |m| next.min(m)))
}

fn with_climate(state: &EntityState, f: impl FnOnce(&mut ClimateState)) -> Option<EntityState> {
    let StateValue::Climate(climate) = &state.value else {
        return None;
    };
    let mut climate = climate.clone();
    f(&mut climate);
    Some(EntityState {
        value: StateValue::Climate(climate),
        ..state.clone()
    })
}

/// The colour of the ring for a mode.
fn tone(mode: Option<&str>) -> &'static str {
    match mode {
        Some("heat") => "heat",
        Some("cool") => "cool",
        Some("off") => "off",
        _ => "other",
    }
}

#[component]
pub fn ClimateCard(entity: EntityView) -> impl IntoView {
    let sender = Sender::from_context();
    let lang = use_lang();
    let t = use_text();
    let (current, target, mode, fan) = match &entity.state.value {
        StateValue::Climate(c) => (c.current, c.target, c.mode.clone(), c.fan_mode.clone()),
        _ => (None, None, None, None),
    };
    let caps = &entity.capabilities;
    let is_off = mode.as_deref() == Some("off");
    let (min, max) = (caps.min_temp.unwrap_or(7.0), caps.max_temp.unwrap_or(30.0));
    let step = caps.temp_step.filter(|s| *s > 0.0).unwrap_or(DEFAULT_STEP);

    let send_target = {
        let id = entity.id.clone();
        let state = entity.state.clone();
        move |temperature: f64| {
            let guess = with_climate(&state, |c| c.target = Some(temperature));
            sender.send(&id, Command::SetTemperature { temperature }, guess);
        }
    };
    let on_commit = Callback::new(send_target.clone());
    let on_step = {
        let (step, min, max) = (caps.temp_step, caps.min_temp, caps.max_temp);
        move |direction: f64| {
            if let Some(temperature) = stepped(target, step, direction, min, max) {
                send_target(temperature);
            }
        }
    };
    let (down, up) = (on_step.clone(), on_step);

    // The mode as a badge: a picker when the unit lists its modes.
    let trailing = if caps.hvac_modes.is_empty() {
        mode.clone().map(|m| {
            view! { <span class="card__badge">{move || mode_label(lang(), &m)}</span> }.into_any()
        })
    } else {
        let id = entity.id.clone();
        let state = entity.state.clone();
        let options = caps
            .hvac_modes
            .iter()
            .map(|option| {
                let selected = mode.as_deref() == Some(option.as_str());
                let value = option.clone();
                let label = option.clone();
                view! {
                    <option value=value selected=selected>{move || mode_label(lang(), &label)}</option>
                }
            })
            .collect_view();
        let on_change = move |ev| {
            let picked = event_target_value(&ev);
            let guess = with_climate(&state, |c| c.mode = Some(picked.clone()));
            sender.send(&id, Command::SetHvacMode { mode: picked }, guess);
        };
        Some(
            view! {
                <select class="card__badge card__badge--select" aria-label=move || t(Key::Mode) on:change=on_change>
                    {options}
                </select>
            }
            .into_any(),
        )
    };

    let fan_select = (use_card_place() == CardPlace::Sheet && !caps.fan_modes.is_empty()).then(|| {
        let id = entity.id.clone();
        let state = entity.state.clone();
        let on_select = Callback::new(move |picked: String| {
            let guess = with_climate(&state, |c| c.fan_mode = Some(picked.clone()));
            sender.send(&id, Command::SetFanMode { mode: picked }, guess);
        });
        view! { <ModeSelect label=Signal::derive(move || t(Key::ModeFan)) options=caps.fan_modes.clone() selected=fan on_select=on_select captioned=true modes=true /> }
    });

    // "It's 22°", or the mode when the unit reports no room temperature.
    let unit_mode = mode.clone();
    let now_text = move || match (current, unit_mode.as_deref()) {
        (Some(c), _) => format!("{} {}°", t(Key::ItIs), decimal(lang(), c)),
        (None, Some(m)) => mode_label(lang(), m),
        (None, None) => "—".to_owned(),
    };
    let value_text = Callback::new(move |v: f64| format!("{}°", decimal(lang(), v)));
    let placeholder = Signal::derive(move || {
        if is_off {
            mode_label(lang(), "off")
        } else {
            "—".to_owned()
        }
    });
    let disabled = is_off || target.is_none();
    let name = {
        let entity = entity.clone();
        Signal::derive(move || entity_title(lang(), &entity))
    };

    let pending = sender.pending(&entity.id);
    let on_click = open_entity(&entity.id);
    let id = entity.id.clone();
    view! {
        <CardFrame
            entity=entity
            on_click=on_click
            pending=pending
            trailing=trailing.unwrap_or_else(|| ().into_any())
            tint="sage"
            row_header=true
        >
            <div class="climate">
                <Dial
                    draft_key=id
                    value=target
                    min=min
                    max=max
                    step=step
                    disabled=disabled
                    aria_label=name
                    value_text=value_text
                    placeholder=placeholder
                    caption=Signal::derive(move || t(Key::TargetTemperature))
                    tone=tone(mode.as_deref())
                    on_commit=on_commit
                />
                <div class="climate__foot">
                    <button
                        type="button"
                        class="btn btn--round"
                        aria-label=move || t(Key::Decrease)
                        disabled=disabled
                        on:click=move |_| down(-1.0)
                    >
                        "−"
                    </button>
                    <span class="climate__now">{now_text}</span>
                    <button
                        type="button"
                        class="btn btn--round"
                        aria-label=move || t(Key::Increase)
                        disabled=disabled
                        on:click=move |_| up(1.0)
                    >
                        "+"
                    </button>
                </div>
                {fan_select}
            </div>
        </CardFrame>
    }
}

#[cfg(test)]
mod tests {
    use super::stepped;

    #[test]
    fn steps_by_the_given_step() {
        assert_eq!(stepped(Some(20.0), Some(1.0), 1.0, None, None), Some(21.0));
        assert_eq!(stepped(Some(20.0), Some(1.0), -1.0, None, None), Some(19.0));
    }

    #[test]
    fn defaults_to_half_a_degree() {
        assert_eq!(stepped(Some(20.0), None, 1.0, None, None), Some(20.5));
    }

    #[test]
    fn clamps_to_bounds() {
        assert_eq!(
            stepped(Some(30.0), Some(1.0), 1.0, Some(5.0), Some(30.0)),
            Some(30.0)
        );
        assert_eq!(
            stepped(Some(5.0), Some(1.0), -1.0, Some(5.0), Some(30.0)),
            Some(5.0)
        );
    }

    #[test]
    fn no_target_no_step() {
        assert_eq!(stepped(None, None, 1.0, None, None), None);
    }
}
