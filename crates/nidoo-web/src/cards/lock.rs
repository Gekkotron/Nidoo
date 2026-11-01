//! The lock card: the lock's state in words and one button that does the opposite, asking
//! first before it unlocks.

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::model::{Availability, Command, EntityState, GenericValue, StateValue};

use super::Sender;
use crate::components::card_frame::{CardFrame, open_entity};
use crate::components::confirm::{ConfirmBar, Confirming};
use crate::house::House;
use crate::i18n::security::{SecurityText, action_question, lock_word, security_text};
use crate::i18n::use_lang;

/// What the card says about the lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockWord {
    Locked,
    Unlocked,
    Locking,
    Unlocking,
    Jammed,
    Open,
    Opening,
    Offline,
    Unknown,
}

/// What the card's one button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockAction {
    Lock,
    Unlock,
}

/// The card's face: its word, its button (`None` shows the button disabled) and whether the
/// state wants attention.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockFace {
    pub word: LockWord,
    pub action: Option<LockAction>,
    pub warn: bool,
}

/// The face for `state`: an offline lock is offline whatever it last reported, and a word the
/// lock type does not know reads as unknown.
pub fn lock_face(state: &EntityState) -> LockFace {
    use LockAction::*;
    use LockWord::*;
    if state.availability == Availability::Offline {
        return LockFace {
            word: Offline,
            action: None,
            warn: false,
        };
    }
    let raw = match &state.value {
        StateValue::Generic(g) => match g.fields.get("state") {
            Some(GenericValue::Text(s)) => s.as_str(),
            _ => "",
        },
        _ => "",
    };
    let (word, action, warn) = match raw {
        "locked" => (Locked, Some(Unlock), false),
        "unlocked" => (Unlocked, Some(Lock), true),
        "jammed" => (Jammed, Some(Lock), true),
        "open" => (Open, Some(Lock), true),
        "opening" => (Opening, None, true),
        "locking" => (Locking, None, false),
        "unlocking" => (Unlocking, None, false),
        _ => (Unknown, None, false),
    };
    LockFace { word, action, warn }
}

/// The lock type's `locked` on/off control: on locks, off unlocks.
pub fn lock_command(action: LockAction) -> Command {
    Command::Control {
        control: "locked".into(),
        value: Some(serde_json::Value::Bool(action == LockAction::Lock)),
    }
}

/// Whether an "Unlock?" answered now should still send: only while the lock reads locked.
pub fn unlock_still_wanted(state: &EntityState) -> bool {
    lock_face(state).action == Some(LockAction::Unlock)
}

