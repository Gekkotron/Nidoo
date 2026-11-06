use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::dashboard::{DashboardHistoryRange, Widget, WidgetConfig, WidgetKind};

use super::camera_widget::CameraWidget;
use super::config_sheet::title_of;
use super::energy_stat::EnergyStatWidget;
use super::flow_button::FlowButtonWidget;
use super::history_widget::HistoryWidget;
use super::library::target_id;
use super::room_group::RoomGroupWidget;
use crate::cards::climate::ClimateCard;
use crate::cards::cover::CoverCard;
use crate::cards::light::LightCard;
use crate::cards::lock::LockCard;
use crate::cards::media_player::MediaPlayerCard;
use crate::cards::sensor::SensorCard;
use crate::cards::water_heater::WaterHeaterCard;
use crate::house::House;
use crate::i18n::{use_lang, widget_kind_word, widget_placeholder};

/// What a widget's card is built from, apart from the title and a Camera widget's list (both
/// followed live): its target and a history chart's range.
pub fn shown(config: &WidgetConfig) -> (Option<String>, Option<DashboardHistoryRange>) {
    let range = match config {
        WidgetConfig::History { range, .. } => Some(*range),
        _ => None,
    };
    (target_id(config).map(str::to_owned), range)
}

/// Picks the card for a widget. An entity card takes a copy of its entity, so it is rebuilt on
/// each change of that entity, as a room's cards are; the history chart only waits for its
/// entity to be known (a dashboard opened by its address paints before the snapshot) and then
/// refreshes itself.
///
/// `current` is the widget as the dashboard holds it now (the grid keeps a tile mounted across
/// edits); the Camera widget follows its span and camera list, which pick its layout.
#[component]
pub fn WidgetView(
    widget: Widget,
    #[prop(optional, into)] current: Option<Signal<Widget>>,
) -> impl IntoView {
    let house = expect_context::<House>();
    // Read live: the grid keeps the tile mounted while the config sheet edits the title.
    let title = {
        let initial = title_of(&widget);
        Signal::derive(move || match current {
            Some(current) => current.with(title_of),
            None => initial.clone(),
        })
    };
    match (widget.kind, widget.config) {
        (WidgetKind::Camera, WidgetConfig::Cameras { cameras, .. }) => {
            let span = widget.span;
            let shape = Memo::new(move |_| match current {
                Some(current) => current.with(|w| match &w.config {
                    WidgetConfig::Cameras { cameras, .. } => (cameras.clone(), w.span),
                    _ => (Vec::new(), w.span),
                }),
                None => (cameras.clone(), span),
            });
            (move || {
                let (cameras, span) = shape.get();
                view! { <CameraWidget cameras=cameras span=span /> }
            })
            .into_any()
        }
        (kind, WidgetConfig::Entity { entity_id, .. }) => {
            card_widget(house, entity_id.clone(), move |entity| match kind {
                WidgetKind::Light => view! { <LightCard entity=entity /> }.into_any(),
                WidgetKind::Cover => view! { <CoverCard entity=entity /> }.into_any(),
                WidgetKind::Climate => view! { <ClimateCard entity=entity /> }.into_any(),
                WidgetKind::Sensor => view! { <SensorCard entity=entity /> }.into_any(),
                WidgetKind::WaterHeater => view! { <WaterHeaterCard entity=entity /> }.into_any(),
                WidgetKind::Lock => view! { <LockCard entity=entity /> }.into_any(),
                WidgetKind::MediaPlayer => view! { <MediaPlayerCard entity=entity /> }.into_any(),
                // A kind that takes no entity (an invalid widget): a labelled placeholder.
                _ => {
                    let lang = use_lang();
                    let id = entity_id.clone();
                    let label = Signal::derive(move || widget_placeholder(lang(), kind, &id));
                    view! { <Unavailable label=label /> }.into_any()
                }
            })
        }
        (WidgetKind::HistoryChart, WidgetConfig::History { entity_id, range }) => {
            entity_widget(house, entity_id, move |entity| {
                view! { <HistoryWidget entity=entity range=range /> }.into_any()
            })
        }
        (WidgetKind::RoomGroup, WidgetConfig::Area { area_id, .. }) => {
            let room = {
                let id = area_id.clone();
                Memo::new(move |_| {
                    house
                        .areas
                        .with(|a| a.iter().find(|a| a.id == id).map(|a| a.name.clone()))
                })
            };
            (move || match room.get() {
                Some(room) => view! {
                    <RoomGroupWidget area_id=area_id.clone() room=room title=title />
                }
                .into_any(),
                None => view! { <Unavailable label=area_id.clone() /> }.into_any(),
            })
            .into_any()
        }
        (WidgetKind::FlowButton, WidgetConfig::Flow { flow_id, .. }) => {
            let flow = {
                let id = flow_id.clone();
                Memo::new(move |_| {
                    house.flows.with(|flows| {
                        flows
                            .iter()
                            .find(|f| f.definition.id == id)
                            .map(|f| (f.definition.name.clone(), f.invalid.clone()))
                    })
                })
            };
            (move || match flow.get() {
                Some((flow_name, invalid)) => view! {
                    <FlowButtonWidget
                        flow_id=flow_id.clone()
                        flow_name=flow_name
                        title=title
                        invalid=invalid
                    />
                }
                .into_any(),
                None => view! { <Unavailable label=flow_id.clone() /> }.into_any(),
            })
            .into_any()
        }
        (WidgetKind::EnergyStat, WidgetConfig::Energy { metric, .. }) => {
            view! { <EnergyStatWidget metric=metric title=title /> }.into_any()
        }
        (kind, _) => {
            let lang = use_lang();
            let label = Signal::derive(move || widget_kind_word(lang(), kind).to_owned());
            view! { <Unavailable label=label /> }.into_any()
        }
    }
}

