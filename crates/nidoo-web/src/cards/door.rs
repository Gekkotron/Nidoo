//! The door opener card: one "Open" button that turns the relay on, asking first by default.

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::model::Command;

use super::Sender;
use super::light::read_on;
use crate::components::card_frame::{CardFrame, open_entity};
use crate::components::confirm::{ConfirmBar, Confirming};
use crate::i18n::security::{SecurityText, action_question, security_text};
use crate::i18n::states::on_off;
use crate::i18n::use_lang;

/// What a tap on "Open" does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DoorTap {
    /// Show the question first.
    Ask,
    Send,
}

pub fn door_tap(ask: bool) -> DoorTap {
    if ask { DoorTap::Ask } else { DoorTap::Send }
}

pub(crate) fn turn_on() -> Command {
    Command::TurnOn {
        brightness: None,
        color: None,
        effect: None,
    }
}

#[component]
pub fn DoorCard(entity: EntityView) -> impl IntoView {
    let sender = Sender::from_context();
    let confirming = expect_context::<Confirming>();
    let on = read_on(&entity.state) == Some(true);
    let lang = use_lang();
    let question = {
        let name = entity.name.clone();
        Signal::derive(move || action_question(lang(), SecurityText::VerbOpen, &name))
    };
    let send = {
        let id = entity.id.clone();
        Callback::new(move |()| {
            confirming.clear();
            sender.send(&id, turn_on(), None);
        })
    };
    let tap = door_tap(entity.ui.security_ask);
    let ask_id = entity.id.clone();
    let on_open = Callback::new(move |()| match tap {
        DoorTap::Ask => confirming.ask(&ask_id),
        DoorTap::Send => send.run(()),
    });
    let asking = {
        let id = entity.id.clone();
        move || confirming.is_asking(&id)
    };
    let pending = sender.pending(&entity.id);
    let on_click = open_entity(&entity.id);
    view! {
        <CardFrame entity=entity on_click=on_click pending=pending>
            <Show
                when=asking
                fallback=move || {
                    view! {
                        <div class="card__controls">
                            <button type="button" class="btn" on:click=move |_| on_open.run(())>
                                {move || security_text(lang(), SecurityText::VerbOpen)}
                            </button>
                        </div>
                        {on
                            .then(|| {
                                view! {
                                    <span class="card__note">{move || on_off(lang(), false, true)}</span>
                                }
                            })}
                    }
                }
            >
                <ConfirmBar
                    question=question
                    yes=Signal::derive(move || security_text(lang(), SecurityText::VerbOpen))
                    on_yes=send
                    on_cancel=Callback::new(move |()| confirming.clear())
                />
            </Show>
        </CardFrame>
    }
}

#[cfg(test)]
mod tests {
    use super::{DoorTap, door_tap};

    #[test]
    fn door_asks_only_with_security_ask() {
        assert_eq!(door_tap(true), DoorTap::Ask);
        assert_eq!(door_tap(false), DoorTap::Send);
    }
}
