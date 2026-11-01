//! The Alerts page: channel chips on top, the alerts grouped by day below.

use leptos::prelude::*;
use nidoo_api::AlertChannel;

use crate::alerts::{DayLabel, filtered, group_by_day};
use crate::components::alert_row::AlertRow;
use crate::components::now::Now;
use crate::house::House;
use crate::i18n::format::local_date_time;
use crate::i18n::security::{SecurityText, channel_name, day_heading, security_text};
use crate::i18n::{Key, Lang, use_lang, use_text};
use crate::icons::channel_icon;
use crate::nav::BackLink;
use chrono::Datelike;

/// The heading of a day group: "Today", "Yesterday" or the date, in `lang`.
pub fn day_title(lang: Lang, label: &DayLabel, now_secs: u64) -> String {
    match label {
        DayLabel::Today => security_text(lang, SecurityText::Today).into(),
        DayLabel::Yesterday => security_text(lang, SecurityText::Yesterday).into(),
        DayLabel::On { year, month, day } => {
            day_heading(lang, *year, *month, *day, local_date_time(now_secs).year())
        }
    }
}

/// The browser's `Date.getTimezoneOffset()` in minutes; 0 off the browser.
#[cfg(target_arch = "wasm32")]
fn tz_offset_min() -> i32 {
    js_sys::Date::new_0().get_timezone_offset() as i32
}

#[cfg(not(target_arch = "wasm32"))]
fn tz_offset_min() -> i32 {
    0
}

#[component]
pub fn AlertsScreen() -> impl IntoView {
    let house = expect_context::<House>();
    let now = expect_context::<Now>();
    let lang = use_lang();
    let t = use_text();
    let selected = RwSignal::new(None::<AlertChannel>);
    let shown = Memo::new(move |_| house.alerts.with(|alerts| filtered(alerts, selected.get())));
    let groups =
        Memo::new(move |_| shown.with(|alerts| group_by_day(alerts, now.0.get(), tz_offset_min())));

    let all_chip = view! {
        <button
            type="button"
            class="chip"
            class:chip--selected=move || selected.get().is_none()
            on:click=move |_| selected.set(None)
        >
            {move || security_text(lang(), SecurityText::All)}
        </button>
    };
    let channel_chips = AlertChannel::ALL
        .into_iter()
        .map(|channel| {
            view! {
                <button
                    type="button"
                    class="chip"
                    class:chip--selected=move || selected.get() == Some(channel)
                    on:click=move |_| selected.set(Some(channel))
                >
                    <span class="chip__glyph" inner_html=channel_icon(channel)></span>
                    {move || channel_name(lang(), channel)}
                </button>
            }
        })
        .collect_view();

    view! {
        <div class="alerts">
            <header class="sheet__header">
                <BackLink />
                <div class="sheet__titles">
                    <h1 class="sheet__title">{move || t(Key::Alerts)}</h1>
                </div>
            </header>
            <nav class="chips" aria-label=move || security_text(lang(), SecurityText::Channels)>{all_chip} {channel_chips}</nav>
            <Show
                when=move || shown.with(|a| !a.is_empty())
                fallback=move || {
                    view! {
                        <p class="alerts__empty">
                            {move || {
                                security_text(
                                    lang(),
                                    if selected.get().is_none() {
                                        SecurityText::NoAlerts
                                    } else {
                                        SecurityText::NoAlertsInChannel
                                    },
                                )
                            }}
                        </p>
                    }
                }
            >
                <For
                    each=move || groups.get()
                    key=|(label, list)| {
                        (
                            label.clone(),
                            list.iter()
                                .map(|a| (a.id.clone(), a.title.clone(), a.body.clone(), a.image_url.clone(), a.resolved, a.sent_at))
                                .collect::<Vec<_>>(),
                        )
                    }
                    children=move |(label, list)| {
                        view! {
                            <h2 class="alerts__day">{move || day_title(lang(), &label, now.0.get() / 1000)}</h2>
                            <div class="alerts__list">
                                {list
                                    .into_iter()
                                    .map(|alert| view! { <AlertRow alert=alert /> })
                                    .collect_view()}
                            </div>
                        }
                    }
                />
            </Show>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::day_title;
    use crate::alerts::DayLabel;
    use crate::i18n::Lang;

    /// 2026-10-19, so 2026 is the current year.
    const NOW: u64 = 1_792_402_860;

    #[test]
    fn day_title_words_and_dates_in_both_languages() {
        assert_eq!(day_title(Lang::En, &DayLabel::Today, NOW), "Today");
        assert_eq!(day_title(Lang::Fr, &DayLabel::Today, NOW), "Aujourd'hui");
        assert_eq!(day_title(Lang::En, &DayLabel::Yesterday, NOW), "Yesterday");
        assert_eq!(day_title(Lang::Fr, &DayLabel::Yesterday, NOW), "Hier");
        let day = DayLabel::On {
            year: 2026,
            month: 10,
            day: 5,
        };
        assert_eq!(day_title(Lang::En, &day, NOW), "Oct 5");
        assert_eq!(day_title(Lang::Fr, &day, NOW), "5 oct.");
    }
}
