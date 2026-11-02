//! The Weather, Climate and Alarm forms of the Services step. Weather and Climate are hub
//! settings sections, so a save restarts the hub; the alarm saves live through its own route.

use leptos::prelude::*;
use serde_json::{Value, json};

use super::{EnabledRow, NotifyRow, load_section, place_line, save_section, with_fields};
use crate::api::alarm::{get_alarm, put_alarm_settings};
use crate::i18n::setup::{SetupText, use_setup_text};
use crate::i18n::use_lang;
use crate::setup::SetupState;
use nidoo_alarm::doc::AlarmSettings;

/// A whole number of minutes, 1 or more.
fn parse_minutes(text: &str) -> Option<u64> {
    text.trim().parse::<u64>().ok().filter(|n| *n >= 1)
}

/// What a form shows until its section has loaded, or when it could not.
fn load_note(
    loaded: RwSignal<Option<Result<Value, String>>>,
    form: impl Fn(Value) -> AnyView + Send + Sync + 'static,
) -> impl IntoView {
    let t = use_setup_text();
    move || {
        loaded.get().map(|result| match result {
            Ok(base) => form(base),
            Err(e) => view! {
                <p class="settings__empty" role="alert">
                    {format!("{} {e}", t(SetupText::LoadFailed))}
                </p>
            }
            .into_any(),
        })
    }
}

#[component]
fn WeatherFields(base: Value, applying_target: String) -> impl IntoView {
    let t = use_setup_text();
    let on = RwSignal::new(base["enabled"].as_bool().unwrap_or(false));
    let notify = RwSignal::new(base["notify_unreachable"].as_bool().unwrap_or(false));
    let minutes = RwSignal::new(
        base["refresh_minutes"]
            .as_u64()
            .map(|n| n.to_string())
            .unwrap_or_default(),
    );
    let saving = RwSignal::new(false);
    let failure = RwSignal::new(None::<String>);
    let invalid = move || parse_minutes(&minutes.get()).is_none();
    let lang = use_lang();
    let state = use_context::<SetupState>();
    let save = move |_| {
        let Some(n) = parse_minutes(&minutes.get_untracked()) else {
            return;
        };
        if saving.get_untracked() {
            return;
        }
        let body = with_fields(
            base.clone(),
            &[
                ("enabled", json!(on.get_untracked())),
                ("refresh_minutes", json!(n)),
                ("notify_unreachable", json!(notify.get_untracked())),
            ],
        );
        saving.set(true);
        failure.set(None);
        let done = save_section("weather", body, &applying_target);
        leptos::task::spawn_local(async move {
            if let Err(message) = done.await {
                failure.try_set(Some(message));
            }
            saving.try_set(false);
        });
    };
    view! {
        <div class="svc__form">
            <p class="settings__hint svc__place">{move || place_line(state, lang())}</p>
            <EnabledRow on=on />
            <NotifyRow on=notify />
            <label class="field" for="weather-minutes">
                <span class="field__caption">{move || t(SetupText::RefreshMinutes)}</span>
                <input
                    id="weather-minutes"
                    class="field__input"
                    type="text"
                    inputmode="numeric"
                    prop:value=move || minutes.get()
                    on:input=move |ev| minutes.set(event_target_value(&ev))
                />
            </label>
            <p class="loc__problem" role="status">
                {move || invalid().then(|| t(SetupText::MinutesInvalid))}
            </p>
            <p class="loc__problem" role="alert">{move || failure.get()}</p>
            <div class="loc__actions">
                <button
                    type="button"
                    class="btn btn--primary svc__save"
                    prop:disabled=move || invalid() || saving.get()
                    on:click=save
                >
                    {move || t(SetupText::Save)}
                </button>
            </div>
        </div>
    }
}

/// The weather switch and refresh interval. A line names the home's place, taken from the
/// setup status; the hub refuses to enable weather without one, and that reason shows as received.
#[component]
pub fn WeatherForm(applying_target: String) -> impl IntoView {
    let loaded = load_section("weather");
    load_note(loaded, move |base| {
        view! { <WeatherFields base=base applying_target=applying_target.clone() /> }.into_any()
    })
}

