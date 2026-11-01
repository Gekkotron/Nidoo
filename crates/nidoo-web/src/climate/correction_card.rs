//! The thermometer correction card of a room: a switch, a one-line summary and, folded under
//! Advanced settings, the numbers that tune it.
//!
//! Every change sends the whole zone settings, as the room's other controls do.

use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_climate::doc::{FeedbackSettings, ZoneSettings};
use nidoo_climate::view::ZoneView;

use crate::api::climate as api;
use crate::climate::correction_summary;
use crate::climate::settings_sheet::{NumField, Unit};
use crate::components::toast::Toasts;
use crate::i18n::climate::{ClimateText, could_not_save, use_climate_text};
use crate::i18n::use_lang;

/// How many fields the Advanced settings fold holds.
const FIELDS: usize = 7;

/// The quiet limit in hours, for the field.
fn quiet_hours(min: u32) -> f64 {
    f64::from(min) / 60.0
}

/// The quiet limit in whole minutes, from the field's hours.
fn quiet_minutes(h: f64) -> u32 {
    whole(h * 60.0)
}

#[component]
pub fn CorrectionCard(zone: Signal<ZoneView>, on_changed: Callback<()>) -> impl IntoView {
    let toasts = expect_context::<Toasts>();
    let lang = use_lang();
    let t = use_climate_text();
    let id = zone.with_untracked(|z| z.zone.clone());

    // Memos, so a refetch that changes neither keeps the fields and an opened fold as they are.
    let enabled = Memo::new(move |_| zone.with(|z| z.settings.feedback.enabled));
    let has_sensor =
        Memo::new(move |_| zone.with(|z| z.room_sensor.is_some() || z.settings.sensor.is_some()));
    let summary = move || zone.with(|z| correction_summary(lang(), &z.settings.feedback));

    // Applies `edit` to the zone's feedback settings and saves the whole settings.
    let save = {
        let id = id.clone();
        move |edit: Box<dyn FnOnce(&mut FeedbackSettings)>| {
            let mut next: ZoneSettings = zone.with_untracked(|z| z.settings.clone());
            edit(&mut next.feedback);
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
            let on = !enabled.get_untracked();
            save(Box::new(move |f| f.enabled = on));
        }
    };

    let num = {
        let save = save.clone();
        move |label: ClimateText,
              unit: Unit,
              get: fn(&FeedbackSettings) -> f64,
              set: fn(&mut FeedbackSettings, f64)| {
            let save = save.clone();
            view! {
                <NumField
                    label=label
                    unit=unit
                    value=Signal::derive(move || zone.with(|z| get(&z.settings.feedback)))
                    on_change=Callback::new(move |v| save(Box::new(move |f| set(f, v))))
                />
            }
        }
    };

    let fields = move || {
        enabled.get().then(|| {
            let fields = view! {
                {num(ClimateText::CorrectionStep, Unit::Celsius, |f| f.step, |f, v| f.step = v)}
                {num(ClimateText::CorrectionBand, Unit::Celsius, |f| f.band, |f, v| f.band = v)}
                {num(
                    ClimateText::CorrectionInterval,
                    Unit::Word(ClimateText::UnitMin),
                    |f| f64::from(f.interval_min),
                    |f, v| f.interval_min = whole(v),
                )}
                {num(
                    ClimateText::CorrectionWarmup,
                    Unit::Word(ClimateText::UnitMin),
                    |f| f64::from(f.warmup_min),
                    |f, v| f.warmup_min = whole(v),
                )}
                {num(
                    ClimateText::CorrectionMaxBelow,
                    Unit::Celsius,
                    |f| f.max_below,
                    |f, v| f.max_below = v,
                )}
                {num(
                    ClimateText::CorrectionMaxAbove,
                    Unit::Celsius,
                    |f| f.max_above,
                    |f, v| f.max_above = v,
                )}
                {num(
                    ClimateText::CorrectionQuiet,
                    Unit::Word(ClimateText::UnitHours),
                    |f| quiet_hours(f.quiet_limit_min),
                    |f, v| f.quiet_limit_min = quiet_minutes(v),
                )}
            };
            view! {
                <p class="climate-field__hint">{summary}</p>
                <details class="climate-correction__more">
                    <summary>{move || format!("{} ({FIELDS})", t(ClimateText::AdvancedSettings))}</summary>
                    <div class="climate-correction__fields">{fields}</div>
                </details>
            }
        })
    };

    view! {
        <div class="card climate-correction-card">
            <label class="override__flag">
                <span class="override__label">{move || t(ClimateText::CorrectionTitle)}</span>
                <button
                    type="button"
                    role="switch"
                    class="switch"
                    class:switch--on=move || enabled.get()
                    aria-checked=move || enabled.get().to_string()
                    on:click=toggle
                >
                    <span class="switch__knob"></span>
                </button>
            </label>
            {move || {
                if has_sensor.get() {
                    fields().into_any()
                } else {
                    view! { <p class="climate-field__hint">{move || t(ClimateText::CorrectionNoSensor)}</p> }
                        .into_any()
                }
            }}
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
    fn quiet_hours_and_minutes_convert() {
        assert_eq!(quiet_minutes(6.0), 360);
        assert!((quiet_hours(90) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn whole_rounds_minutes() {
        assert_eq!(whole(15.4), 15);
        assert_eq!(whole(-3.0), 0);
    }
}
