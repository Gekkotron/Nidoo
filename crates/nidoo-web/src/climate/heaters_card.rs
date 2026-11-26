//! The heaters card of a room: its on/off heaters, how each is driven, the room's default heater
//! and whether the climate units take over.
//!
//! The card edits a copy of the zone settings and hands the result to `on_change`, which saves
//! the whole settings, as the room's other controls do.

use std::sync::Arc;

use leptos::prelude::*;
use nidoo_climate::doc::{Heater, HeaterDrive, ZoneSettings};

use crate::climate::entity_field::label_with_room;
use crate::climate::settings_sheet::{NumField, Unit};
use crate::entity_picker::EntityAddButton;
use crate::entity_picker::needs::{is_switch, keep};
use crate::house::House;
use crate::i18n::climate::{ClimateText, use_climate_text};
use crate::i18n::use_lang;

/// The longest cool-down the hub accepts, in minutes.
const MAX_COOL_DOWN: u32 = 240;

/// Remove a heater and every reference to it (room default, heat blocks); a list emptied that
/// way becomes None.
pub fn remove_heater(s: &mut ZoneSettings, id: &str) {
    s.heaters.retain(|h| h.entity != id);
    let strip = |list: &mut Option<Vec<String>>| {
        if let Some(ids) = list {
            ids.retain(|i| i != id);
            if ids.is_empty() {
                *list = None;
            }
        }
    };
    strip(&mut s.heated_by);
    for rules in [s.rules.heat.as_mut(), s.rules.cool.as_mut()]
        .into_iter()
        .flatten()
    {
        for block in &mut rules.blocks {
            strip(&mut block.heated_by);
        }
    }
}

/// id -> label for the room's units and heaters, units first.
pub fn heater_labels(
    settings: &ZoneSettings,
    units: &[(String, String)],
    name_of: impl Fn(&str) -> String,
) -> Vec<(String, String)> {
    let mut all = units.to_vec();
    for h in &settings.heaters {
        if !all.iter().any(|(id, _)| *id == h.entity) {
            all.push((h.entity.clone(), name_of(&h.entity)));
        }
    }
    all
}

/// The room default after ticking or unticking `id`: `Some(new value of heated_by)`, or `None`
/// when nothing changes (the last tick cannot be removed). A list that is exactly the climate
/// units is written as `None`.
fn toggled_default(
    settings: &ZoneSettings,
    unit_ids: &[String],
    all: &[String],
    id: &str,
) -> Option<Option<Vec<String>>> {
    let selected: Vec<&String> = match &settings.heated_by {
        Some(list) => list.iter().collect(),
        None => unit_ids.iter().collect(),
    };
    let was_on = selected.iter().any(|i| i.as_str() == id);
    let next: Vec<String> = all
        .iter()
        .filter(|i| (i.as_str() == id) != selected.contains(i))
        .cloned()
        .collect();
    if was_on && next.is_empty() {
        return None;
    }
    let is_units = next.len() == unit_ids.len() && unit_ids.iter().all(|u| next.contains(u));
    Some(if is_units { None } else { Some(next) })
}

