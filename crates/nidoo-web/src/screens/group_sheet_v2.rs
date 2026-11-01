//! The sheet of a hand-made group, `/group/:id`: its controls, one row per member and the
//! group's settings. It reads the hub's `GroupView`, and drives the controls from the group's own
//! entity.

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::components::A;
use leptos_router::hooks::{use_navigate, use_params_map};
use nidoo_api::{GroupMemberView, GroupSpec, GroupView};
use nidoo_core::model::{Availability, EntityState, Kind};

use crate::api::{delete_group, encode_id, list_groups, set_group, set_override};
use crate::cards::light::power_command;
use crate::cards::{PowerToggle, Sender};
use crate::components::card_frame::open_entity;
use crate::components::card_place::InSheet;
use crate::components::toast::Toasts;
use crate::format::{area_name, display_value, is_on};
use crate::house::House;
use crate::i18n::devices::{DeviceText, device_text, key_value};
use crate::i18n::groups::{
    GroupText, delete_message, delete_question, group_text, group_text_signal, subtitle,
};
use crate::i18n::{Key, text, use_lang};
use crate::icons::{PENCIL, TRASH, kind_icon};
use crate::nav::{BackLink, use_back};
use crate::screens::home::card_for;

/// Whether a member row offers its own toggle: only a member of the group's kind that the
/// registry knows, and only a kind with an on/off command.
fn row_has_toggle(member: &GroupMemberView, kind: Kind) -> bool {
    member.present && !member.mismatch && matches!(kind, Kind::Light | Kind::Switch)
}

/// The spec that republishes `group` with `hide_members` flipped, every other field kept.
fn spec_with_hide(group: &GroupView, hide_members: bool) -> GroupSpec {
    GroupSpec {
        kind: group.kind,
        name: group.name.clone(),
        members: group.members.iter().map(|m| m.id.clone()).collect(),
        hide_members,
    }
}

/// The route `/group/:id`.
#[component]
pub fn GroupSheetV2Screen() -> impl IntoView {
    let toasts = expect_context::<Toasts>();
    let lang = use_lang();
    let params = use_params_map();
    let id = Memo::new(move |_| params.read().get("id").unwrap_or_default());
    // `None` while loading, `Some(None)` when the hub has no such group.
    let group: RwSignal<Option<Option<GroupView>>> = RwSignal::new(None);
    Effect::new(move |_| {
        let id = id.get();
        group.set(None);
        leptos::task::spawn_local(async move {
            match list_groups().await {
                Ok(list) => group.set(Some(list.into_iter().find(|g| g.id == id))),
                Err(e) => {
                    let lead = group_text(untrack(lang), GroupText::CouldntLoad);
                    toasts.push(key_value(untrack(lang), lead, &e.error));
                    group.set(Some(None));
                }
            }
        });
    });

    view! {
        <div class="sheet">
            {move || match group.get() {
                Some(Some(_)) => view! { <GroupBody group=group /> }.into_any(),
                Some(None) => {
                    view! {
                        <header class="sheet__header">
                            <BackLink />
                            <div class="sheet__titles">
                                <h1 class="sheet__title">
                                    {move || group_text(lang(), GroupText::GroupGone)}
                                </h1>
                            </div>
                        </header>
                    }
                        .into_any()
                }
                None => view! { <div class="card card--skeleton" aria-busy="true"></div> }.into_any(),
            }}
        </div>
    }
}

