//! The water heater card: a tank picture next to the current temperature, an activity chip (the
//! average heating power when the device reports it, else the mode) and the cold-water inlet
//! temperature. The target stepper and mode select appear only when the heater supports them.

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::model::{Command, EntityState, StateValue, WaterHeaterState};

use super::climate::stepped;
use super::{ModeSelect, Sender, Stepper};
use crate::components::card_frame::{CardFrame, open_entity};
use crate::components::card_place::{CardPlace, use_card_place};
use crate::format::degrees;
use crate::i18n::climate::{ClimateText, climate_text, heating_power, use_climate_text};
use crate::i18n::states::{StateCtx, state_word};
use crate::i18n::{Key, Lang, use_lang, use_text};

fn with_water_heater(
    state: &EntityState,
    f: impl FnOnce(&mut WaterHeaterState),
) -> Option<EntityState> {
    let StateValue::WaterHeater(heater) = &state.value else {
        return None;
    };
    let mut heater = heater.clone();
    f(&mut heater);
    Some(EntityState {
        value: StateValue::WaterHeater(heater),
        ..state.clone()
    })
}

/// What the card shows beyond the temperature, derived from the state and its attributes.
#[derive(Debug, Clone, PartialEq)]
struct HeaterView {
    /// The element is heating: `power` above zero, or without `power` a mode other than "off".
    heating: bool,
    /// The average heating power in percent, from the numeric `power` attribute.
    power: Option<u8>,
    /// The cold-water inlet temperature, from the numeric `inlet_temperature` attribute.
    inlet: Option<f64>,
    /// The activity chip text; `None` when there is nothing to say.
    chip: Option<String>,
}

impl HeaterView {
    fn from_state(lang: Lang, state: &EntityState) -> Self {
        let mode = match &state.value {
            StateValue::WaterHeater(w) => w.mode.as_deref(),
            _ => None,
        };
        let power = state
            .attributes
            .get("power")
            .and_then(serde_json::Value::as_f64)
            .filter(|p| p.is_finite())
            .map(|p| p.clamp(0.0, 100.0).round() as u8);
        let inlet = state
            .attributes
            .get("inlet_temperature")
            .and_then(serde_json::Value::as_f64)
            .filter(|t| t.is_finite());
        let (heating, chip) = match (power, mode) {
            (Some(p), _) if p > 0 => (true, Some(heating_power(lang, p))),
            (Some(_), _) => (
                false,
                Some(climate_text(lang, ClimateText::Idle).to_owned()),
            ),
            (None, Some(m)) if m.eq_ignore_ascii_case("off") => {
                (false, Some(state_word(lang, StateCtx::Hvac, "off")))
            }
            (None, Some(m)) => (true, Some(state_word(lang, StateCtx::Hvac, m))),
            (None, None) => (false, None),
        };
        Self {
            heating,
            power,
            inlet,
            chip,
        }
    }
}

#[component]
pub fn WaterHeaterCard(entity: EntityView) -> impl IntoView {
    let sender = Sender::from_context();
    let t = use_text();
    let ct = use_climate_text();
    let label = super::live_value(&entity);
    let lang = use_lang();
    let target_text = {
        let target = match &entity.state.value {
            StateValue::WaterHeater(w) => w.target,
            _ => None,
        };
        let unit = entity.unit.clone();
        Signal::derive(move || degrees(lang(), target, unit.as_deref()))
    };
    let (target, mode) = match &entity.state.value {
        StateValue::WaterHeater(w) => (w.target, w.mode.clone()),
        _ => (None, None),
    };
    let caps = &entity.capabilities;

    let on_step = {
        let id = entity.id.clone();
        let state = entity.state.clone();
        let (step, min, max) = (caps.temp_step, caps.min_temp, caps.max_temp);
        Callback::new(move |direction: f64| {
            let Some(temperature) = stepped(target, step, direction, min, max) else {
                return;
            };
            let guess = with_water_heater(&state, |w| w.target = Some(temperature));
            sender.send(&id, Command::SetTemperature { temperature }, guess);
        })
    };

    let mode_select = (!caps.hvac_modes.is_empty()).then(|| {
        let id = entity.id.clone();
        let state = entity.state.clone();
        let on_select = Callback::new(move |picked: String| {
            let guess = with_water_heater(&state, |w| w.mode = Some(picked.clone()));
            sender.send(&id, Command::SetHvacMode { mode: picked }, guess);
        });
        view! { <ModeSelect label=Signal::derive(move || t(Key::Mode)) options=caps.hvac_modes.clone() selected=mode on_select=on_select captioned=true modes=true /> }
    });

    let heater = {
        let state = entity.state.clone();
        Signal::derive(move || HeaterView::from_state(lang(), &state))
    };
    let chip = move || {
        heater.get().chip.map(|text| {
            view! {
                <span
                    class="heater__chip"
                    class:heater__chip--heating=move || heater.get().heating
                >
                    {text}
                </span>
            }
        })
    };
    let unit = entity.unit.clone();
    let inlet = move || {
        heater.get().inlet.map(|degrees_in| {
            let text = degrees(lang(), Some(degrees_in), unit.as_deref());
            view! {
                <span class="heater__inlet">
                    <span aria-hidden="true">"↓ "</span>
                    {ct(ClimateText::Inlet)} " " {text}
                </span>
            }
        })
    };
    // The tile keeps the reading; the target and mode are one tap away, in the sheet.
    let in_sheet = use_card_place() == CardPlace::Sheet;
    let stepper = (in_sheet && target.is_some()).then(|| {
        view! { <Stepper label=target_text caption=Signal::derive(move || ct(ClimateText::Target)) can_step=true on_step=on_step /> }
    });
    let mode_select = mode_select.filter(|_| in_sheet);

    let open = open_entity(&entity.id);
    // The menu leads to the sheet, so the sheet has none.
    let trailing = (!in_sheet).then(|| {
        view! {
            <button
                type="button"
                class="btn btn--icon card__more"
                aria-label=move || t(Key::More)
                on:click=move |_| open.run(())
            >
                "⋯"
            </button>
        }
        .into_any()
    });
    let pending = sender.pending(&entity.id);
    let on_click = open_entity(&entity.id);
    view! {
        <CardFrame
            entity=entity
            on_click=on_click
            pending=pending
            trailing=trailing.unwrap_or_else(|| ().into_any())
        >
            <div class="heater">
                <div class="heater__main">
                    <span
                        class="heater__tank"
                        class:heater__tank--heating=move || heater.get().heating
                        aria-hidden="true"
                    >
                        <span class="heater__element"></span>
                    </span>
                    <div class="heater__info">
                        <div class="heater__reading">
                            <span class="heater__caption">{move || ct(ClimateText::Tank)}</span>
                            <span class="heater__now">{label}</span>
                        </div>
                        {chip}
                        {inlet}
                    </div>
                </div>
                {stepper}
                <div class="thermo__picks">{mode_select}</div>
            </div>
        </CardFrame>
    }
}

