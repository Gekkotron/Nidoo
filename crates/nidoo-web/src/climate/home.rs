//! The climate room card on Home: a room whose zone is active shows the Climate screen's room
//! card above its device cards. Its ⚙ opens the Climate screen, where the room settings live.

use leptos::prelude::*;
use leptos_router::hooks::use_navigate;
use nidoo_climate::doc::ZoneState;
use nidoo_climate::view::{ClimateView, ZoneView};

use crate::climate::room_card::RoomCard;
use crate::climate::{parse_zone, week_minute};
use crate::components::now::Now;
use crate::house::House;

/// The engine's latest view, fetched once for the whole app; `None` until it answers and while
/// the engine is disabled.
#[derive(Clone, Copy)]
pub struct ClimateFeed(pub RwSignal<Option<ClimateView>>);

/// The zone of `area` when the engine runs it: active and still backed by the area's devices.
pub fn active_zone(view: Option<&ClimateView>, area: &str) -> Option<ZoneView> {
    view?
        .zones
        .iter()
        .find(|z| z.zone == area && z.settings.state == ZoneState::Active && !z.orphan)
        .cloned()
}

/// `area`'s climate room card while its zone is active; nothing otherwise (or without a feed).
#[component]
pub fn HomeClimateCard(area: String) -> impl IntoView {
    let Some(feed) = use_context::<ClimateFeed>() else {
        return ().into_any();
    };
    let house = expect_context::<House>();
    let clock = expect_context::<Now>().0;
    let current = Memo::new(move |_| feed.0.with(|v| active_zone(v.as_ref(), &area)));
    let tz = Memo::new(move |_| {
        feed.0.with(|v| {
            v.as_ref()
                .map_or(chrono_tz::Tz::UTC, |v| parse_zone(&v.timezone))
        })
    });
    let now = Memo::new(move |_| week_minute(clock.get(), tz.get()));
    // Any change refetches the feed, as the hub's own climate event does.
    let on_changed = Callback::new(move |()| house.climate_rev.update(|r| *r += 1));
    let navigate = use_navigate();
    let on_settings = Callback::new(move |()| navigate("/climate", Default::default()));

    view! {
        <Show when=move || current.with(Option::is_some)>
            {move || {
                // Built while the zone is there; it keeps its last state while the card closes.
                let first = current.get_untracked();
                let zone = Memo::new(move |last: Option<&ZoneView>| {
                    current
                        .get()
                        .or_else(|| last.cloned())
                        .or_else(|| first.clone())
                        .expect("the card is built only while the zone is active")
                });
                view! {
                    <div class="room__climate">
                        <RoomCard
                            zone=zone.into()
                            tz=tz.into()
                            now=now.into()
                            on_changed=on_changed
                            on_settings=on_settings
                        />
                    </div>
                }
            }}
        </Show>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::active_zone;
    use nidoo_climate::doc::{HouseSettings, ZoneSettings, ZoneState};
    use nidoo_climate::view::{ClimateView, LearnStatus, ZoneView};

    fn zone(id: &str, state: ZoneState, orphan: bool) -> ZoneView {
        ZoneView {
            eco: None,
            zone: id.into(),
            name: id.into(),
            orphan,
            settings: ZoneSettings {
                state,
                ..ZoneSettings::default()
            },
            settings_invalid: None,
            entities: vec![],
            reading: None,
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
    fn only_an_active_zone_of_the_room_shows() {
        let v = view(vec![
            zone("salon", ZoneState::Active, false),
            zone("chambre", ZoneState::Pending, false),
            zone("bureau", ZoneState::Off, false),
            zone("garage", ZoneState::Active, true),
        ]);
        assert_eq!(
            active_zone(Some(&v), "salon").map(|z| z.zone),
            Some("salon".to_owned())
        );
        assert!(active_zone(Some(&v), "chambre").is_none());
        assert!(active_zone(Some(&v), "bureau").is_none());
        assert!(
            active_zone(Some(&v), "garage").is_none(),
            "an orphan zone has no devices left"
        );
        assert!(active_zone(Some(&v), "cuisine").is_none());
        assert!(active_zone(None, "salon").is_none());
    }
}
