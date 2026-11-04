//! The Home screen: every room's cards, then the latest alerts.

use leptos::prelude::*;
use nidoo_api::{EntityView, Span};
use nidoo_core::model::{Kind, SOUND_DEVICE_PREFIX, SwitchType};
use nidoo_core::types::Placement as TypePlacement;
use std::collections::HashSet;

use crate::ambiances::row::HomeAmbiances;
use crate::cameras::{CameraList, house_cameras};
use crate::cards::alarm::AlarmPanelCard;
use crate::cards::binary_sensor::BinarySensorCard;
use crate::cards::button::ButtonCard;
use crate::cards::chime::ChimeCard;
use crate::cards::climate::ClimateCard;
use crate::cards::cover::CoverCard;
use crate::cards::door::DoorCard;
use crate::cards::generic::GenericCard;
use crate::cards::group::{
    ButtonGroupCard, ClimateGroupCard, CoverGroupEntityCard, LightGroupEntityCard, NumberGroupCard,
    SelectGroupCard, SwitchGroupCard, VacuumGroupCard, WaterHeaterGroupCard,
};
use crate::cards::light::LightCard;
use crate::cards::lock::LockCard;
use crate::cards::number::NumberCard;
use crate::cards::satellite::SatelliteCard;
use crate::cards::select::SelectCard;
use crate::cards::sensor::{SensorCard, companion};
use crate::cards::siren::SirenCard;
use crate::cards::switch::SwitchCard;
use crate::cards::vacuum::VacuumCard;
use crate::cards::water_heater::WaterHeaterCard;
use crate::components::alert_row::RecentAlerts;
use crate::components::card_frame::GROUP_PREFIX;
use crate::dashboards::camera_widget::CameraWidget;
use crate::home_header::view::HomeHeader;
use crate::i18n::{DashText, dash_text, use_lang};
use crate::rooms::{generic_placement, use_group_refresh, use_loaded, use_room_chips};
use crate::screens::room::{RoomSection, RoomsSkeleton};

fn is_sensor_of_class(e: &EntityView, class: &str) -> bool {
    e.kind == Kind::Sensor && e.device_class.as_deref() == Some(class)
}

/// `shown` without the humidity (or soil moisture) sensors whose device's temperature sensor is
/// also in `shown`: the temperature card carries that reading.
pub(crate) fn without_paired_companions(mut shown: Vec<&EntityView>) -> Vec<&EntityView> {
    let folded: HashSet<&str> = shown
        .iter()
        .copied()
        .filter(|e| is_sensor_of_class(e, "temperature"))
        .filter_map(|e| e.device_id.as_deref())
        .filter_map(|d| companion(shown.iter().copied(), d))
        .map(|e| e.id.as_str())
        .collect();
    shown.retain(|e| !folded.contains(e.id.as_str()));
    shown
}

/// The card component for an entity's kind.
pub fn card_for(entity: EntityView) -> AnyView {
    if entity.id.starts_with(GROUP_PREFIX) {
        match entity.kind {
            Kind::Switch => return view! { <SwitchGroupCard entity=entity /> }.into_any(),
            Kind::Climate => return view! { <ClimateGroupCard entity=entity /> }.into_any(),
            Kind::WaterHeater => {
                return view! { <WaterHeaterGroupCard entity=entity /> }.into_any();
            }
            Kind::Number => return view! { <NumberGroupCard entity=entity /> }.into_any(),
            Kind::Select => return view! { <SelectGroupCard entity=entity /> }.into_any(),
            Kind::Button => return view! { <ButtonGroupCard entity=entity /> }.into_any(),
            Kind::Vacuum => return view! { <VacuumGroupCard entity=entity /> }.into_any(),
            Kind::Light => return view! { <LightGroupEntityCard entity=entity /> }.into_any(),
            Kind::Cover => return view! { <CoverGroupEntityCard entity=entity /> }.into_any(),
            _ => {}
        }
    }
    match entity.kind {
        Kind::Light => view! { <LightCard entity=entity /> }.into_any(),
        Kind::Switch => match SwitchType::of(entity.protocol_kind, &entity.ui) {
            Some(SwitchType::Door) => view! { <DoorCard entity=entity /> }.into_any(),
            Some(SwitchType::Siren) => view! { <SirenCard entity=entity /> }.into_any(),
            Some(SwitchType::Chime) => view! { <ChimeCard entity=entity /> }.into_any(),
            _ => view! { <SwitchCard entity=entity /> }.into_any(),
        },
        Kind::Cover => view! { <CoverCard entity=entity /> }.into_any(),
        Kind::Climate => view! { <ClimateCard entity=entity /> }.into_any(),
        Kind::Sensor => view! { <SensorCard entity=entity /> }.into_any(),
        Kind::BinarySensor => view! { <BinarySensorCard entity=entity /> }.into_any(),
        Kind::Number
            if entity
                .device_id
                .as_deref()
                .is_some_and(|d| d.starts_with(SOUND_DEVICE_PREFIX)) =>
        {
            view! { <SatelliteCard entity=entity /> }.into_any()
        }
        Kind::Number => view! { <NumberCard entity=entity /> }.into_any(),
        Kind::Select => view! { <SelectCard entity=entity /> }.into_any(),
        Kind::Button => view! { <ButtonCard entity=entity /> }.into_any(),
        Kind::AlarmPanel => view! { <AlarmPanelCard entity=entity /> }.into_any(),
        Kind::Vacuum => view! { <VacuumCard entity=entity /> }.into_any(),
        Kind::WaterHeater => view! { <WaterHeaterCard entity=entity /> }.into_any(),
        Kind::Generic if entity.component == "lock" => {
            view! { <LockCard entity=entity /> }.into_any()
        }
        Kind::Generic => match generic_placement(&entity) {
            TypePlacement::Card => view! { <GenericCard entity=entity /> }.into_any(),
            // Signal, target and update entities do not show on Home.
            _ => ().into_any(),
        },
    }
}

