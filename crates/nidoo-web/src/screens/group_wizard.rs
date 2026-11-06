//! The three-step group wizard: type, members, then name, room and the hide-members switch.
//! `/groups/new` creates a group; `/groups/:id/edit` reopens it with the type locked.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_params_map};
use nidoo_api::{AreaView, EntityView, GroupSpec};
use nidoo_core::model::{Kind, UNASSIGNED_AREA};

use crate::api::{encode_id, list_groups, set_group, set_override};
use crate::cards::PowerToggle;
use crate::components::toast::Toasts;
use crate::entity_picker::{Keep, Mode, PickerBody};
use crate::flow_edit::fold_accents;
use crate::house::House;
use crate::i18n::devices::{DeviceText, device_text};
use crate::i18n::groups::{GroupText, group_text, group_text_signal, kind_label, step_of};
use crate::i18n::{Key, Lang, text, use_lang};
use crate::nav::{BackLink, use_back};

/// The kinds a group can be made of: every controllable kind.
pub const GROUP_KINDS: &[Kind] = &[
    Kind::Light,
    Kind::Switch,
    Kind::Cover,
    Kind::Climate,
    Kind::Number,
    Kind::Select,
    Kind::Button,
    Kind::Vacuum,
    Kind::WaterHeater,
];

/// What a group of `kind` can hold: entities of that (effective) kind, never another group.
fn member_pool(kind: Option<Kind>) -> Keep {
    Arc::new(move |e: &EntityView| Some(e.kind) == kind && !e.id.starts_with("group_"))
}

/// A group id from its name; see [`crate::ambiances::slug_id`] for the rules.
pub fn slug_group_id(name: &str, taken: &BTreeSet<String>) -> String {
    crate::ambiances::slug_id(name, taken, "group")
}

/// `(id, label, area id)` items grouped by room in the hub's room order, each room sorted by
/// label (accents folded), unknown rooms last and unassigned ones under "No room".
pub fn group_by_room(
    lang: Lang,
    items: impl IntoIterator<Item = (String, String, String)>,
    areas: &[AreaView],
) -> Vec<(String, Vec<(String, String)>)> {
    let mut by_area: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for (id, label, area) in items {
        by_area.entry(area).or_default().push((id, label));
    }
    let mut rooms = Vec::new();
    for area in areas {
        if let Some(mut list) = by_area.remove(&area.id) {
            list.sort_by_key(|(_, label)| fold_accents(label));
            rooms.push((crate::format::area_name(lang, &area.id, &area.name), list));
        }
    }
    for (area, mut list) in by_area {
        list.sort_by_key(|(_, label)| fold_accents(label));
        let name = if area.is_empty() {
            device_text(lang, DeviceText::NoRoom).to_owned()
        } else {
            area
        };
        rooms.push((name, list));
    }
    rooms
}

/// The room select's value for a group's area: the hub's catch-all has no option of its own, so
/// it opens as the blank "No room" choice (saving a blank room writes no override, as before).
fn room_choice(area: &str) -> String {
    if area == UNASSIGNED_AREA {
        String::new()
    } else {
        area.to_owned()
    }
}