/// The room's units (read-only), its heaters, the default heater and the fill-in switch.
#[component]
pub fn HeatersCard(
    settings: Signal<ZoneSettings>,
    /// The room's climate units: id, label.
    units: Signal<Vec<(String, String)>>,
    on_change: Callback<ZoneSettings>,
) -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let t = use_climate_text();

    // Applies `edit` to a copy of the settings and sends it.
    let edit = move |edit: Box<dyn FnOnce(&mut ZoneSettings)>| {
        let mut next = settings.get_untracked();
        edit(&mut next);
        on_change.run(next);
    };
    let edit_heater = move |idx: usize, f: Box<dyn FnOnce(&mut Heater)>| {
        edit(Box::new(move |s| {
            if let Some(h) = s.heaters.get_mut(idx) {
                f(h);
            }
        }));
    };
    let name_of = move |id: &str| label_with_room(house, lang(), id);

    let unit_rows = move || {
        units
            .get()
            .into_iter()
            .map(|(_, label)| {
                view! {
                    <div class="climate-heater">
                        <span class="climate-heater__name">{label}</span>
                        <span class="climate-field__hint">{move || t(ClimateText::ClimateUnit)}</span>
                    </div>
                }
            })
            .collect_view()
    };

    let heater_rows = move || {
        settings
            .with(|s| s.heaters.clone())
            .into_iter()
            .enumerate()
            .map(|(idx, h)| {
                let id = h.entity.clone();
                let on = h.enabled;
                let hold = h.drive == HeaterDrive::Hold;
                let min_off = f64::from(h.min_off_min);
                view! {
                    <div class="climate-heater">
                        <span class="climate-heater__name">{name_of(&id)}</span>
                        <select
                            class="field"
                            on:change=move |ev| {
                                let drive = if event_target_value(&ev) == "hold" {
                                    HeaterDrive::Hold
                                } else {
                                    HeaterDrive::Thermostat
                                };
                                edit_heater(idx, Box::new(move |h| h.drive = drive));
                            }
                        >
                            <option value="thermostat" selected=!hold>
                                {move || t(ClimateText::DriveThermostat)}
                            </option>
                            <option value="hold" selected=hold>
                                {move || t(ClimateText::DriveHold)}
                            </option>
                        </select>
                        <NumField
                            label=ClimateText::CoolDown
                            unit=Unit::Word(ClimateText::UnitMin)
                            value=Signal::derive(move || min_off)
                            on_change=Callback::new(move |v: f64| {
                                let minutes = cool_down(v);
                                edit_heater(idx, Box::new(move |h| h.min_off_min = minutes));
                            })
                        />
                        <button
                            type="button"
                            role="switch"
                            class="switch"
                            class:switch--on=on
                            aria-checked=on.to_string()
                            on:click={
                                move |_| edit_heater(idx, Box::new(move |h| h.enabled = !on))
                            }
                        >
                            <span class="switch__knob"></span>
                        </button>
                        <button
                            type="button"
                            class="btn btn--compact"
                            on:click={
                                let id = id.clone();
                                move |_| {
                                    let id = id.clone();
                                    edit(Box::new(move |s| remove_heater(s, &id)));
                                }
                            }
                        >
                            "✕"
                        </button>
                    </div>
                }
            })
            .collect_view()
    };

    let heater_ids = Signal::derive(move || {
        settings.with(|s| {
            s.heaters
                .iter()
                .map(|h| h.entity.clone())
                .collect::<Vec<_>>()
        })
    });
    let add = Callback::new(move |id: String| {
        edit(Box::new(move |s| {
            if s.heaters.iter().any(|h| h.entity == id) {
                remove_heater(s, &id);
            } else {
                s.heaters.push(Heater {
                    entity: id,
                    ..Heater::default()
                });
            }
        }));
    });

    let has_heaters = move || settings.with(|s| !s.heaters.is_empty());
    let chips = move || {
        let all = settings.with(|s| heater_labels(s, &units.get(), name_of));
        let unit_ids: Vec<String> = units.with(|u| u.iter().map(|(id, _)| id.clone()).collect());
        let ids: Vec<String> = all.iter().map(|(id, _)| id.clone()).collect();
        let selected: Vec<String> =
            settings.with(|s| s.heated_by.clone().unwrap_or_else(|| unit_ids.clone()));
        all.into_iter()
            .map(|(id, label)| {
                let on = selected.contains(&id);
                let (unit_ids, ids, pick) = (unit_ids.clone(), ids.clone(), id.clone());
                view! {
                    <button
                        type="button"
                        class="chip"
                        class:chip--selected=on
                        aria-pressed=on.to_string()
                        on:click=move |_| {
                            let change = settings
                                .with_untracked(|s| toggled_default(s, &unit_ids, &ids, &pick));
                            if let Some(heated_by) = change {
                                edit(Box::new(move |s| s.heated_by = heated_by));
                            }
                        }
                    >
                        {label}
                    </button>
                }
            })
            .collect_view()
    };
    let fill_in = move || settings.with(|s| s.fill_in);

    let pool = keep(is_switch);
    view! {
        <div class="climate-heaters">
            <h3 class="climate-room-settings__heading">{move || t(ClimateText::Heaters)}</h3>
            {unit_rows}
            {heater_rows}
            <div class="picker-chips">
                <EntityAddButton
                    label=Signal::derive(move || t(ClimateText::AddHeater).to_owned())
                    taken=heater_ids
                    on_toggle=add
                    pool=Some(Arc::clone(&pool))
                />
            </div>
            <Show when=has_heaters>
                <span class="climate-field__label">{move || t(ClimateText::DefaultHeater)}</span>
                <div class="picker-chips">{chips}</div>
                <label class="climate-heaters__fill">
                    <input
                        type="checkbox"
                        prop:checked=fill_in
                        on:change=move |ev| {
                            let on = event_target_checked(&ev);
                            edit(Box::new(move |s| s.fill_in = on));
                        }
                    />
                    <span>{move || t(ClimateText::FillIn)}</span>
                </label>
            </Show>
        </div>
    }
}

/// A whole number of minutes from a field value, within the hub's 0..=240.
fn cool_down(v: f64) -> u32 {
    // A saturating cast after clamping to the valid range.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let minutes = v.round().clamp(0.0, f64::from(MAX_COOL_DOWN)) as u32;
    minutes
}