/// The house overview: every room one under the other (Favorites first, Unassigned last), then
/// the latest alerts.
#[component]
pub fn HomeScreen() -> impl IntoView {
    let chips = use_room_chips();
    let loaded = use_loaded();
    use_group_refresh();
    // Back from a card's sheet comes with `#room-<id>` (see `RoomSection`). The router looks the
    // fragment up too early to find the room, so the overview brings it into view once drawn.
    Effect::new(move |_| {
        if loaded.get() {
            request_animation_frame(scroll_to_fragment);
        }
    });

    view! {
        <Show when=move || loaded.get() fallback=|| view! { <RoomsSkeleton /> }>
            <HomeHeader />
            <HomeAmbiances />
            <HouseCameras />
            <For
                each=move || chips.get()
                key=|c| c.id.clone()
                children=|area| view! { <RoomSection id=area.id anchored=true /> }
            />
            <RecentAlerts />
        </Show>
    }
}

/// Every Frigate camera, one live tile each, while `[frigate] live` is on; nothing otherwise.
#[component]
fn HouseCameras() -> impl IntoView {
    let lang = use_lang();
    let list = use_context::<CameraList>().unwrap_or_default();
    let ids = Memo::new(move |_| list.0.with(house_cameras));
    view! {
        <Show when=move || ids.with(|v| !v.is_empty())>
            <section class="room house-cameras">
                <div class="room__heading">
                    <h1 class="room__title">{move || dash_text(lang(), DashText::CamerasLabel)}</h1>
                </div>
                <div class="house-cameras__grid">
                    <For
                        each=move || ids.get()
                        key=|id| id.clone()
                        children=|id| view! { <CameraWidget cameras=vec![id] span=Span::X2x1 /> }
                    />
                </div>
            </section>
        </Show>
    }
}