#[component]
pub fn GroupWizardScreen() -> impl IntoView {
    let house = expect_context::<House>();
    let toasts = expect_context::<Toasts>();
    let lang = use_lang();
    let navigate = use_navigate();
    let back = use_back();
    let params = use_params_map();
    let edit_id: Option<String> = params.read_untracked().get("id").filter(|s| !s.is_empty());
    let editing = edit_id.is_some();

    let step = RwSignal::new(if editing { 2u8 } else { 1u8 });
    let kind = RwSignal::new(None::<Kind>);
    let selected = RwSignal::new(BTreeSet::<String>::new());
    let name = RwSignal::new(String::new());
    let area = RwSignal::new(String::new());
    let hide_members = RwSignal::new(true);
    let loading = RwSignal::new(editing);
    let saving = RwSignal::new(false);

    if let Some(id) = edit_id.clone() {
        let navigate = navigate.clone();
        leptos::task::spawn_local(async move {
            match list_groups().await {
                Ok(groups) => match groups.into_iter().find(|g| g.id == id) {
                    Some(group) => {
                        kind.set(Some(group.kind));
                        selected.set(group.members.iter().map(|m| m.id.clone()).collect());
                        name.set(group.name);
                        area.set(room_choice(&group.entity.area));
                        hide_members.set(group.hide_members);
                        loading.set(false);
                    }
                    None => {
                        toasts.push(group_text(untrack(lang), GroupText::GroupNotFound));
                        navigate(&back.get_untracked(), Default::default());
                    }
                },
                Err(e) => toasts.push(e.error),
            }
        });
    }

    let members = Signal::derive(move || selected.with(|s| s.iter().cloned().collect::<Vec<_>>()));
    let toggle = Callback::new(move |id: Option<String>| {
        if let Some(id) = id {
            selected.update(|s| {
                if !s.remove(&id) {
                    s.insert(id);
                }
            });
        }
    });

    let can_next = move || match step.get() {
        1 => kind.get().is_some(),
        2 => !selected.with(BTreeSet::is_empty),
        _ => !name.with(|n| n.trim().is_empty()) && !saving.get(),
    };

    let submit = StoredValue::new_local({
        let edit_id = edit_id.clone();
        move || {
            let Some(kind) = kind.get_untracked() else {
                return;
            };
            let spec = GroupSpec {
                kind,
                name: name.get_untracked().trim().to_owned(),
                members: selected.get_untracked().into_iter().collect(),
                hide_members: hide_members.get_untracked(),
            };
            let room = area.get_untracked();
            let edit_id = edit_id.clone();
            let navigate = navigate.clone();
            saving.set(true);
            leptos::task::spawn_local(async move {
                let id = match edit_id {
                    Some(id) => id,
                    None => match list_groups().await {
                        Ok(groups) => {
                            let taken = groups.into_iter().map(|g| g.id).collect();
                            slug_group_id(&spec.name, &taken)
                        }
                        Err(e) => {
                            toasts.push(e.error);
                            saving.set(false);
                            return;
                        }
                    },
                };
                match set_group(&id, &spec).await {
                    Ok(group) => {
                        if !room.is_empty() && room != group.entity.area {
                            let mut patch = nidoo_api::OverridePatch::new();
                            patch.insert("area".to_owned(), room.into());
                            if let Err(e) = set_override(&group.entity.id, patch).await {
                                toasts.push(e.error);
                            }
                        }
                        navigate(&format!("/group/{}", encode_id(&id)), Default::default());
                    }
                    Err(e) => toasts.push(e.error),
                }
                saving.set(false);
            });
        }
    });

    let area_options = move || {
        house
            .areas
            .get()
            .into_iter()
            .filter(|a| a.id != UNASSIGNED_AREA)
            .map(|a| {
                let value = a.id.clone();
                view! {
                    <option value=value>
                        {move || crate::format::area_name(lang(), &a.id, &a.name)}
                    </option>
                }
            })
            .collect_view()
    };

    view! {
        <div class="devices">
            <BackLink />
            <h1 class="devices__title">
                {move || text(lang(), if editing { Key::EditGroup } else { Key::NewGroup })}
            </h1>
            <Show when=move || !loading.get() fallback=move || view! { <p>{move || group_text(lang(), GroupText::Loading)}</p> }>
                <section class="sheet__panel wizard">
                    <div class="wizard__head">
                        <h2 class="sheet__heading">
                            {move || {
                                group_text(
                                    lang(),
                                    match step.get() {
                                        1 => GroupText::StepType,
                                        2 => GroupText::StepMembers,
                                        _ => GroupText::StepNameRoom,
                                    },
                                )
                            }}
                        </h2>
                        <div class="wizard__step" aria-label=move || group_text(lang(), GroupText::Progress)>
                            <span class="wizard__step-count">
                                {move || step_of(lang(), step.get(), 3)}
                            </span>
                            <span class="wizard__pips" aria-hidden="true">
                                <span class:lit=move || { step.get() >= 1 }></span>
                                <span class:lit=move || { step.get() >= 2 }></span>
                                <span class:lit=move || { step.get() >= 3 }></span>
                            </span>
                        </div>
                    </div>
                    {move || match step.get() {
                        1 => {
                            view! {
                                <div class="card__controls card__controls--grid">
                                    {GROUP_KINDS
                                        .iter()
                                        .map(|&k| {
                                            view! {
                                                <button
                                                    type="button"
                                                    class="card"
                                                    class:card--on=move || kind.get() == Some(k)
                                                    aria-pressed=move || (kind.get() == Some(k)).to_string()
                                                    disabled=editing
                                                    on:click=move |_| {
                                                        if kind.get_untracked() != Some(k) {
                                                            selected.set(BTreeSet::new());
                                                        }
                                                        kind.set(Some(k));
                                                    }
                                                >
                                                    <span class="card__name">{move || kind_label(lang(), k)}</span>
                                                </button>
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            }
                                .into_any()
                        }
                        2 => {
                            view! {
                                <PickerBody
                                    mode=Mode::Multi
                                    inline=true
                                    current=members
                                    pool=Some(member_pool(kind.get_untracked()))
                                    on_pick=toggle
                                />
                            }
                                .into_any()
                        }
                        _ => {
                            view! {
                                <label class="field">
                                    <span class="field__caption">
                                        {move || device_text(lang(), DeviceText::Name)}
                                    </span>
                                    <input
                                        class="field__input"
                                        type="text"
                                        aria-label=move || group_text(lang(), GroupText::GroupName)
                                        prop:value=move || name.get()
                                        on:input=move |ev| name.set(event_target_value(&ev))
                                    />
                                </label>
                                <label class="field">
                                    <span class="field__caption">
                                        {move || device_text(lang(), DeviceText::Room)}
                                    </span>
                                    <select
                                        class="field__input field__select"
                                        prop:value=move || area.get()
                                        on:change=move |ev| area.set(event_target_value(&ev))
                                    >
                                        <option value="">{move || device_text(lang(), DeviceText::NoRoom)}</option>
                                        {area_options()}
                                    </select>
                                </label>
                                <PowerToggle
                                    on=hide_members.get()
                                    label=group_text_signal(GroupText::HideMembers)
                                    on_toggle=Callback::new(move |()| hide_members.update(|v| *v = !*v))
                                />
                            }
                                .into_any()
                        }
                    }}
                    <div class="sheet__actions">
                        <button
                            type="button"
                            class="btn btn--with-icon"
                            disabled=move || step.get() == if editing { 2 } else { 1 }
                            on:click=move |_| step.update(|s| *s = s.saturating_sub(1).max(1))
                        >
                            {move || group_text(lang(), GroupText::Back)}
                        </button>
                        <button
                            type="button"
                            class="btn btn--with-icon btn--primary"
                            disabled=move || !can_next()
                            on:click=move |_| {
                                if step.get_untracked() < 3 {
                                    step.update(|s| *s += 1);
                                } else {
                                    submit.with_value(|f| f());
                                }
                            }
                        >
                            {move || {
                                group_text(
                                    lang(),
                                    if step.get() < 3 {
                                        GroupText::Next
                                    } else if editing {
                                        GroupText::SaveGroup
                                    } else {
                                        GroupText::CreateGroup
                                    },
                                )
                            }}
                        </button>
                    </div>
                </section>
            </Show>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_core::model::{Capabilities, EntityCategory, EntityState, UiOverride};

    fn taken(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn slug_folds_accents_and_replaces_separators() {
        let none = BTreeSet::new();
        assert_eq!(
            slug_group_id("Lumières Salon-Est.1", &none),
            "lumieres_salon_est_1"
        );
        assert_eq!(slug_group_id("A  b!?", &none), "a__b");
        assert_eq!(slug_group_id("!!!", &none), "group");
    }

    #[test]
    fn slug_truncates_to_48() {
        let long = "x".repeat(80);
        assert_eq!(slug_group_id(&long, &BTreeSet::new()).len(), 48);
    }

    #[test]
    fn slug_suffixes_on_collision_within_the_limit() {
        assert_eq!(slug_group_id("Salon", &taken(&["salon"])), "salon_2");
        assert_eq!(
            slug_group_id("Salon", &taken(&["salon", "salon_2"])),
            "salon_3"
        );
        let long = "x".repeat(80);
        let full = "x".repeat(48);
        let id = slug_group_id(&long, &taken(&[full.as_str()]));
        assert_eq!(id.len(), 48);
        assert!(id.ends_with("_2"));
    }

    #[test]
    fn group_kinds_are_the_controllable_ones() {
        for k in [
            Kind::Sensor,
            Kind::BinarySensor,
            Kind::AlarmPanel,
            Kind::Generic,
            // A media player never joins a group.
            Kind::MediaPlayer,
        ] {
            assert!(!GROUP_KINDS.contains(&k));
        }
        assert_eq!(GROUP_KINDS.len(), Kind::ALL.len() - 5);
    }

    fn entity(id: &str, kind: Kind, protocol: Kind, area: &str) -> EntityView {
        EntityView {
            id: id.into(),
            device_id: None,
            name: id.into(),
            device_name: None,
            label: None,
            kind,
            component: kind.component().to_string(),
            protocol_kind: protocol,
            device_class: None,
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: area.into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride::default(),
            state: EntityState::default(),
        }
    }

    #[test]
    fn the_pool_is_the_effective_kind_and_never_a_group() {
        let pool = member_pool(Some(Kind::Light));
        assert!(pool(&entity("lamp", Kind::Light, Kind::Light, "a")));
        assert!(pool(&entity("plug", Kind::Light, Kind::Switch, "a")));
        assert!(!pool(&entity("fan", Kind::Switch, Kind::Switch, "a")));
        assert!(!pool(&entity("group_all", Kind::Light, Kind::Light, "a")));
        assert!(!member_pool(None)(&entity(
            "lamp",
            Kind::Light,
            Kind::Light,
            "a"
        )));
    }

    #[test]
    fn a_group_in_the_catch_all_room_opens_with_no_room() {
        assert_eq!(room_choice(UNASSIGNED_AREA), "");
        assert_eq!(room_choice("salon"), "salon");
    }
}
