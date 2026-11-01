//! The binary sensor card: a state word, the icon tinted by the card's on state.

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::model::StateValue;

use super::Sender;
use crate::components::card_frame::{CardFrame, open_entity};
use crate::i18n::states::{StateCtx, state_word};
use crate::i18n::{Lang, use_lang};

/// The word for a binary sensor: "Open"/"Closed" for doors and windows, "Motion"/"Clear" for
/// motion, "Smoke"/"Clear" for smoke, "On"/"Off" otherwise; French agrees with the device class.
pub fn binary_text(lang: Lang, device_class: Option<&str>, on: bool) -> String {
    let ctx = StateCtx::BinarySensor(device_class.map(str::to_owned));
    state_word(lang, ctx, if on { "on" } else { "off" })
}

#[component]
pub fn BinarySensorCard(entity: EntityView) -> impl IntoView {
    let sender = Sender::from_context();
    let lang = use_lang();
    let text = match &entity.state.value {
        StateValue::OnOff(on) => {
            let (class, on) = (entity.device_class.clone(), *on);
            Signal::derive(move || binary_text(lang(), class.as_deref(), on))
        }
        _ => super::live_value(&entity),
    };
    let pending = sender.pending(&entity.id);
    let on_click = open_entity(&entity.id);
    view! {
        <CardFrame entity=entity on_click=on_click pending=pending>
            <span class="card__value">{text}</span>
        </CardFrame>
    }
}

#[cfg(test)]
mod tests {
    use super::binary_text;
    use crate::i18n::Lang;

    #[test]
    fn words_by_device_class() {
        let en = |class, on| binary_text(Lang::En, class, on);
        assert_eq!(en(Some("door"), true), "Open");
        assert_eq!(en(Some("window"), false), "Closed");
        assert_eq!(en(Some("motion"), true), "Motion");
        assert_eq!(en(Some("motion"), false), "Clear");
        assert_eq!(en(Some("smoke"), true), "Smoke");
        assert_eq!(en(Some("smoke"), false), "Clear");
        assert_eq!(en(None, true), "On");
        assert_eq!(en(Some("battery"), false), "Off");
    }

    #[test]
    fn french_agrees_with_the_device_class() {
        let fr = |class, on| binary_text(Lang::Fr, class, on);
        assert_eq!(fr(Some("door"), true), "Ouverte");
        assert_eq!(fr(Some("motion"), false), "Aucun mouvement");
        assert_eq!(fr(Some("smoke"), true), "Fumée");
        assert_eq!(fr(None, true), "Activé");
    }
}
