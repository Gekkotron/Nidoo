//! The Zigbee devices of the house, with the discovery topics the hub ignored.
//!
//! Admin / diagnostics screen reached from Settings; the top nav no longer links to it, and
//! user-facing groups moved to [`crate::screens::groups`].

use std::collections::{BTreeMap, BTreeSet};

use leptos::prelude::*;
use leptos_router::components::A;
use nidoo_api::{AreaView, DeviceView, EntityView};

use crate::format::{list_label, room_name, wrap_id};
use crate::house::House;
use crate::i18n::devices::{
    DeviceText, availability_text, devices_count, ignored_topics, link_quality_badge,
    use_device_text,
};
use crate::i18n::{Lang, use_lang};
use crate::icons::MAGNIFIER;
use crate::screens::entity_sheet::availability_class;

/// True when the device's name, model, vendor, IEEE address or one of its `rooms` contains
/// `query`, ignoring case. A blank query matches everything.
fn matches(device: &DeviceView, rooms: &[String], query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || [
            Some(device.friendly_name.as_str()),
            device.model.as_deref(),
            device.vendor.as_deref(),
            Some(device.ieee.as_str()),
        ]
        .into_iter()
        .flatten()
        .chain(rooms.iter().map(String::as_str))
        .any(|field| field.to_lowercase().contains(&query))
}

/// The rooms of the device's entities by name, sorted and without repeats.
pub(crate) fn device_rooms(
    device: &DeviceView,
    entities: &BTreeMap<String, EntityView>,
    areas: &[AreaView],
) -> Vec<String> {
    let rooms: BTreeSet<String> = device
        .entity_ids
        .iter()
        .filter_map(|id| entities.get(id))
        .filter_map(|e| room_name(areas, &e.area))
        .collect();
    rooms.into_iter().collect()
}

/// The rooms as one line, "No room" when the device has none.
fn room_line(lang: Lang, rooms: &[String]) -> String {
    if rooms.is_empty() {
        crate::i18n::devices::device_text(lang, DeviceText::NoRoom).to_owned()
    } else {
        rooms.join(", ")
    }
}

/// "Model · Vendor", whichever of the two is known.
fn model_line(device: &DeviceView) -> String {
    let parts: Vec<&str> = [device.model.as_deref(), device.vendor.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    if parts.is_empty() {
        "—".to_owned()
    } else {
        parts.join(" · ")
    }
}

#[component]
fn DeviceRow(device: DeviceView, expanded: RwSignal<Option<String>>) -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let t = use_device_text();
    let id = device.id.clone();
    let open = {
        let id = id.clone();
        Memo::new(move |_| expanded.with(|e| e.as_deref() == Some(id.as_str())))
    };
    let toggle = move |_| {
        let id = id.clone();
        expanded.update(|e| {
            *e = if e.as_deref() == Some(id.as_str()) {
                None
            } else {
                Some(id)
            }
        });
    };
    let room = {
        let device = device.clone();
        move || {
            house.entities.with(|m| {
                house
                    .areas
                    .with(|a| room_line(lang(), &device_rooms(&device, m, a)))
            })
        }
    };
    let ids = device.entity_ids.clone();
    let links = move || {
        house.entities.with(|m| {
            ids.iter()
                .filter_map(|id| m.get(id).cloned())
                .collect::<Vec<_>>()
        })
    };
    let entity_count = device.entity_ids.len();
    view! {
        <li class="device" class:device--open=move || open.get()>
            <button
                type="button"
                class="device__head"
                aria-expanded=move || open.get().to_string()
                on:click=toggle
            >
                <span
                    class=availability_class(device.availability)
                    title=move || availability_text(lang(), device.availability)
                ></span>
                <span class="device__titles">
                    <span class="device__name">{wrap_id(&device.friendly_name)}</span>
                    <span class="device__room">{room}</span>
                    <span class="device__model">{model_line(&device)}</span>
                </span>
                <span class="device__badges">
                    {device
                        .update
                        .clone()
                        .filter(|u| u.available)
                        .map(|u| {
                            view! {
                                <span class="chip-badge device__update-badge" title=move || t(DeviceText::FirmwareUpdate)>
                                    {format!("v{}", u.latest.unwrap_or_default())}
                                </span>
                            }
                        })}
                    {device
                        .battery
                        .map(|b| view! { <span class="chip-badge">{format!("{b}\u{a0}%")}</span> })}
                    {device
                        .linkquality
                        .map(|q| view! { <span class="chip-badge">{move || link_quality_badge(q)}</span> })}
                    <span class="chip-badge" title=move || t(DeviceText::Entities)>
                        {entity_count}
                    </span>
                </span>
            </button>
            <Show when=move || open.get()>
                <div class="device__body">
                    <ul class="sheet__links">
                        {links()
                            .into_iter()
                            .map(|e| {
                                let href = format!("/entity/{}", crate::api::encode_id(&e.id));
                                view! {
                                    <li>
                                        <A href=href>{move || wrap_id(&list_label(lang(), &e))}</A>
                                    </li>
                                }
                            })
                            .collect_view()}
                    </ul>
                </div>
            </Show>
        </li>
    }
}

