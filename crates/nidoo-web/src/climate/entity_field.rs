//! The climate settings' room thermometer field, on the entity picker.

use leptos::prelude::*;

use crate::entity_picker::EntityPickField;
use crate::entity_picker::needs::{is_thermometer, keep};
use crate::format::{list_label, room_name};
use crate::house::House;
use crate::i18n::Lang;
use crate::i18n::climate::{ClimateText, automatic_with, climate_text};
use crate::i18n::use_lang;

/// The label of `id` followed by its room, or the id itself when the house no longer has it.
fn label_with_room(house: House, lang: Lang, id: &str) -> String {
    house.entities.with(|m| match m.get(id) {
        Some(e) => match house.areas.with(|a| room_name(a, &e.area)) {
            Some(room) => format!("{} ({room})", list_label(lang, e)),
            None => list_label(lang, e),
        },
        None => id.to_owned(),
    })
}

/// The thermometer that reads a room: "Automatic" pinned first, then the house's entities with
/// the temperature sensors suggested.
/// `auto` is what Automatic resolves to now; none means the climate devices' own reading.
/// `label` and `auto_text` replace the climate screen's wording ("Room thermometer",
/// "Automatic · …") when given.
#[component]
pub fn ThermometerField(
    value: Signal<Option<String>>,
    auto: Signal<Option<String>>,
    on_change: Callback<Option<String>>,
    #[prop(strip_option, into, default = None)] label: Option<Signal<String>>,
    #[prop(strip_option, into, default = None)] auto_text: Option<Signal<String>>,
) -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let label = Signal::derive(move || {
        label.map_or_else(
            || climate_text(lang(), ClimateText::RoomThermometer).to_owned(),
            |l| l.get(),
        )
    });
    let pinned = Signal::derive(move || {
        if let Some(text) = auto_text {
            return text.get();
        }
        match auto.get() {
            Some(id) => automatic_with(lang(), &label_with_room(house, lang(), &id)),
            None => climate_text(lang(), ClimateText::AutomaticClimateDevice).to_owned(),
        }
    });
    view! {
        <EntityPickField
            label=label
            value=value
            on_change=on_change
            pool=Some(keep(is_thermometer))
            pinned=Some(pinned)
            class=Some("climate-field".to_owned())
        />
    }
}
