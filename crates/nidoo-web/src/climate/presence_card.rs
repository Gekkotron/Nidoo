//! The presence card of a room: a switch, the room's presence sensors and the numbers that tune
//! the setback.
//!
//! Every change sends the whole zone settings, as the room's other controls do.

use std::sync::Arc;

use chrono_tz::Tz;
use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_climate::doc::{PresenceSettings, ZoneSettings};
use nidoo_climate::view::{PresenceView, ZoneView};

use crate::api::climate as api;
use crate::climate::clock_text;
use crate::climate::settings_sheet::{NumField, Unit};
use crate::components::toast::Toasts;
use crate::entity_picker::EntityPickListField;
use crate::entity_picker::Keep;
use crate::entity_picker::needs::is_room_presence;
use crate::i18n::Lang;
use crate::i18n::climate::{
    ClimateText, climate_text, could_not_save, empty_since, use_climate_text,
};
use crate::i18n::use_lang;

/// "Occupied", or "Empty since 14:20" in the house's time zone.
fn status_text(lang: Lang, presence: &PresenceView, tz: Tz) -> String {
    match presence.empty_since {
        Some(at) => empty_since(lang, &clock_text(at, tz)),
        None => climate_text(lang, ClimateText::RoomPresenceOccupied).to_owned(),
    }
}

#[component]
pub fn PresenceCard(
    zone: Signal<ZoneView>,
    tz: Signal<Tz>,
    on_changed: Callback<()>,
) -> impl IntoView {
    let toasts = expect_context::<Toasts>();
    let lang = use_lang();
    let t = use_climate_text();
    let id = zone.with_untracked(|z| z.zone.clone());

    let enabled = move || zone.with(|z| z.settings.presence.enabled);
    let has_sensor = move || zone.with(|z| !z.settings.presence.sensors.is_empty());

    // Applies `edit` to the zone's presence settings and saves the whole settings.
    let save = {
        let id = id.clone();
        move |edit: Box<dyn FnOnce(&mut PresenceSettings)>| {
            let mut next: ZoneSettings = zone.with_untracked(|z| z.settings.clone());
            edit(&mut next.presence);
            if let Err(e) = next.validate() {
                toasts.push(could_not_save(untrack(lang), &e));
                return;
            }
            let id = id.clone();
            spawn_local(async move {
                match api::put_zone(&id, &next).await {
                    Ok(()) => on_changed.run(()),
                    Err(e) => toasts.push(could_not_save(untrack(lang), &e.error)),
                }
            });
        }
    };

    let toggle = {
        let save = save.clone();
        move |_| {
            let on = !enabled();
            save(Box::new(move |p| p.enabled = on));
        }
    };

    let num = {
        let save = save.clone();
        move |label: ClimateText,
              unit: Unit,
              get: fn(&PresenceSettings) -> f64,
              set: fn(&mut PresenceSettings, f64)| {
            let save = save.clone();
            view! {
                <NumField
                    label=label
                    unit=unit
                    value=Signal::derive(move || zone.with(|z| get(&z.settings.presence)))
                    on_change=Callback::new(move |v| save(Box::new(move |p| set(p, v))))
                />
            }
        }
    };

    // The room's own presence sensors.
    let pool: Keep = {
        let area = id.clone();
        Arc::new(move |e| is_room_presence(e) && e.area == area)
    };

    let fields = move || {
        let save = save.clone();
        let pool = pool.clone();
        enabled().then(|| {
            view! {
                {move || {
                    zone.with(|z| z.presence.clone())
                        .map(|p| {
                            view! {
                                <p class="climate-field__hint">
                                    {move || status_text(lang(), &p, tz.get())}
                                </p>
                            }
                        })
                }}
                <EntityPickListField
                    label=Signal::derive(move || t(ClimateText::RoomPresenceSensors).to_owned())
                    value=Signal::derive(move || zone.with(|z| z.settings.presence.sensors.clone()))
                    on_change=Callback::new(move |v: Vec<String>| {
                        save(Box::new(move |p| p.sensors = v))
                    })
                    pool=Some(pool)
                    class=Some("climate-field".to_owned())
                />
                {move || {
                    (!has_sensor())
                        .then(|| {
                            view! {
                                <p class="climate-field__hint">
                                    {move || t(ClimateText::RoomPresenceNoSensor)}
                                </p>
                            }
                        })
                }}
                {num(
                    ClimateText::RoomPresenceEmptyAfter,
                    Unit::Word(ClimateText::UnitMin),
                    |p| f64::from(p.empty_after_min),
                    |p, v| p.empty_after_min = whole(v),
                )}
                {num(
                    ClimateText::RoomPresenceBackAfter,
                    Unit::Word(ClimateText::UnitMin),
                    |p| f64::from(p.back_after_min),
                    |p, v| p.back_after_min = whole(v),
                )}
                {num(ClimateText::RoomPresenceSetback, Unit::Celsius, |p| p.setback, |p, v| p.setback = v)}
            }
        })
    };

    view! {
        <div class="card climate-presence-card">
            <label class="override__flag">
                <span class="override__label">{move || t(ClimateText::RoomPresenceTitle)}</span>
                <button
                    type="button"
                    role="switch"
                    class="switch"
                    class:switch--on=enabled
                    aria-checked=move || enabled().to_string()
                    on:click=toggle
                >
                    <span class="switch__knob"></span>
                </button>
            </label>
            {fields}
        </div>
    }
}

/// A whole number of minutes from a field value (negatives become 0).
fn whole(v: f64) -> u32 {
    // A saturating cast; validation refuses what is out of range.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let minutes = v.round().max(0.0) as u32;
    minutes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_says_occupied_or_since_when() {
        let tz: Tz = "Europe/Paris".parse().unwrap();
        let occupied = PresenceView { empty_since: None };
        assert_eq!(status_text(Lang::En, &occupied, tz), "Occupied");
        assert_eq!(status_text(Lang::Fr, &occupied, tz), "Occupée");
        // 2026-10-21 12:20 UTC is 14:20 in Paris.
        let empty = PresenceView {
            empty_since: Some(1_792_585_200),
        };
        assert_eq!(status_text(Lang::En, &empty, tz), "Empty since 14:20");
        assert_eq!(status_text(Lang::Fr, &empty, tz), "Vide depuis 14:20");
    }
}