#[component]
fn GroupBody(group: RwSignal<Option<Option<GroupView>>>) -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let current = move || {
        group
            .get()
            .flatten()
            .expect("GroupBody is built for a found group")
    };
    let first = group.get_untracked().flatten().expect("found group");
    let kind = first.kind;
    let entity_id = first.entity.id.clone();

    // The live group entity: state echoes move the controls, the view is the fallback.
    let entity = {
        let entity_id = entity_id.clone();
        let fallback = first.entity.clone();
        Memo::new(move |_| {
            house
                .entities
                .with(|m| m.get(&entity_id).cloned())
                .unwrap_or_else(|| fallback.clone())
        })
    };
    let room = Memo::new(move |_| {
        let area = entity.with(|e| e.area.clone());
        house.areas.with(|areas| {
            areas
                .iter()
                .find(|a| a.id == area)
                .map_or_else(|| area.clone(), |a| area_name(lang(), &a.id, &a.name))
        })
    });

    let rows = move || {
        let members = current().members;
        members
            .into_iter()
            .map(|m| member_row(house, kind, m))
            .collect_view()
    };

    view! {
        <header class="sheet__header">
            <BackLink />
            <span class="sheet__icon" inner_html=kind_icon(kind, None)></span>
            <div class="sheet__titles">
                <h1 class="sheet__title">{move || current().name}</h1>
                <p class="sheet__subtitle">
                    {move || subtitle(lang(), &room.get(), kind, current().members.len())}
                </p>
            </div>
        </header>
        <section class="sheet__panel">
            <h2 class="sheet__heading">{move || device_text(lang(), DeviceText::Controls)}</h2>
            <div class="sheet__controls">
                <InSheet>{move || card_for(entity.get())}</InSheet>
            </div>
        </section>
        <section class="sheet__panel">
            <h2 class="sheet__heading">{move || group_text(lang(), GroupText::Members)}</h2>
            <ul class="group__members">{rows}</ul>
        </section>
        <Settings group=group entity=entity />
    }
}

/// One member: its name (opens its sheet), "room · state" and, when it makes sense, its own toggle.
fn member_row(house: House, kind: Kind, member: GroupMemberView) -> impl IntoView {
    let sender = Sender::from_context();
    let lang = use_lang();
    let live = house
        .entities
        .with_untracked(|m| m.get(&member.id).cloned());
    let offline = live
        .as_ref()
        .is_some_and(|e| e.state.availability == Availability::Offline);
    // "room · state", or "Not found": the state word follows the language.
    let detail = {
        let live = live.clone();
        let areas = house.areas;
        move || match &live {
            Some(e) => {
                let room = areas.with(|areas| {
                    areas
                        .iter()
                        .find(|a| a.id == e.area)
                        .map_or_else(|| e.area.clone(), |a| area_name(lang(), &a.id, &a.name))
                });
                format!(
                    "{room} · {}",
                    display_value(lang(), &e.state, e.unit.as_deref())
                )
            }
            None => group_text(lang(), GroupText::MemberNotFound).to_owned(),
        }
    };
    let open = open_entity(&member.id);
    let toggle = row_has_toggle(&member, kind).then(|| {
        let id = member.id.clone();
        let name = member.name.clone();
        let entity_id = member.id.clone();
        let on = move || {
            house
                .entities
                .with(|m| m.get(&entity_id).is_some_and(|e| is_on(&e.state)))
        };
        view! {
            {move || {
                let id = id.clone();
                let on = on();
                let toggle = Callback::new(move |()| {
                    sender.send(&id, power_command(!on), None::<EntityState>)
                });
                view! { <PowerToggle on=on label="" on_toggle=toggle aria_label=name.clone() /> }
            }}
        }
    });
    let chip = member.mismatch.then(
        || view! { <span class="chip">{move || group_text(lang(), GroupText::Mismatch)}</span> },
    );
    view! {
        <li class="group__member" style:opacity=if offline { "0.5" } else { "1" }>
            <button type="button" class="group__member-name" on:click=move |_| open.run(())>
                <span>{member.name.clone()}</span>
                <span class="group__member-state">{detail}</span>
            </button>
            {chip}
            {toggle}
        </li>
    }
}