#[cfg(test)]
mod tests {
    use super::HeaterView;
    use crate::i18n::Lang;
    use nidoo_core::model::{EntityState, StateValue, WaterHeaterState};
    use serde_json::json;

    fn state(mode: Option<&str>, attributes: serde_json::Value) -> EntityState {
        let serde_json::Value::Object(attributes) = attributes else {
            panic!("attributes must be an object");
        };
        EntityState {
            value: StateValue::WaterHeater(WaterHeaterState {
                mode: mode.map(str::to_owned),
                current: Some(58.5),
                target: None,
            }),
            attributes,
            ..EntityState::default()
        }
    }

    #[test]
    fn power_above_zero_is_heating() {
        let v = HeaterView::from_state(
            Lang::En,
            &state(
                Some("heat"),
                json!({"inlet_temperature": 17.9, "power": 43}),
            ),
        );
        assert!(v.heating);
        assert_eq!(v.power, Some(43));
        assert_eq!(v.inlet, Some(17.9));
        assert_eq!(v.chip.as_deref(), Some("Heating · 43\u{A0}%"));
    }

    #[test]
    fn the_chip_reads_in_french() {
        let fr = |mode, attrs| HeaterView::from_state(Lang::Fr, &state(mode, attrs)).chip;
        assert_eq!(
            fr(Some("heat"), json!({"power": 43})).as_deref(),
            Some("En chauffe · 43\u{A0}%")
        );
        assert_eq!(
            fr(Some("heat"), json!({"power": 0})).as_deref(),
            Some("Au repos")
        );
        assert_eq!(fr(Some("off"), json!({})).as_deref(), Some("Arrêt"));
        assert_eq!(fr(Some("eco"), json!({})).as_deref(), Some("Éco"));
    }

    #[test]
    fn power_zero_is_idle_even_in_heat_mode() {
        let v = HeaterView::from_state(Lang::En, &state(Some("heat"), json!({"power": 0})));
        assert!(!v.heating);
        assert_eq!(v.power, Some(0));
        assert_eq!(v.chip.as_deref(), Some("Idle"));
    }

    #[test]
    fn without_power_the_mode_decides() {
        let v = HeaterView::from_state(Lang::En, &state(Some("eco"), json!({})));
        assert!(v.heating);
        assert_eq!(v.power, None);
        assert_eq!(v.chip.as_deref(), Some("Eco"));
    }

    #[test]
    fn mode_off_without_power_is_off() {
        let v = HeaterView::from_state(Lang::En, &state(Some("off"), json!({})));
        assert!(!v.heating);
        assert_eq!(v.chip.as_deref(), Some("Off"));
    }

    #[test]
    fn no_mode_and_no_power_has_no_chip() {
        let v = HeaterView::from_state(Lang::En, &state(None, json!({})));
        assert!(!v.heating);
        assert_eq!(v.chip, None);
    }

    #[test]
    fn non_numeric_attributes_are_ignored() {
        let v = HeaterView::from_state(
            Lang::En,
            &state(
                Some("off"),
                json!({"power": "43", "inlet_temperature": null}),
            ),
        );
        assert_eq!(v.power, None);
        assert_eq!(v.inlet, None);
        assert!(!v.heating);
    }

    #[test]
    fn inlet_is_optional() {
        let with = HeaterView::from_state(Lang::En, &state(None, json!({"inlet_temperature": 12})));
        assert_eq!(with.inlet, Some(12.0));
        let without = HeaterView::from_state(Lang::En, &state(None, json!({"power": 10})));
        assert_eq!(without.inlet, None);
    }

    #[test]
    fn power_is_clamped_to_a_percentage() {
        let v = HeaterView::from_state(Lang::En, &state(None, json!({"power": 140.4})));
        assert_eq!(v.power, Some(100));
        let v = HeaterView::from_state(Lang::En, &state(None, json!({"power": -5})));
        assert_eq!(v.power, Some(0));
        assert!(!v.heating);
    }
}
