//! The room group widget: the lights and the covers of one room as a single card, each with
//! its own state line and controls.

use std::collections::BTreeMap;

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::model::Kind;

use crate::cards::group::CoverButtons;
use crate::cards::{PowerToggle, Sender};
use crate::components::card_frame::GROUP_PREFIX;
use crate::groups::{GroupSummary, cover_text, light_text, light_toggle, summary, targets};
use crate::house::House;
use crate::i18n::{WidgetText, use_lang, widget_text};
use crate::icons::HOUSE;
use crate::rooms::shows_on_home;

/// The lights and the covers Home shows for `area`; hand-made groups are left out, their
/// members are already here.
fn room_members(
    entities: &BTreeMap<String, EntityView>,
    area: &str,
) -> (Vec<EntityView>, Vec<EntityView>) {
    let mut lights = Vec::new();
    let mut covers = Vec::new();
    for e in entities.values() {
        if e.area != area || e.id.starts_with(GROUP_PREFIX) || !shows_on_home(e) {
            continue;
        }
        match e.kind {
            Kind::Light => lights.push(e.clone()),
            Kind::Cover => covers.push(e.clone()),
            _ => {}
        }
    }
    (lights, covers)
}

/// What the card shows: one summary per kind the room has.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RoomSummary {
    lights: Option<GroupSummary>,
    covers: Option<GroupSummary>,
}

fn room_summary(entities: &BTreeMap<String, EntityView>, area: &str) -> RoomSummary {
    let (lights, covers) = room_members(entities, area);
    let of = |members: &[EntityView]| (!members.is_empty()).then(|| summary(members, entities));
    RoomSummary {
        lights: of(&lights),
        covers: of(&covers),
    }
}

/// The card's name: the widget's own title when it has one, else the room's.
fn card_name(title: Option<String>, room: &str) -> String {
    title
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| room.to_owned())
}

#[component]
pub fn RoomGroupWidget(
    area_id: String,
    room: String,
    #[prop(into)] title: Signal<Option<String>>,
) -> impl IntoView {
    let house = expect_context::<House>();
    let sender = Sender::from_context();
    let lang = use_lang();
    let name = Memo::new(move |_| card_name(title.get(), &room));
    let state = {
        let area = area_id.clone();
        Memo::new(move |_| house.entities.with(|m| room_summary(m, &area)))
    };

    let on_light = {
        let area = area_id.clone();
        Callback::new(move |()| {
            let Some(s) = state.with_untracked(|s| s.lights.clone()) else {
                return;
            };
            let cmd = light_toggle(&s);
            let ids = house
                .entities
                .with_untracked(|m| targets(&room_members(m, &area).0, &cmd));
            sender.send_group(&ids, cmd);
        })
    };
    let on_cover = {
        let area = area_id;
        Callback::new(move |cmd: nidoo_core::model::Command| {
            let ids = house
                .entities
                .with_untracked(|m| targets(&room_members(m, &area).1, &cmd));
            sender.send_group(&ids, cmd);
        })
    };

    let lights = move || {
        state.get().lights.map(|s| {
            let label = light_text(lang(), &s);
            let (on, disabled) = (s.on > 0, !s.available);
            view! { <PowerToggle on=on label=label on_toggle=on_light disabled=disabled /> }
        })
    };
    let covers = move || {
        state.get().covers.map(|s| {
            let disabled = !s.available;
            view! {
                <div class="widget-room__covers">
                    <span class="widget-room__state">{cover_text(lang(), &s)}</span>
                    <CoverButtons suffix="" on_command=on_cover disabled=disabled />
                </div>
            }
        })
    };

    view! {
        <article class="card widget-room">
            <div class="widget-room__head">
                <span class="widget-room__icon" inner_html=HOUSE></span>
                <span class="widget-room__name" title=name>{name}</span>
            </div>
            <Show
                when=move || state.with(|s| s.lights.is_some() || s.covers.is_some())
                fallback=move || {
                    view! {
                        <p class="widget-room__empty">
                            {widget_text(lang(), WidgetText::NoLightsOrCovers)}
                        </p>
                    }
                }
            >
                {lights}
                {covers}
            </Show>
        </article>
    }
}

#[cfg(test)]
mod tests {
    use nidoo_core::model::{Capabilities, EntityCategory, EntityState, UiOverride};

    use super::*;

    fn ent(id: &str, area: &str, kind: Kind) -> EntityView {
        EntityView {
            id: id.into(),
            device_id: None,
            name: id.into(),
            device_name: None,
            label: None,
            kind,
            component: kind.component().to_string(),
            protocol_kind: kind,
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

    fn house(list: Vec<EntityView>) -> BTreeMap<String, EntityView> {
        list.into_iter().map(|e| (e.id.clone(), e)).collect()
    }

    #[test]
    fn a_room_keeps_its_own_lights_and_covers() {
        let entities = house(vec![
            ent("l1", "salon", Kind::Light),
            ent("l2", "cuisine", Kind::Light),
            ent("c1", "salon", Kind::Cover),
            ent("s1", "salon", Kind::Sensor),
            ent("group_all", "salon", Kind::Light),
        ]);
        let (lights, covers) = room_members(&entities, "salon");
        assert_eq!(
            lights.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["l1"]
        );
        assert_eq!(
            covers.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["c1"]
        );
    }

    #[test]
    fn hidden_entities_are_left_out() {
        let mut hidden = ent("l2", "salon", Kind::Light);
        hidden.ui.hidden = true;
        let entities = house(vec![ent("l1", "salon", Kind::Light), hidden]);
        assert_eq!(room_members(&entities, "salon").0.len(), 1);
    }

    #[test]
    fn the_summary_only_has_the_kinds_the_room_has() {
        let entities = house(vec![ent("l1", "salon", Kind::Light)]);
        let s = room_summary(&entities, "salon");
        assert_eq!(s.lights.map(|l| l.total), Some(1));
        assert_eq!(s.covers, None);
        let empty = room_summary(&entities, "cuisine");
        assert_eq!((empty.lights, empty.covers), (None, None));
    }

    #[test]
    fn the_title_wins_over_the_room_name() {
        assert_eq!(card_name(Some("Living".into()), "Salon"), "Living");
        assert_eq!(card_name(None, "Salon"), "Salon");
        assert_eq!(card_name(Some(String::new()), "Salon"), "Salon");
    }
}
