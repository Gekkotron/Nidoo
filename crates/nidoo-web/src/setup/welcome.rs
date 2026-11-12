//! The Welcome step: the language and the home's place. [`LocationForm`] is also the body of
//! Settings › Location, so it needs no `House`, socket or wizard.

use leptos::prelude::*;
use nidoo_api::{GeoPlace, Location, SetupStatus, SetupStep};

use super::wizard::WizardNav;
use super::{Applying, SetupState, setup_href};
use crate::api::setup::{SaveError, geocode, put_location, restore_pending};
use crate::i18n::setup::{SetupText, use_setup_text};
use crate::i18n::{Lang, use_lang};

/// Most candidates the form lists.
const MAX_PLACES: usize = 5;

/// A place as one line: name, region and country, whichever exist.
pub fn place_label(p: &GeoPlace) -> String {
    [
        Some(p.name.as_str()),
        p.admin.as_deref(),
        p.country.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(", ")
}

/// The coordinates as `44.84° N, 0.58° W · Europe/Paris`.
pub fn coords_label(lat: f64, lon: f64, tz: &str) -> String {
    let ns = if lat >= 0.0 { 'N' } else { 'S' };
    let ew = if lon >= 0.0 { 'E' } else { 'W' };
    format!("{:.2}° {ns}, {:.2}° {ew} · {tz}", lat.abs(), lon.abs())
}

/// What stops the values from being saved, first problem first. The hub checks again; this is
/// so Next says why it is off.
pub fn location_problem(
    city: &str,
    country: &str,
    lat: f64,
    lon: f64,
    tz: &str,
) -> Option<SetupText> {
    let country = country.trim();
    if city.trim().is_empty() {
        Some(SetupText::CityRequired)
    } else if !country.is_empty()
        && !(country.len() == 2 && country.bytes().all(|b| b.is_ascii_alphabetic()))
    {
        Some(SetupText::CountryTwoLetters)
    } else if !(-90.0..=90.0).contains(&lat) {
        Some(SetupText::LatitudeRange)
    } else if !(-180.0..=180.0).contains(&lon) {
        Some(SetupText::LongitudeRange)
    } else if tz.trim().parse::<chrono_tz::Tz>().is_err() {
        Some(SetupText::TimeZoneUnknown)
    } else {
        None
    }
}

/// A coordinate field's text as a number; text that is not one is NaN, which no range accepts.
fn parse_coord(text: &str) -> f64 {
    text.trim().parse().unwrap_or(f64::NAN)
}

/// City, country, a geocoder search and the coordinates to correct it by hand. A save that
/// restarts the hub starts the "Applying…" overlay towards `applying_target`; any other save
/// calls `on_saved`. `wizard` labels the button Next instead of Save.
#[component]
pub fn LocationForm(
    initial: Option<Location>,
    applying_target: String,
    #[prop(into)] on_saved: Callback<SetupStatus>,
    #[prop(optional)] wizard: bool,
) -> impl IntoView {
    let t = use_setup_text();
    let lang = use_lang();
    let applying = expect_context::<Applying>();
    let initial = initial.unwrap_or(Location {
        city: String::new(),
        country: None,
        latitude: f64::NAN,
        longitude: f64::NAN,
        timezone: String::new(),
    });
    let number = |v: f64| {
        if v.is_nan() {
            String::new()
        } else {
            v.to_string()
        }
    };
    let city = RwSignal::new(initial.city);
    let country = RwSignal::new(initial.country.unwrap_or_default());
    let lat = RwSignal::new(number(initial.latitude));
    let lon = RwSignal::new(number(initial.longitude));
    let tz = RwSignal::new(initial.timezone);
    let places = RwSignal::new(Vec::<GeoPlace>::new());
    let none_found = RwSignal::new(false);
    let open = RwSignal::new(false);
    let saving = RwSignal::new(false);
    let failure = RwSignal::new(None::<String>);

    let problem = Memo::new(move |_| {
        location_problem(
            &city.get(),
            &country.get(),
            parse_coord(&lat.get()),
            parse_coord(&lon.get()),
            &tz.get(),
        )
    });

    let search = move |_| {
        let name = city.get_untracked().trim().to_owned();
        let code = country.get_untracked().trim().to_owned();
        if name.is_empty() {
            return;
        }
        none_found.set(false);
        leptos::task::spawn_local(async move {
            let found = geocode(&name, (!code.is_empty()).then_some(code.as_str()))
                .await
                .unwrap_or_default();
            if found.is_empty() {
                open.try_set(true);
                none_found.try_set(true);
            }
            places.try_set(found.into_iter().take(MAX_PLACES).collect());
        });
    };

    let pick = move |p: GeoPlace| {
        city.set(p.name);
        if let Some(code) = p.country {
            country.set(code);
        }
        lat.set(p.latitude.to_string());
        lon.set(p.longitude.to_string());
        // A result without a time zone keeps the current one.
        match p.timezone {
            Some(zone) => tz.set(zone),
            // The zone field lives in the closed disclosure; show it so the user can fill it.
            None => open.set(true),
        }
        places.set(Vec::new());
        none_found.set(false);
    };

    let save = move |_| {
        if problem.get_untracked().is_some() || saving.get_untracked() {
            return;
        }
        let code = country.get_untracked().trim().to_ascii_uppercase();
        let location = Location {
            city: city.get_untracked().trim().to_owned(),
            country: (!code.is_empty()).then_some(code),
            latitude: parse_coord(&lat.get_untracked()),
            longitude: parse_coord(&lon.get_untracked()),
            timezone: tz.get_untracked().trim().to_owned(),
        };
        saving.set(true);
        failure.set(None);
        let target = applying_target.clone();
        leptos::task::spawn_local(async move {
            match put_location(&location).await {
                // The hub restarts: the button stays off until the page reloads.
                Ok(status) if status.restarting => {
                    applying.start(&target);
                    return;
                }
                Err(SaveError::Restarting) => {
                    applying.start(&target);
                    return;
                }
                Ok(status) => on_saved.run(status),
                Err(e) => {
                    failure.try_set(Some(e.message(lang())));
                }
            }
            saving.try_set(false);
        });
    };

    view! {
        <div class="loc">
            <div class="loc__grid">
                <label class="field" for="loc-city">
                    <span class="field__caption">{move || t(SetupText::City)}</span>
                    <input
                        id="loc-city"
                        class="field__input"
                        type="text"
                        autocomplete="off"
                        prop:value=move || city.get()
                        on:input=move |ev| city.set(event_target_value(&ev))
                    />
                </label>
                <label class="field" for="loc-country">
                    <span class="field__caption">{move || t(SetupText::Country)}</span>
                    <input
                        id="loc-country"
                        class="field__input"
                        type="text"
                        maxlength="2"
                        placeholder="FR"
                        autocomplete="off"
                        prop:value=move || country.get()
                        on:input=move |ev| country.set(event_target_value(&ev))
                    />
                </label>
            </div>
            <button
                type="button"
                class="btn loc__search"
                prop:disabled=move || city.with(|c| c.trim().is_empty())
                on:click=search
            >
                {move || t(SetupText::Search)}
            </button>
            <Show when=move || none_found.get()>
                <p class="settings__hint">{move || t(SetupText::NoPlaceFound)}</p>
            </Show>
            <div class="loc__results">
                {move || {
                    places
                        .get()
                        .into_iter()
                        .map(|p| {
                            let label = place_label(&p);
                            view! {
                                <button
                                    type="button"
                                    class="chip loc__result"
                                    on:click=move |_| pick(p.clone())
                                >
                                    {label}
                                </button>
                            }
                        })
                        .collect_view()
                }}
            </div>
            <details
                class="loc__details"
                prop:open=move || open.get()
                on:toggle=move |ev| open.set(event_target::<web_sys::HtmlDetailsElement>(&ev).open())
            >
                <summary class="sheet__summary">{move || t(SetupText::CorrectCoords)}</summary>
                <div class="loc__grid">
                    <label class="field" for="loc-lat">
                        <span class="field__caption">{move || t(SetupText::Latitude)}</span>
                        <input
                            id="loc-lat"
                            class="field__input"
                            type="text"
                            inputmode="decimal"
                            prop:value=move || lat.get()
                            on:input=move |ev| lat.set(event_target_value(&ev))
                        />
                    </label>
                    <label class="field" for="loc-lon">
                        <span class="field__caption">{move || t(SetupText::Longitude)}</span>
                        <input
                            id="loc-lon"
                            class="field__input"
                            type="text"
                            inputmode="decimal"
                            prop:value=move || lon.get()
                            on:input=move |ev| lon.set(event_target_value(&ev))
                        />
                    </label>
                    <label class="field loc__wide" for="loc-tz">
                        <span class="field__caption">{move || t(SetupText::TimeZone)}</span>
                        <input
                            id="loc-tz"
                            class="field__input"
                            type="text"
                            autocomplete="off"
                            prop:value=move || tz.get()
                            on:input=move |ev| tz.set(event_target_value(&ev))
                        />
                    </label>
                </div>
            </details>
            <p class="settings__hint">{move || t(SetupText::LocationUse)}</p>
            <Show when=move || problem.get().is_none()>
                <p class="loc__place">
                    {move || {
                        coords_label(
                            parse_coord(&lat.get()),
                            parse_coord(&lon.get()),
                            tz.get().trim(),
                        )
                    }}
                </p>
            </Show>
            <p class="loc__problem" role="status">
                {move || problem.get().map(t)}
            </p>
            <p class="loc__problem" role="alert">
                {move || failure.get()}
            </p>
            <div class="loc__actions">
                <button
                    type="button"
                    class="btn btn--primary loc__save"
                    prop:disabled=move || problem.get().is_some() || saving.get()
                    on:click=save
                >
                    {move || t(if wizard { SetupText::Next } else { SetupText::Save })}
                </button>
            </div>
        </div>
    }
}

/// The Welcome step: a greeting, the language and the home's place, and a way into the restore
/// path. Needs [`WizardNav`] and [`Applying`] in context.
#[component]
pub fn WelcomeStep() -> impl IntoView {
    let t = use_setup_text();
    let nav = expect_context::<WizardNav>();
    let applying = expect_context::<Applying>();
    let lang = use_context::<RwSignal<Lang>>();
    let state = use_context::<SetupState>();
    let choosing = RwSignal::new(false);
    let restore_failure = RwSignal::new(None::<String>);
    let restore = move |_| {
        if choosing.get_untracked() {
            return;
        }
        choosing.set(true);
        restore_failure.set(None);
        let message_lang = lang.map_or(Lang::En, |l| l.get_untracked());
        leptos::task::spawn_local(async move {
            match restore_pending(true).await {
                Ok(status) if status.restarting => {
                    applying.start(&setup_href(SetupStep::Mqtt));
                    return;
                }
                Err(SaveError::Restarting) => {
                    applying.start(&setup_href(SetupStep::Mqtt));
                    return;
                }
                Ok(status) => {
                    if let Some(st) = state {
                        st.0.try_set(Some(status));
                    }
                    nav.step.try_set(SetupStep::Mqtt);
                }
                Err(e) => {
                    restore_failure.try_set(Some(e.message(message_lang)));
                }
            }
            choosing.try_set(false);
        });
    };
    let chip = move |value: Lang, label: SetupText| {
        view! {
            <button
                type="button"
                class="chip"
                class:chip--selected=move || lang.is_some_and(|l| l.get() == value)
                aria-pressed=move || {
                    lang.is_some_and(|l| l.get() == value).to_string()
                }
                on:click=move |_| {
                    if let Some(l) = lang {
                        l.set(value);
                    }
                }
            >
                {move || t(label)}
            </button>
        }
    };
    view! {
        <div class="setup__body">
            <h1 class="setup__title">{move || t(SetupText::WelcomeTitle)}</h1>
            <p class="setup__lead">{move || t(SetupText::WelcomeLead)}</p>
            <h2 class="sheet__heading">{move || t(SetupText::Language)}</h2>
            <div class="chip-row">
                {chip(Lang::En, SetupText::LangEnglish)} {chip(Lang::Fr, SetupText::LangFrench)}
            </div>
            <h2 class="sheet__heading">{move || t(SetupText::LocationTitle)}</h2>
            <LocationForm
                initial=state.and_then(|s| s.0.get_untracked()).and_then(|s| s.location)
                applying_target=setup_href(SetupStep::Mqtt)
                on_saved=move |s: SetupStatus| {
                    if let Some(st) = state {
                        st.0.try_set(Some(s));
                    }
                    nav.step.set(SetupStep::Mqtt);
                }
                wizard=true
            />
            <button
                type="button"
                class="btn welcome__restore"
                prop:disabled=move || choosing.get()
                on:click=restore
            >
                {move || t(SetupText::RestoreFromBackup)}
            </button>
            <p class="loc__problem" role="alert">
                {move || restore_failure.get()}
            </p>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::setup::SetupText;
    use nidoo_api::GeoPlace;

    fn place(admin: Option<&str>) -> GeoPlace {
        GeoPlace {
            name: "Bordeaux".into(),
            admin: admin.map(Into::into),
            country: Some("FR".into()),
            latitude: 44.84,
            longitude: -0.58,
            timezone: Some("Europe/Paris".into()),
        }
    }

    #[test]
    fn place_label_skips_missing_parts() {
        assert_eq!(
            place_label(&place(Some("Nouvelle-Aquitaine"))),
            "Bordeaux, Nouvelle-Aquitaine, FR"
        );
        assert_eq!(place_label(&place(None)), "Bordeaux, FR");
    }

    #[test]
    fn coords_label_signs_pick_the_hemisphere() {
        assert_eq!(
            coords_label(44.8378, -0.5792, "Europe/Paris"),
            "44.84° N, 0.58° W · Europe/Paris"
        );
        assert_eq!(
            coords_label(-33.87, 151.21, "Australia/Sydney"),
            "33.87° S, 151.21° E · Australia/Sydney"
        );
    }

    #[test]
    fn location_problem_names_the_first_bad_field() {
        let p = location_problem;
        assert_eq!(
            p(" ", "FR", 1.0, 1.0, "Europe/Paris"),
            Some(SetupText::CityRequired)
        );
        assert_eq!(
            p("B", "FRA", 1.0, 1.0, "Europe/Paris"),
            Some(SetupText::CountryTwoLetters)
        );
        assert_eq!(
            p("B", "F1", 1.0, 1.0, "Europe/Paris"),
            Some(SetupText::CountryTwoLetters)
        );
        assert_eq!(
            p("B", "FR", 91.0, 1.0, "Europe/Paris"),
            Some(SetupText::LatitudeRange)
        );
        assert_eq!(
            p("B", "FR", 1.0, -181.0, "Europe/Paris"),
            Some(SetupText::LongitudeRange)
        );
        assert_eq!(
            p("B", "FR", 1.0, 1.0, "Europe/Nowhere"),
            Some(SetupText::TimeZoneUnknown)
        );
        assert_eq!(p("Bordeaux", "FR", 44.84, -0.58, "Europe/Paris"), None);
        assert_eq!(p("Bordeaux", "", 44.84, -0.58, "Europe/Paris"), None);
    }
}
