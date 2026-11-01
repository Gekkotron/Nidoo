//! The siren card: a power toggle that reads "Ringing" in red while on and asks before ringing.

use leptos::prelude::*;
use nidoo_api::EntityView;

use super::light::{power_command, read_on, with_on};
use super::{PowerToggle, Sender};
use crate::components::card_frame::{CardFrame, open_entity};
use crate::components::confirm::{ConfirmBar, Confirming};
use crate::format::entity_value;
use crate::i18n::security::{SecurityText, action_question, security_text};

#[component]
pub fn SirenCard(entity: EntityView) -> impl IntoView {
    let sender = Sender::from_context();
    let confirming = expect_context::<Confirming>();
    let on = read_on(&entity.state).unwrap_or(false);
    let lang = crate::i18n::use_lang();
    let label = {
        let entity = entity.clone();
        Signal::derive(move || {
            if on {
                security_text(lang(), SecurityText::Ringing).to_owned()
            } else {
                entity_value(lang(), &entity)
            }
        })
    };
    let question = {
        let name = entity.name.clone();
        Signal::derive(move || action_question(lang(), SecurityText::VerbRing, &name))
    };
    let send = {
        let id = entity.id.clone();
        let state = entity.state.clone();
        Callback::new(move |next: bool| {
            confirming.clear();
            sender.send(&id, power_command(next), with_on(&state, next));
        })
    };
    let ask = entity.ui.security_ask;
    let ask_id = entity.id.clone();
    let toggle = Callback::new(move |()| {
        if !on && ask {
            confirming.ask(&ask_id);
        } else {
            send.run(!on);
        }
    });
    let asking = {
        let id = entity.id.clone();
        move || confirming.is_asking(&id)
    };
    let pending = sender.pending(&entity.id);
    let on_click = open_entity(&entity.id);
    view! {
        <CardFrame entity=entity on_click=on_click pending=pending alert=on>
            <Show
                when=asking
                fallback=move || {
                    view! { <PowerToggle on=on label=label on_toggle=toggle /> }
                }
            >
                <ConfirmBar
                    question=question
                    yes=Signal::derive(move || security_text(lang(), SecurityText::VerbRing))
                    on_yes=Callback::new(move |()| send.run(true))
                    on_cancel=Callback::new(move |()| confirming.clear())
                />
            </Show>
        </CardFrame>
    }
}