#[cfg(test)]
mod tests {
    use nidoo_climate::doc::{Block, Day, Heater, HhMm, ModeRules, Rules, Setpoint};

    use super::*;

    fn heater(id: &str) -> Heater {
        Heater {
            entity: id.into(),
            ..Heater::default()
        }
    }

    fn block(by: Option<&[&str]>) -> Block {
        Block {
            days: vec![Day::Mon],
            from: HhMm(360),
            to: HhMm(480),
            setpoint: Setpoint::Temp(21.0),
            evidence: None,
            heated_by: by.map(|v| v.iter().map(|s| (*s).to_owned()).collect()),
        }
    }

    fn ids(v: &[&str]) -> Option<Vec<String>> {
        Some(v.iter().map(|s| (*s).to_owned()).collect())
    }

    #[test]
    fn remove_heater_clears_every_reference() {
        let mut s = ZoneSettings {
            heaters: vec![heater("stove"), heater("rad")],
            heated_by: ids(&["stove"]),
            rules: Rules {
                heat: Some(ModeRules {
                    base: Setpoint::Temp(19.0),
                    blocks: vec![
                        block(Some(&["stove", "rad"])),
                        block(Some(&["stove"])),
                        block(None),
                    ],
                }),
                cool: None,
            },
            ..ZoneSettings::default()
        };
        remove_heater(&mut s, "stove");
        assert_eq!(s.heaters, vec![heater("rad")]);
        assert_eq!(s.heated_by, None);
        let blocks = &s.rules.heat.as_ref().unwrap().blocks;
        assert_eq!(blocks[0].heated_by, ids(&["rad"]));
        assert_eq!(blocks[1].heated_by, None);
        assert_eq!(blocks[2].heated_by, None);
    }

    #[test]
    fn remove_heater_keeps_the_rest_of_a_default() {
        let mut s = ZoneSettings {
            heaters: vec![heater("stove")],
            heated_by: ids(&["unit", "stove"]),
            ..ZoneSettings::default()
        };
        remove_heater(&mut s, "stove");
        assert!(s.heaters.is_empty());
        assert_eq!(s.heated_by, ids(&["unit"]));
    }

    #[test]
    fn cool_down_stays_within_the_hubs_range() {
        assert_eq!(cool_down(-5.0), 0);
        assert_eq!(cool_down(14.6), 15);
        assert_eq!(cool_down(900.0), 240);
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn ticking_a_heater_writes_an_explicit_default() {
        let s = ZoneSettings {
            heaters: vec![heater("stove")],
            ..ZoneSettings::default()
        };
        let (units, all) = (strings(&["pump"]), strings(&["pump", "stove"]));
        // None shows the units ticked; adding the stove makes both explicit.
        assert_eq!(
            toggled_default(&s, &units, &all, "stove"),
            Some(ids(&["pump", "stove"]))
        );
        // The pump is the only one ticked: it stays.
        assert_eq!(toggled_default(&s, &units, &all, "pump"), None);
        // With both ticked, unticking the pump leaves the stove.
        let both = ZoneSettings {
            heated_by: ids(&["pump", "stove"]),
            ..s
        };
        assert_eq!(
            toggled_default(&both, &units, &all, "pump"),
            Some(ids(&["stove"]))
        );
    }

    #[test]
    fn a_default_equal_to_the_units_goes_back_to_none() {
        let s = ZoneSettings {
            heaters: vec![heater("stove")],
            heated_by: ids(&["pump", "stove"]),
            ..ZoneSettings::default()
        };
        let (units, all) = (strings(&["pump"]), strings(&["pump", "stove"]));
        assert_eq!(toggled_default(&s, &units, &all, "stove"), Some(None));
    }

    #[test]
    fn the_last_ticked_heater_stays() {
        let s = ZoneSettings {
            heaters: vec![heater("stove")],
            heated_by: ids(&["stove"]),
            ..ZoneSettings::default()
        };
        let (units, all) = (strings(&["pump"]), strings(&["pump", "stove"]));
        assert_eq!(toggled_default(&s, &units, &all, "stove"), None);
    }

    #[test]
    fn heater_labels_lists_units_first() {
        let s = ZoneSettings {
            heaters: vec![heater("stove"), heater("rad")],
            ..ZoneSettings::default()
        };
        let units = vec![("heat_pump".to_owned(), "Heat pump".to_owned())];
        let got = heater_labels(&s, &units, |id| id.to_uppercase());
        assert_eq!(
            got,
            vec![
                ("heat_pump".to_owned(), "Heat pump".to_owned()),
                ("stove".to_owned(), "STOVE".to_owned()),
                ("rad".to_owned(), "RAD".to_owned()),
            ]
        );
    }
}