#[component]
fn ClimateFields(base: Value, applying_target: String) -> impl IntoView {
    let t = use_setup_text();
    let on = RwSignal::new(base["enabled"].as_bool().unwrap_or(false));
    let saving = RwSignal::new(false);
    let failure = RwSignal::new(None::<String>);
    let save = move |_| {
        if saving.get_untracked() {
            return;
        }
        // The prefix and any other key travel back as the hub returned them.
        let body = with_fields(base.clone(), &[("enabled", json!(on.get_untracked()))]);
        saving.set(true);
        failure.set(None);
        let done = save_section("climate", body, &applying_target);
        leptos::task::spawn_local(async move {
            if let Err(message) = done.await {
                failure.try_set(Some(message));
            }
            saving.try_set(false);
        });
    };
    view! {
        <div class="svc__form">
            <EnabledRow on=on />
            <p class="settings__hint">{move || t(SetupText::ClimateBeta)}</p>
            <p class="loc__problem" role="alert">{move || failure.get()}</p>
            <div class="loc__actions">
                <button
                    type="button"
                    class="btn btn--primary svc__save"
                    prop:disabled=move || saving.get()
                    on:click=save
                >
                    {move || t(SetupText::Save)}
                </button>
            </div>
        </div>
    }
}

/// The climate engine's switch, with the beta warning.
#[component]
pub fn ClimateForm(applying_target: String) -> impl IntoView {
    let loaded = load_section("climate");
    load_note(loaded, move |base| {
        view! { <ClimateFields base=base applying_target=applying_target.clone() /> }.into_any()
    })
}

/// The alarm's switch. It goes through the alarm's own route, which applies live, so there is
/// no restart and no overlay. `set_up` follows the switch so the card's pill stays current.
/// Sensors, sirens and delays stay out of the wizard: a link to the alarm page would leave the
/// setup with nothing to bring the user back, so the hint points there for later.
#[component]
pub fn AlarmForm(#[prop(optional)] set_up: Option<RwSignal<Option<bool>>>) -> impl IntoView {
    let t = use_setup_text();
    let lang = use_lang();
    let settings = RwSignal::new(None::<AlarmSettings>);
    let on = RwSignal::new(false);
    // The switch stays disabled until the settings have loaded: flipping it with nothing to
    // save would show a state the hub never got.
    let busy = RwSignal::new(true);
    let failure = RwSignal::new(None::<String>);
    leptos::task::spawn_local(async move {
        match get_alarm().await {
            Ok(view) => {
                busy.try_set(false);
                on.try_set(view.settings.enabled);
                settings.try_set(Some(view.settings));
            }
            Err(e) => {
                let note = format!(
                    "{} {}",
                    crate::i18n::setup::setup_text(lang(), SetupText::LoadFailed),
                    e.error
                );
                failure.try_set(Some(note));
            }
        }
    });
    // The switch row flips `on`; this commits the flip, and undoes it when the hub refuses.
    let mut last = false;
    Effect::new(move |_| {
        let next = on.get();
        if next == last {
            return;
        }
        last = next;
        let Some(mut current) = settings.get_untracked() else {
            return;
        };
        if current.enabled == next {
            return;
        }
        current.enabled = next;
        busy.set(true);
        failure.set(None);
        leptos::task::spawn_local(async move {
            match put_alarm_settings(&current).await {
                Ok(()) => {
                    settings.try_set(Some(current));
                    if let Some(s) = set_up {
                        s.try_set(Some(next));
                    }
                }
                Err(e) => {
                    failure.try_set(Some(e.error));
                    on.try_set(!next);
                }
            }
            busy.try_set(false);
        });
    });
    view! {
        <div class="svc__form">
            <EnabledRow on=on busy=busy />
            <p class="loc__problem" role="alert">{move || failure.get()}</p>
            <p class="settings__hint">{move || t(SetupText::AlarmLater)}</p>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minutes_need_a_whole_number_of_one_or_more() {
        assert_eq!(parse_minutes("15"), Some(15));
        assert_eq!(parse_minutes(" 1 "), Some(1));
        assert_eq!(parse_minutes("0"), None);
        assert_eq!(parse_minutes("1.5"), None);
        assert_eq!(parse_minutes(""), None);
        assert_eq!(parse_minutes("-3"), None);
    }
}
