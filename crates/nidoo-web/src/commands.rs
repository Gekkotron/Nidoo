//! Optimistic commands: show the expected state at once, reconcile when the hub answers.

use std::collections::BTreeSet;

use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_core::model::{Command, EntityState};

use crate::api::send_command;
use crate::components::toast::Toasts;
use crate::house::House;
use crate::i18n::current_lang;
use crate::i18n::errors::{ErrorText, error_text, named_failure};

/// The ids of the entities with a command in flight, provided as a context by the shell. Cards
/// derive their `pending` flag from it.
#[derive(Clone, Copy)]
pub struct Pending(pub RwSignal<BTreeSet<String>>);

impl Pending {
    pub fn new() -> Self {
        Self(RwSignal::new(BTreeSet::new()))
    }

    /// A signal that is true while `id` has a command in flight.
    pub fn signal(&self, id: &str) -> Signal<bool> {
        let set = self.0;
        let id = id.to_owned();
        Signal::derive(move || set.with(|s| s.contains(&id)))
    }

    /// True while any of `ids` has a command in flight; reads the set once (tracked).
    pub fn any<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> bool {
        self.0.with(|s| ids.into_iter().any(|id| s.contains(id)))
    }
}

impl Default for Pending {
    fn default() -> Self {
        Self::new()
    }
}

fn write_state(house: House, id: &str, state: EntityState) {
    house.entities.update(|m| {
        if let Some(entity) = m.get_mut(id) {
            entity.state = state;
        }
    });
}

/// Sends `cmd` to entity `id`.
///
/// `optimistic` is the state the caller expects the command to produce; it is written to
/// `house` immediately, so the card reacts before the hub answers. The entity is marked pending
/// until the reply arrives. Then:
/// - accepted: the echoed state, when there is one, replaces the guess;
/// - refused or failed: the state from before the call is restored (unless a newer state has
///   arrived meanwhile) and a toast carries the reason.
pub fn fire_command(
    house: House,
    id: String,
    cmd: Command,
    toasts: Toasts,
    pending: Pending,
    optimistic: Option<EntityState>,
) {
    let before = house
        .entities
        .with_untracked(|m| m.get(&id).map(|e| e.state.clone()));
    if let Some(guess) = optimistic.clone() {
        write_state(house, &id, guess);
    }
    pending.0.update(|s| {
        s.insert(id.clone());
    });

    spawn_local(async move {
        let outcome = send_command(&id, cmd).await;
        pending.0.update(|s| {
            s.remove(&id);
        });
        let failure = match outcome {
            Ok(reply) if reply.accepted => {
                if let Some(state) = reply.state {
                    write_state(house, &id, state);
                }
                return;
            }
            Ok(reply) => reply
                .reason
                .unwrap_or_else(|| error_text(current_lang(), ErrorText::CommandRefused).into()),
            Err(e) => e.error,
        };
        // Only undo our own guess: a state that moved on since belongs to someone else.
        if let (Some(before), Some(guess)) = (before, optimistic) {
            let untouched = house
                .entities
                .with_untracked(|m| m.get(&id).is_some_and(|e| e.state == guess));
            if untouched {
                write_state(house, &id, before);
            }
        }
        let name = house
            .entities
            .with_untracked(|m| m.get(&id).map(|e| e.name.clone()));
        toasts.push(match name {
            Some(name) => named_failure(current_lang(), &name, &failure),
            None => failure,
        });
    });
}