/// The lock card. It follows the lock's live state by id, so a question left open closes as
/// soon as the lock stops reading locked, whoever moved it.
#[component]
pub fn LockCard(entity: EntityView) -> impl IntoView {
    let house = expect_context::<House>();
    let sender = Sender::from_context();
    let confirming = expect_context::<Confirming>();
    let lang = use_lang();
    let id = entity.id.clone();
    let live = {
        let id = id.clone();
        let fallback = entity.state.clone();
        Memo::new(move |_| {
            house
                .entities
                .with(|m| m.get(&id).map(|e| e.state.clone()))
                .unwrap_or_else(|| fallback.clone())
        })
    };
    let face = Memo::new(move |_| live.with(lock_face));

    // A question about a lock that no longer reads locked is stale: close it.
    {
        let id = id.clone();
        Effect::new(move |_| {
            if face.get().action != Some(LockAction::Unlock) && confirming.is_asking_untracked(&id)
            {
                confirming.clear();
            }
        });
    }

    let send = {
        let id = id.clone();
        move |action: LockAction| sender.send(&id, lock_command(action), None)
    };
    let on_yes = {
        let send = send.clone();
        Callback::new(move |()| {
            confirming.clear();
            if live.with_untracked(unlock_still_wanted) {
                send(LockAction::Unlock);
            }
        })
    };
    let on_press = {
        let id = id.clone();
        Callback::new(move |()| match face.get_untracked().action {
            Some(LockAction::Unlock) => confirming.ask(&id),
            Some(LockAction::Lock) => send(LockAction::Lock),
            None => {}
        })
    };
    let question = {
        let name = entity.name.clone();
        Signal::derive(move || action_question(lang(), SecurityText::VerbUnlock, &name))
    };
    let asking = {
        let id = id.clone();
        move || confirming.is_asking(&id)
    };
    let pending = sender.pending(&id);
    let on_click = open_entity(&id);
    move || {
        let face = face.get();
        // A dashboard mounts the card once, so the frame (offline dot and look) reads the live
        // state too, not the snapshot it was mounted with.
        let mut shown = entity.clone();
        shown.state = live.get();
        let label = move || match face.action {
            Some(LockAction::Lock) => security_text(lang(), SecurityText::VerbLock),
            Some(LockAction::Unlock) => security_text(lang(), SecurityText::VerbUnlock),
            None => lock_word(lang(), face.word),
        };
        let asking = asking.clone();
        view! {
            <CardFrame entity=shown on_click=on_click pending=pending warn=face.warn>
                <span class="card__note">{move || lock_word(lang(), face.word)}</span>
                <Show
                    when=asking
                    fallback=move || {
                        view! {
                            <div class="card__controls">
                                <button
                                    type="button"
                                    class="btn"
                                    disabled=face.action.is_none()
                                    on:click=move |_| on_press.run(())
                                >
                                    {label}
                                </button>
                            </div>
                        }
                    }
                >
                    <ConfirmBar
                        question=question
                        yes=Signal::derive(move || security_text(lang(), SecurityText::VerbUnlock))
                        on_yes=on_yes
                        on_cancel=Callback::new(move |()| confirming.clear())
                    />
                </Show>
            </CardFrame>
        }
    }
}

#[cfg(test)]
mod tests {
    use nidoo_core::model::GenericState;

    use super::*;

    fn state(raw: &str, availability: Availability) -> EntityState {
        let mut g = GenericState::default();
        g.fields
            .insert("state".into(), GenericValue::Text(raw.into()));
        EntityState {
            value: StateValue::Generic(g),
            availability,
            ..EntityState::default()
        }
    }

    #[test]
    fn face_per_state() {
        use LockAction::*;
        use LockWord::*;
        for (raw, word, action, warn) in [
            ("locked", Locked, Some(Unlock), false),
            ("unlocked", Unlocked, Some(Lock), true),
            ("jammed", Jammed, Some(Lock), true),
            ("open", Open, Some(Lock), true),
            ("opening", Opening, None, true),
            ("locking", Locking, None, false),
            ("unlocking", Unlocking, None, false),
            ("weird", Unknown, None, false),
        ] {
            assert_eq!(
                lock_face(&state(raw, Availability::Online)),
                LockFace { word, action, warn },
                "{raw}"
            );
        }
    }

    #[test]
    fn offline_wins_and_disables() {
        assert_eq!(
            lock_face(&state("locked", Availability::Offline)),
            LockFace {
                word: LockWord::Offline,
                action: None,
                warn: false
            }
        );
    }

    #[test]
    fn no_state_is_unknown() {
        assert_eq!(lock_face(&EntityState::default()).word, LockWord::Unknown);
    }

    #[test]
    fn unlock_is_wanted_only_while_locked() {
        assert!(unlock_still_wanted(&state("locked", Availability::Online)));
        assert!(!unlock_still_wanted(&state(
            "unlocked",
            Availability::Online
        )));
        assert!(!unlock_still_wanted(&state(
            "locked",
            Availability::Offline
        )));
    }

    #[test]
    fn commands_use_the_locked_control() {
        assert_eq!(
            lock_command(LockAction::Lock),
            Command::Control {
                control: "locked".into(),
                value: Some(serde_json::Value::Bool(true))
            }
        );
        assert_eq!(
            lock_command(LockAction::Unlock),
            Command::Control {
                control: "locked".into(),
                value: Some(serde_json::Value::Bool(false))
            }
        );
    }
}
