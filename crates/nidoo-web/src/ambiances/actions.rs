//! What the ambiance screens share: applying a mood with its toast, and adding the examples.

use std::collections::BTreeSet;

use leptos::prelude::*;
use nidoo_api::AmbianceView;

use super::{examples, failed_count, next_order};
use crate::api::ambiances::{apply_ambiance, put_ambiance};
use crate::components::toast::Toasts;
use crate::house::House;
use crate::i18n::{self, Lang};

/// Applies ambiance `id` and toasts the outcome: done, how many devices did not respond, or the
/// hub's error, and keeps the report for "Last run". Returns whether the request succeeded. Toasts are text only, so the partial
/// message has no link; the report stays reachable from the page.
pub async fn apply_and_toast(
    house: House,
    id: String,
    name: String,
    toasts: Toasts,
    lang: Lang,
) -> bool {
    match apply_ambiance(&id).await {
        Ok(report) => {
            // The hub also pushes the report; keeping it here covers a missed event.
            house
                .ambiance_reports
                .try_update(|m| _ = m.insert(id.clone(), report.clone()));
            let failed = failed_count(&report);
            toasts.push(if failed == 0 {
                i18n::ambiance_done(lang, &name)
            } else {
                i18n::ambiance_partial(lang, &name, failed)
            });
            true
        }
        Err(e) => {
            toasts.push(e.error);
            false
        }
    }
}

/// Puts `view` in the house list, replacing the entry with its id: the hub's push does the same
/// a moment later, this just keeps the page honest meanwhile.
pub fn upsert(house: House, view: AmbianceView) {
    house.ambiances.update(|list| {
        match list.iter_mut().find(|v| v.id == view.id) {
            Some(slot) => *slot = view,
            None => list.push(view),
        };
    });
}

/// Saves the four examples one by one, after the existing moods. The first error toasts and
/// stops the rest.
pub async fn add_examples(house: House, toasts: Toasts, lang: Lang) {
    let (taken, first_order) = house.ambiances.with_untracked(|list| {
        (
            list.iter().map(|v| v.id.clone()).collect::<BTreeSet<_>>(),
            next_order(list),
        )
    });
    for ambiance in examples(lang, &taken, first_order) {
        match put_ambiance(&ambiance).await {
            Ok(view) => upsert(house, view),
            Err(e) => {
                toasts.push(e.error);
                return;
            }
        }
    }
}
