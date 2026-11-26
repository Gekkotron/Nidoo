//! A room's settings popup, opened by the card's ⚙: the room thermometer, its correction, room
//! presence, learning and the recent decisions.
//!
//! The decision list lives here, outside the popup's content, so a refetch while the popup is
//! open keeps it; it is fetched when the popup opens and again on each zone change while open.

use chrono_tz::Tz;
use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_climate::doc::{ZoneSettings, ZoneState};
use nidoo_climate::view::{DecisionLog, ZoneView};

use crate::api::climate as api;
use crate::climate::correction_card::CorrectionCard;
use crate::climate::entity_field::{ThermometerField, label_with_room};
use crate::climate::heaters_card::{HeatersCard, zone_labels};
use crate::climate::presence_card::PresenceCard;
use crate::climate::{act, decision_line, learn_status_text, row_line};
use crate::components::now::Now;
use crate::components::popup::Popup;
use crate::components::toast::Toasts;
use crate::house::House;
use crate::i18n::climate::{ClimateText, could_not_save, use_climate_text};
use crate::i18n::use_lang;

/// How many recent decisions the list shows before the older rows.
const RECENT: usize = 10;
/// How many decisions the hub is asked for.
const FETCH: u32 = 50;

#[component]
pub fn RoomSettings(
    zone: Signal<ZoneView>,
    tz: Signal<Tz>,
    on_changed: Callback<()>,
    open: RwSignal<bool>,
    /// Whether the panel is on screen; decisions are fetched only when open && shown.
    shown: Signal<bool>,
) -> impl IntoView {
    let toasts = expect_context::<Toasts>();
    let lang = use_lang();
    let t = use_climate_text();
    let clock = expect_context::<Now>().0;

    let title = Signal::derive(move || {
        format!(
            "{} {}",
            t(ClimateText::RoomSettingsTitle),
            zone.with(|z| z.name.clone())
        )
    });

    // Room thermometer: none on the zone lets the hub pick one in the room.
    let set_sensor = Callback::new(move |sensor: Option<String>| {
        let (id, mut next) = zone.with_untracked(|z| (z.zone.clone(), z.settings.clone()));
        next.sensor = sensor;
        act(toasts, on_changed, async move {
            api::put_zone(&id, &next).await
        });
    });

    // The heaters card: validate, then save the whole settings; a refusal shows the hub's message.
    let house = expect_context::<House>();
    let units = Signal::derive(move || {
        let lang = lang();
        zone.with(|z| {
            z.entities
                .iter()
                .map(|id| (id.clone(), label_with_room(house, lang, id)))
                .collect::<Vec<_>>()
        })
    });
    let set_heaters = Callback::new(move |next: ZoneSettings| {
        if let Err(e) = next.validate() {
            toasts.push(could_not_save(untrack(lang), &e));
            return;
        }
        let id = zone.with_untracked(|z| z.zone.clone());
        spawn_local(async move {
            match api::put_zone(&id, &next).await {
                Ok(()) => on_changed.run(()),
                Err(e) => toasts.push(could_not_save(untrack(lang), &e.error)),
            }
        });
    });

    let learn_line = move || zone.with(|z| learn_status_text(lang(), &z.learn));
    let can_learn = Memo::new(move |_| zone.with(|z| z.settings.state != ZoneState::Off));
    let learn = move |_| {
        let id = zone.with_untracked(|z| z.zone.clone());
        act(toasts, on_changed, async move {
            api::learn(&id).await.map(|_| ())
        });
    };

    let log = RwSignal::new(None::<DecisionLog>);
    Effect::new(move |_| {
        zone.track();
        if open.get() && shown.get() {
            let id = zone.with_untracked(|z| z.zone.clone());
            spawn_local(async move {
                match api::decisions(&id, FETCH).await {
                    Ok(l) => {
                        log.try_set(Some(l));
                    }
                    Err(e) => toasts.push(e.error),
                }
            });
        }
    });
    let list = move || {
        let tz = tz.get();
        let lang = lang();
        let now = clock.get();
        let labels = zone.with(|z| zone_labels(house, lang, z));
        log.get().map(|l| {
            let recent = l
                .recent
                .iter()
                .take(RECENT)
                .map(|d| view! { <li>{decision_line(lang, d, now, tz, &labels)}</li> })
                .collect_view();
            let older = l
                .older
                .iter()
                .map(|r| {
                    view! {
                        <li class="climate-decisions__older">{row_line(lang, r, now, tz, &labels)}</li>
                    }
                })
                .collect_view();
            view! { <ul class="climate-decisions__list">{recent}{older}</ul> }
        })
    };

    view! {
        <Popup open=open title=title>
            <div class="climate-room-settings">
                <section class="climate-room-settings__section">
                    <HeatersCard
                        settings=Signal::derive(move || zone.with(|z| z.settings.clone()))
                        units=units
                        on_change=set_heaters
                    />
                </section>
                <section class="climate-room-settings__section">
                    <ThermometerField
                        value=Signal::derive(move || zone.with(|z| z.settings.sensor.clone()))
                        auto=Signal::derive(move || {
                            zone.with(|z| z.settings.sensor.is_none().then(|| z.room_sensor.clone()).flatten())
                        })
                        on_change=set_sensor
                    />
                </section>
                <section class="climate-room-settings__section">
                    <CorrectionCard zone=zone on_changed=on_changed />
                </section>
                <section class="climate-room-settings__section">
                    <PresenceCard zone=zone tz=tz on_changed=on_changed />
                </section>
                <section class="climate-room-settings__section">
                    <h3 class="climate-room-settings__heading">{move || t(ClimateText::Learning)}</h3>
                    <div class="climate-learn">
                        <span class="climate-learn__text">{learn_line}</span>
                        <Show when=move || can_learn.get()>
                            <button type="button" class="btn btn--compact" on:click=learn>
                                {move || t(ClimateText::LearnNow)}
                            </button>
                        </Show>
                    </div>
                </section>
                <section class="climate-room-settings__section climate-decisions">
                    <h3 class="climate-room-settings__heading">
                        {move || t(ClimateText::RecentDecisions)}
                    </h3>
                    {list}
                </section>
            </div>
        </Popup>
    }
}