/// Favorites, Hide members on Home, Edit group and Delete group.
#[component]
fn Settings(
    group: RwSignal<Option<Option<GroupView>>>,
    entity: Memo<nidoo_api::EntityView>,
) -> impl IntoView {
    let house = expect_context::<House>();
    let toasts = expect_context::<Toasts>();
    let lang = use_lang();
    let navigate = use_navigate();
    let back = use_back();
    let confirming = RwSignal::new(false);
    let deleting = RwSignal::new(false);
    let current = move || group.get().flatten().expect("found group");
    let id = current().id.clone();
    let entity_id = entity.with_untracked(|e| e.id.clone());

    let toggle_favorite = Callback::new(move |()| {
        let favorite = !entity.with_untracked(|e| e.ui.favorite);
        let entity_id = entity_id.clone();
        leptos::task::spawn_local(async move {
            let mut patch = nidoo_api::OverridePatch::new();
            patch.insert("favorite".to_owned(), serde_json::json!(favorite));
            match set_override(&entity_id, patch).await {
                Ok(view) => house.entities.update(|m| {
                    m.insert(view.id.clone(), view);
                }),
                Err(_) => toasts.push(group_text(untrack(lang), GroupText::SaveFailed)),
            }
        });
    });

    let toggle_hide = {
        let id = id.clone();
        Callback::new(move |()| {
            let spec = spec_with_hide(&current(), !current().hide_members);
            let id = id.clone();
            leptos::task::spawn_local(async move {
                match set_group(&id, &spec).await {
                    Ok(view) => group.set(Some(Some(view))),
                    Err(_) => toasts.push(group_text(untrack(lang), GroupText::SaveFailed)),
                }
            });
        })
    };

    let delete = {
        let id = id.clone();
        move |_| {
            let id = id.clone();
            let navigate = navigate.clone();
            deleting.set(true);
            leptos::task::spawn_local(async move {
                match delete_group(&id).await {
                    // The group is gone: leave its sheet out of the browser history too.
                    Ok(()) => navigate(
                        &back.get_untracked(),
                        NavigateOptions {
                            replace: true,
                            ..Default::default()
                        },
                    ),
                    Err(_) => {
                        toasts.push(group_text(untrack(lang), GroupText::DeleteFailed));
                        deleting.set(false);
                        confirming.set(false);
                    }
                }
            });
        }
    };

    let edit_href = format!("/groups/{}/edit", encode_id(&id));
    view! {
        <section class="sheet__panel">
            <h2 class="sheet__heading">{move || text(lang(), Key::Settings)}</h2>
            {move || {
                view! {
                    <PowerToggle
                        on=entity.with(|e| e.ui.favorite)
                        label=group_text_signal(GroupText::ShowInFavorites)
                        on_toggle=toggle_favorite
                    />
                    <PowerToggle
                        on=current().hide_members
                        label=group_text_signal(GroupText::HideMembers)
                        on_toggle=toggle_hide
                    />
                }
            }}
            <div class="sheet__actions">
                <A href=edit_href attr:class="btn btn--with-icon">
                    <span class="btn__glyph" inner_html=PENCIL></span>
                    {move || text(lang(), Key::EditGroup)}
                </A>
                <button
                    type="button"
                    class="btn btn--with-icon btn--danger"
                    on:click=move |_| confirming.set(true)
                >
                    <span class="btn__glyph" inner_html=TRASH></span>
                    {move || group_text(lang(), GroupText::DeleteGroup)}
                </button>
            </div>
            <Show when=move || confirming.get()>
                <section
                    class="sheet__panel"
                    role="alertdialog"
                    aria-label=move || group_text(lang(), GroupText::DeleteGroup)
                >
                    <h3 class="sheet__subheading">
                        {move || delete_question(lang(), &current().name)}
                    </h3>
                    <p>{move || delete_message(lang(), &current().used_by_flows)}</p>
                    <button
                        type="button"
                        class="btn"
                        disabled=move || deleting.get()
                        on:click=delete.clone()
                    >
                        {move || text(lang(), Key::Delete)}
                    </button>
                    <button type="button" class="btn" on:click=move |_| confirming.set(false)>
                        {move || device_text(lang(), DeviceText::Cancel)}
                    </button>
                </section>
            </Show>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(mismatch: bool, present: bool) -> GroupMemberView {
        GroupMemberView {
            id: "m".into(),
            name: "M".into(),
            present,
            mismatch,
        }
    }

    #[test]
    fn mismatch_and_missing_members_have_no_toggle() {
        assert!(row_has_toggle(&member(false, true), Kind::Light));
        assert!(!row_has_toggle(&member(true, true), Kind::Light));
        assert!(!row_has_toggle(&member(false, false), Kind::Light));
        assert!(!row_has_toggle(&member(false, true), Kind::Climate));
    }
}