/// Scrolls the element the address's fragment names to the top of the view, at once (so reduced
/// motion has nothing to tone down). The fragment is decoded once, as the router does.
pub(crate) fn scroll_to_fragment() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let hash = window.location().hash().unwrap_or_default();
    let Some(fragment) = hash.strip_prefix('#').filter(|f| !f.is_empty()) else {
        return;
    };
    let id = js_sys::decode_uri(fragment)
        .ok()
        .and_then(|d| d.as_string())
        .unwrap_or_else(|| fragment.to_owned());
    if let Some(el) = window.document().and_then(|d| d.get_element_by_id(&id)) {
        el.scroll_into_view();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::groups::{HomeItem, home_items};
    use crate::rooms::FAVORITES;
    use nidoo_api::EntityView;
    use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};

    /// The ids of the single cards `home_items` gives under `chip` (no groups expected).
    fn entity_ids<'a>(entities: impl Iterator<Item = &'a EntityView>, chip: &str) -> Vec<String> {
        home_items(entities, &[], chip, &BTreeMap::new())
            .into_iter()
            .map(|i| match i {
                HomeItem::Entity(id) => id,
            })
            .collect()
    }

    fn entity(id: &str, kind: Kind, device_class: Option<&str>, ui: UiOverride) -> EntityView {
        EntityView {
            id: id.into(),
            device_id: None,
            name: id.into(),
            device_name: None,
            label: None,
            kind,
            component: kind.component().to_string(),
            protocol_kind: kind,
            device_class: device_class.map(Into::into),
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: "living".into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui,
            state: EntityState::default(),
        }
    }

    fn role(role: &str, pinned: bool, hidden: bool) -> UiOverride {
        UiOverride {
            energy_role: Some(role.into()),
            home_pinned: pinned,
            hidden,
            ..Default::default()
        }
    }

    #[test]
    fn energy_and_security_entities_leave_home() {
        let all = [
            entity("lamp", Kind::Light, None, UiOverride::default()),
            entity(
                "grid",
                Kind::Sensor,
                Some("power"),
                role("grid", false, false),
            ),
            entity(
                "solar",
                Kind::Sensor,
                Some("power"),
                role("solar", true, false),
            ),
            entity(
                "hidden",
                Kind::Sensor,
                Some("power"),
                role("grid", true, true),
            ),
            entity(
                "door",
                Kind::BinarySensor,
                Some("door"),
                UiOverride::default(),
            ),
            entity(
                "siren",
                Kind::Switch,
                None,
                UiOverride {
                    security_action: true,
                    ..Default::default()
                },
            ),
            entity("plug", Kind::Switch, None, UiOverride::default()),
        ];
        assert_eq!(entity_ids(all.iter(), "living"), ["lamp", "plug", "solar"]);
    }

    fn on_device(mut e: EntityView, device: &str) -> EntityView {
        e.device_id = Some(device.into());
        e
    }

    fn thermometer_parts(prefix: &str, device: &str) -> [EntityView; 2] {
        [
            on_device(
                entity(
                    &format!("{prefix}_t"),
                    Kind::Sensor,
                    Some("temperature"),
                    UiOverride::default(),
                ),
                device,
            ),
            on_device(
                entity(
                    &format!("{prefix}_h"),
                    Kind::Sensor,
                    Some("humidity"),
                    UiOverride::default(),
                ),
                device,
            ),
        ]
    }

    #[test]
    fn humidity_is_folded_into_its_temperature_sensor() {
        let [t, h] = thermometer_parts("th", "dev");
        let lamp = entity("lamp", Kind::Light, None, UiOverride::default());
        let all = [t, h, lamp];
        assert_eq!(entity_ids(all.iter(), "living"), ["lamp", "th_t"]);
    }

    #[test]
    fn soil_moisture_is_folded_into_its_temperature_sensor() {
        let t = on_device(
            entity(
                "soil_t",
                Kind::Sensor,
                Some("temperature"),
                UiOverride::default(),
            ),
            "soil",
        );
        let m = on_device(
            entity(
                "soil_m",
                Kind::Sensor,
                Some("moisture"),
                UiOverride::default(),
            ),
            "soil",
        );
        let all = [t, m];
        assert_eq!(entity_ids(all.iter(), "living"), ["soil_t"]);
    }

    #[test]
    fn only_the_preferred_companion_is_folded() {
        let [t, h] = thermometer_parts("th", "dev");
        let m = on_device(
            entity(
                "th_m",
                Kind::Sensor,
                Some("moisture"),
                UiOverride::default(),
            ),
            "dev",
        );
        let all = [t, h, m];
        assert_eq!(entity_ids(all.iter(), "living"), ["th_m", "th_t"]);
    }

    #[test]
    fn humidity_stays_without_a_temperature_on_the_same_device() {
        let [t, _] = thermometer_parts("a", "dev_a");
        let [_, h] = thermometer_parts("b", "dev_b");
        let all = [t, h];
        assert_eq!(entity_ids(all.iter(), "living"), ["a_t", "b_h"]);
    }

    #[test]
    fn humidity_without_device_id_stays() {
        let t = entity(
            "t",
            Kind::Sensor,
            Some("temperature"),
            UiOverride::default(),
        );
        let h = entity("h", Kind::Sensor, Some("humidity"), UiOverride::default());
        let all = [t, h];
        assert_eq!(entity_ids(all.iter(), "living"), ["h", "t"]);
    }

    #[test]
    fn favorite_humidity_stays_when_its_temperature_is_not_a_favorite() {
        let [t, mut h] = thermometer_parts("th", "dev");
        h.ui.favorite = true;
        let all = [t, h];
        assert_eq!(entity_ids(all.iter(), FAVORITES), ["th_h"]);
    }
}
