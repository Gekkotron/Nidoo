//! A room's eco as a pill: **Eco…** opens a small popup for the end date and time; a running
//! eco reads "Eco until …" with a ✕ that cancels it. Hidden while the room is off.

use chrono_tz::Tz;
use leptos::prelude::*;
use nidoo_climate::doc::ZoneState;
use nidoo_climate::view::ZoneView;

use crate::api::climate as api;
use crate::climate::{act, eco_end_text, eco_until};
use crate::components::now::{Now, unix_now};
use crate::components::popup::{Popup, PopupSize};
use crate::components::toast::Toasts;
use crate::i18n::climate::{ClimateText, climate_text, eco_until_line, use_climate_text};
use crate::i18n::use_lang;

#[component]
pub fn EcoPill(zone: Signal<ZoneView>, tz: Signal<Tz>, on_changed: Callback<()>) -> impl IntoView {
    let toasts = expect_context::<Toasts>();
    let lang = use_lang();
    let t = use_climate_text();
    let clock = expect_context::<Now>().0;

    // The popup and its draft live here, so a refetch leaves them as they are.
    let eco_open = RwSignal::new(false);
    let eco_date = RwSignal::new(String::new());
    let eco_time = RwSignal::new("00:00".to_owned());
    let allowed = Memo::new(move |_| zone.with(|z| z.settings.state != ZoneState::Off));
    let live_until = Memo::new(move |_| {
        let now = clock.get();
        zone.with(|z| z.eco.as_ref().map(|e| e.until))
            .filter(|until| *until > now)
    });

    let start_eco = move || {
        let until = eco_until(
            &eco_date.get_untracked(),
            &eco_time.get_untracked(),
            tz.get_untracked(),
        )
        .filter(|until| *until > unix_now());
        let Some(until) = until else {
            toasts.push(climate_text(untrack(lang), ClimateText::EcoPickDate));
            return;
        };
        eco_open.set(false);
        let id = zone.with_untracked(|z| z.zone.clone());
        act(
            toasts,
            on_changed,
            async move { api::set_eco(&id, until).await },
        );
    };
    let cancel_eco = move || {
        let id = zone.with_untracked(|z| z.zone.clone());
        act(toasts, on_changed, async move { api::clear_eco(&id).await });
    };
    let open_eco = move || {
        // Tomorrow by default, in the house's time zone.
        let tomorrow = unix_now() + 86_400;
        let date = chrono::DateTime::from_timestamp(i64::try_from(tomorrow).unwrap_or(0), 0)
            .map(|t| {
                t.with_timezone(&tz.get_untracked())
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .unwrap_or_default();
        eco_date.set(date);
        eco_time.set("00:00".to_owned());
        eco_open.set(true);
    };

    let pill = move || {
        allowed.get().then(|| match live_until.get() {
            Some(until) => view! {
                <span class="chip climate-eco-pill climate-eco-pill--on">
                    {move || eco_until_line(lang(), &eco_end_text(lang(), until, tz.get()))}
                    <button
                        type="button"
                        class="climate-eco-pill__cancel"
                        aria-label=move || t(ClimateText::CancelEco)
                        on:click=move |_| cancel_eco()
                    >
                        "✕"
                    </button>
                </span>
            }
            .into_any(),
            None => view! {
                <button type="button" class="chip climate-eco-pill" on:click=move |_| open_eco()>
                    {move || t(ClimateText::EcoPill)}
                </button>
            }
            .into_any(),
        })
    };
    view! {
        {pill}
        <Popup
            open=eco_open
            title=Signal::derive(move || t(ClimateText::EcoUntilButton).to_owned())
            size=PopupSize::Small
        >
            <label class="climate-eco__field">
                <span>{move || t(ClimateText::EcoUntilDate)}</span>
                <input
                    class="field"
                    type="date"
                    prop:value=move || eco_date.get()
                    on:input=move |ev| eco_date.set(event_target_value(&ev))
                />
            </label>
            <label class="climate-eco__field">
                <span>{move || t(ClimateText::EcoUntilTime)}</span>
                <input
                    class="field"
                    type="time"
                    prop:value=move || eco_time.get()
                    on:input=move |ev| eco_time.set(event_target_value(&ev))
                />
            </label>
            <div class="climate-eco-pill__actions">
                <button type="button" class="btn btn--active" on:click=move |_| start_eco()>
                    {move || t(ClimateText::StartEco)}
                </button>
                <button type="button" class="btn" on:click=move |_| eco_open.set(false)>
                    {move || t(ClimateText::Cancel)}
                </button>
            </div>
        </Popup>
    }
}
