//! The Climate screen: the room list, the selected room's panel, the house settings behind a ⚙
//! and the engine's banners.
//!
//! Only the zone list is keyed by zone id; the panels, the settings popup and their local state
//! live outside the closures that react to a refetch, so a refresh never resets them. Every
//! room's panel stays mounted and only the selected one shows, so switching rooms keeps an edit
//! in progress too.

use chrono_tz::Tz;
use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_climate::view::{ClimateView, ZoneView};

use crate::api::climate as api;
use crate::climate::room_list::RoomList;
use crate::climate::room_panel::RoomPanel;
use crate::climate::settings_sheet::SettingsSheet;
use crate::climate::{parse_zone, pick_zone};
use crate::components::popup::Popup;
use crate::house::House;
use crate::i18n::climate::{ClimateText, use_climate_text};
use crate::i18n::{Key, use_text};

/// The zone ids of `view` in display order.
pub fn zone_ids(view: Option<&ClimateView>) -> Vec<String> {
    view.map(|v| v.zones.iter().map(|z| z.zone.clone()).collect())
        .unwrap_or_default()
}

/// The zone `id` in `view`.
pub fn find_zone(view: Option<&ClimateView>, id: &str) -> Option<ZoneView> {
    view?.zones.iter().find(|z| z.zone == id).cloned()
}

#[component]
pub fn ClimateScreen() -> impl IntoView {
    let house = expect_context::<House>();
    let t = use_climate_text();
    let text = use_text();
    let climate = RwSignal::new(None::<ClimateView>);
    let failed = RwSignal::new(None::<String>);
    let settings_open = RwSignal::new(false);

    let reload = Callback::new(move |_: ()| {
        spawn_local(async move {
            match api::get_climate().await {
                Ok(v) => {
                    failed.try_set(None);
                    climate.try_set(Some(v));
                }
                Err(e) => {
                    failed.try_set(Some(e.error));
                }
            }
        });
    });
    // Refetch on mount and whenever the hub says the engine changed.
    Effect::new(move |_| {
        house.climate_rev.track();
        reload.run(());
    });

    let ids = Memo::new(move |_| climate.with(|v| zone_ids(v.as_ref())));
    let selected = RwSignal::new(None::<String>);
    let current = Memo::new(move |_| ids.with(|ids| pick_zone(ids, selected.get().as_deref())));
    // A choice that left the view is forgotten, so a zone coming back under that id does not
    // pull the screen away from the room shown since.
    Effect::new(move |_| {
        let gone = selected.with(|s| s.as_ref().is_some_and(|s| ids.with(|ids| !ids.contains(s))));
        if gone {
            selected.set(None);
        }
    });
    let zones = Signal::derive(move || {
        climate.with(|v| v.as_ref().map(|v| v.zones.clone()).unwrap_or_default())
    });
    // A memo, so a refetch that keeps the time zone redraws no calendar.
    let tz = Memo::new(move |_| {
        climate.with(|v| v.as_ref().map_or(Tz::UTC, |v| parse_zone(&v.timezone)))
    });
    let loaded = move || climate.with(|v| v.is_some());

    view! {
        <div class="climate-screen">
            <header class="sheet__header">
                <div class="sheet__titles">
                    <h1 class="sheet__title">
                        {move || text(Key::Climate)}
                        <span class="badge climate__beta">{move || text(Key::Beta)}</span>
                    </h1>
                    <p class="sheet__subtitle">{move || t(ClimateText::Subtitle)}</p>
                </div>
                // Disabled until the settings are loaded, so the draft never starts from defaults.
                <button
                    type="button"
                    class="climate-screen__gear"
                    aria-label=move || t(ClimateText::HouseSettings)
                    disabled=move || !loaded()
                    on:click=move |_| settings_open.set(true)
                >
                    "⚙"
                </button>
            </header>
            {move || failed.get().map(|e| view! { <p class="badge climate__error">{e}</p> })}
            {move || {
                climate
                    .with(|v| v.as_ref().and_then(|v| v.house_invalid.clone()))
                    .map(|e| view! { <p class="badge">{e}</p> })
            }}
            {move || {
                (loaded() && !climate.with(|v| v.as_ref().is_some_and(|v| v.history)))
                    .then(|| view! {
                        <p class="badge climate__banner">
                            {move || t(ClimateText::HistoryOffBanner)}
                        </p>
                    })
            }}
            // Built when the popup opens from the settings of that moment; a refetch leaves the
            // draft alone.
            <Popup open=settings_open title=Signal::derive(move || text(Key::Settings).to_owned())>
                {move || {
                    let current = climate.get_untracked().map(|v| v.house).unwrap_or_default();
                    view! {
                        <SettingsSheet
                            house=current
                            on_saved=Callback::new(move |_| {
                                settings_open.set(false);
                                reload.run(());
                            })
                            on_close=Callback::new(move |_| settings_open.set(false))
                        />
                    }
                }}
            </Popup>
            <Show when=move || loaded() && ids.with(|i| i.is_empty())>
                <p class="climate-screen__empty">{move || t(ClimateText::NoRoomClimate)}</p>
            </Show>
            <div class="climate-screen__body" hidden=move || ids.with(|i| i.is_empty())>
                <RoomList
                    zones=zones
                    selected=current.into()
                    on_select=Callback::new(move |id| selected.set(Some(id)))
                />
                <div class="climate-screen__panels">
                    <For each=move || ids.get() key=|id| id.clone() let:id>
                        {
                            let fallback = find_zone(climate.get_untracked().as_ref(), &id);
                            let shown = Memo::new({
                                let id = id.clone();
                                move |_| current.with(|c| c.as_deref() == Some(id.as_str()))
                            });
                            let zone = Memo::new(move |last: Option<&ZoneView>| {
                                find_zone(climate.get().as_ref(), &id)
                                    .or_else(|| last.cloned())
                                    .or_else(|| fallback.clone())
                                    .expect("a keyed zone was in the view when its panel was built")
                            });
                            view! {
                                <div class="climate-screen__panel" hidden=move || !shown.get()>
                                    <RoomPanel
                                        zone=zone.into()
                                        tz=tz.into()
                                        on_changed=reload
                                        shown=shown
                                    />
                                </div>
                            }
                        }
                    </For>
                </div>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_climate::doc::{HouseSettings, ZoneSettings};
    use nidoo_climate::view::LearnStatus;

    fn zone(id: &str, reading: Option<f64>) -> ZoneView {
        ZoneView {
            eco: None,
            zone: id.into(),
            name: id.into(),
            orphan: false,
            settings: ZoneSettings::default(),
            settings_invalid: None,
            entities: vec![],
            reading,
            reading_age_s: None,
            room_sensor: None,
            thermometer: None,
            correction_step_at: None,
            presence: None,
            decision: None,
            overrides: vec![],
            proposal: None,
            learn: LearnStatus::Off,
        }
    }

    fn view(zones: Vec<ZoneView>) -> ClimateView {
        ClimateView {
            enabled: true,
            house: HouseSettings::default(),
            house_invalid: None,
            history: true,
            timezone: "Europe/Paris".into(),
            zones,
        }
    }

    #[test]
    fn ids_keep_order_and_find_reads_the_fresh_zone() {
        let v = view(vec![zone("a", Some(19.0)), zone("b", None)]);
        assert_eq!(zone_ids(Some(&v)), ["a", "b"]);
        assert!(zone_ids(None).is_empty());
        assert_eq!(find_zone(Some(&v), "a").unwrap().reading, Some(19.0));
        assert!(find_zone(Some(&v), "gone").is_none());
    }
}