/// The entity `id` as the house holds it now, `None` while it has none.
fn tracked_entity(house: House, id: String) -> Memo<Option<EntityView>> {
    Memo::new(move |_| house.entities.with(|m| m.get(&id).cloned()))
}

/// `render` over the entity `entity_id`, run again on each of its changes, or the placeholder
/// while the house has none.
fn card_widget(
    house: House,
    entity_id: String,
    render: impl Fn(EntityView) -> AnyView + Send + Sync + 'static,
) -> AnyView {
    let entity = tracked_entity(house, entity_id.clone());
    (move || match entity.get() {
        Some(entity) => render(entity),
        None => view! { <Unavailable label=entity_id.clone() /> }.into_any(),
    })
    .into_any()
}

/// `render` over the entity `entity_id` once the house knows it, or the placeholder while it
/// has none. Only its presence is tracked: what `render` builds follows the entity itself.
fn entity_widget(
    house: House,
    entity_id: String,
    render: impl Fn(EntityView) -> AnyView + Send + Sync + 'static,
) -> AnyView {
    let present = {
        let id = entity_id.clone();
        Memo::new(move |_| house.entities.with(|m| m.contains_key(&id)))
    };
    (move || {
        let found = present
            .get()
            .then(|| {
                house
                    .entities
                    .with_untracked(|m| m.get(&entity_id).cloned())
            })
            .flatten();
        match found {
            Some(entity) => render(entity),
            None => view! { <Unavailable label=entity_id.clone() /> }.into_any(),
        }
    })
    .into_any()
}

#[component]
pub fn Unavailable(#[prop(into)] label: Signal<String>) -> impl IntoView {
    view! {
        <div class="widget-unavailable">
            <span class="widget-unavailable-icon">"⚠"</span>
            <span class="widget-unavailable-label">{move || label.get()}</span>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use nidoo_api::WsEvent;
    use nidoo_core::model::{
        Capabilities, EntityCategory, EntityState, Kind, StateValue, UiOverride,
    };

    use super::*;

    fn sensor(id: &str, value: f64) -> EntityView {
        EntityView {
            id: id.into(),
            device_id: None,
            name: "Temperature".into(),
            device_name: None,
            label: None,
            kind: Kind::Sensor,
            component: Kind::Sensor.component().to_string(),
            protocol_kind: Kind::Sensor,
            device_class: None,
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: "salon".into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride::default(),
            state: EntityState {
                value: StateValue::Number(value),
                ..Default::default()
            },
        }
    }

    fn value_of(entity: Option<EntityView>) -> Option<StateValue> {
        entity.map(|e| e.state.value)
    }

    #[test]
    fn a_card_widget_follows_its_entity_state() {
        let house = House::new();
        let entity = tracked_entity(house, "s".into());
        assert_eq!(value_of(entity.get_untracked()), None);
        house.apply(WsEvent::EntityUpserted(sensor("s", 1.0)));
        assert_eq!(
            value_of(entity.get_untracked()),
            Some(StateValue::Number(1.0))
        );
        house.apply(WsEvent::StateChanged {
            id: "s".into(),
            state: sensor("s", 2.0).state,
        });
        assert_eq!(
            value_of(entity.get_untracked()),
            Some(StateValue::Number(2.0))
        );
    }
}