#[component]
pub fn DevicesScreen() -> impl IntoView {
    let house = expect_context::<House>();
    let query = RwSignal::new(String::new());
    let expanded = RwSignal::new(None::<String>);
    let filtered = Memo::new(move |_| {
        let query = query.get();
        let mut devices: Vec<DeviceView> = house.devices.with(|d| {
            house.entities.with(|m| {
                house.areas.with(|a| {
                    d.iter()
                        .filter(|d| matches(d, &device_rooms(d, m, a), &query))
                        .cloned()
                        .collect()
                })
            })
        });
        devices.sort_by_key(|d| d.friendly_name.to_lowercase());
        devices
    });
    let total = Memo::new(move |_| house.devices.with(Vec::len));
    let lang = use_lang();
    let t = use_device_text();

    view! {
        <div class="devices">
            <h1 class="devices__title">{move || t(DeviceText::Zigbee2mqttDevices)}</h1>
            <label class="devices__search">
                <span class="devices__search-icon" inner_html=MAGNIFIER></span>
                <input
                    class="devices__search-input"
                    type="search"
                    placeholder=move || t(DeviceText::SearchPlaceholder)
                    aria-label=move || t(DeviceText::SearchDevices)
                    prop:value=move || query.get()
                    on:input=move |ev| query.set(event_target_value(&ev))
                />
            </label>
            <p class="devices__count">
                {move || devices_count(lang(), filtered.with(Vec::len), total.get())}
            </p>
            <ul class="device-list">
                <For
                    each=move || filtered.get()
                    key=|d| d.id.clone()
                    children=move |device| view! { <DeviceRow device expanded /> }
                />
            </ul>
            <section class="sheet__panel">
                <h2 class="sheet__heading">
                    {move || ignored_topics(lang(), house.ignored.with(Vec::len))}
                </h2>
                <ul class="ignored-list">
                    <For
                        each=move || house.ignored.get()
                        key=|i| i.topic.clone()
                        children=|item| {
                            view! {
                                <li class="ignored-list__item">
                                    <code class="ignored-list__topic">{wrap_id(&item.topic)}</code>
                                    <span class="ignored-list__reason">{item.reason}</span>
                                </li>
                            }
                        }
                    />
                </ul>
            </section>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::{matches, room_line};
    use crate::i18n::Lang;
    use nidoo_api::DeviceView;
    use nidoo_core::model::Availability;

    fn device() -> DeviceView {
        DeviceView {
            id: "d1".into(),
            ieee: "0x00158D0001ABCDEF".into(),
            friendly_name: "Salon/Lamp".into(),
            model: Some("TS0601".into()),
            vendor: Some("Tuya".into()),
            description: None,
            supported: true,
            interview_state: "SUCCESSFUL".into(),
            power_source: None,
            entity_ids: vec![],
            availability: Availability::Online,
            battery: None,
            linkquality: None,
            update: None,
        }
    }

    #[test]
    fn filter_matches_name_model_vendor_and_ieee_ignoring_case() {
        let d = device();
        assert!(matches(&d, &[], ""));
        assert!(matches(&d, &[], "  "));
        assert!(matches(&d, &[], "salon"));
        assert!(matches(&d, &[], "ts06"));
        assert!(matches(&d, &[], "TUYA"));
        assert!(matches(&d, &[], "abcdef"));
        assert!(!matches(&d, &[], "ikea"));
    }

    #[test]
    fn filter_tolerates_missing_model_and_vendor() {
        let d = DeviceView {
            model: None,
            vendor: None,
            ..device()
        };
        assert!(!matches(&d, &[], "tuya"));
        assert!(matches(&d, &[], "lamp"));
    }

    #[test]
    fn filter_matches_the_device_rooms() {
        let d = device();
        let rooms = vec!["Bedroom".to_owned()];
        assert!(matches(&d, &rooms, "bed"));
        assert!(!matches(&d, &[], "bed"));
    }

    #[test]
    fn room_line_joins_rooms_or_says_no_room() {
        assert_eq!(room_line(Lang::En, &[]), "No room");
        assert_eq!(room_line(Lang::Fr, &[]), "Sans pièce");
        assert_eq!(
            room_line(Lang::En, &["Bedroom".to_owned(), "Office".to_owned()]),
            "Bedroom, Office"
        );
    }
}
