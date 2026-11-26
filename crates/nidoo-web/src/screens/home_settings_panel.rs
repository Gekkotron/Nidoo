//! The Home panel in Settings: the five home-header settings and the off-peak hours behind one
//! Save.

use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use leptos::task::spawn_local;

use nidoo_api::{EntityView, HomeSettings, OffPeakDevice, OffPeakWindow};
use nidoo_core::model::Kind;

use crate::climate::entity_field::ThermometerField;
use crate::components::toast::Toasts;
use crate::entity_picker::EntityPickField;
use crate::entity_picker::needs::keep;
use crate::home_header::mood::{COLD_DEFAULT, HOT_DEFAULT};
use crate::house::House;
use crate::i18n::format::{input_number, parse_decimal};
use crate::i18n::settings::{
    RULE_BAD_TIME, RULE_BETWEEN, RULE_LOWER_THAN, RULE_NOT_A, RULE_OVERLAP, RULE_START_EQUALS_END,
};
use crate::i18n::{HomeText, Lang, home_text};
use crate::if_value::{ValueInput, value_input};
use crate::setup::services::energy::EnergyPanel;

/// A setting the panel edits, for pointing an error at its field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Eco,
    Mode,
    Indoor,
    Cold,
    Hot,
    OffPeak,
}

/// The fields as typed: numbers as text, entity pickers as ids, off-peak windows as
/// `(start, end)` `HH:MM` pairs.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Draft {
    pub eco: String,
    pub mode: Option<String>,
    pub indoor: Option<String>,
    pub cold: String,
    pub hot: String,
    pub off_peak: Vec<(String, String)>,
    /// The off-peak device as picked: its entity id and the values that mean off-peak.
    pub off_peak_device: Option<(String, Vec<String>)>,
}

/// A number as the field shows it, with the language's decimal mark.
fn number_text(lang: Lang, v: Option<f64>) -> String {
    v.map(|v| input_number(lang, v)).unwrap_or_default()
}

impl Draft {
    /// The fields for `s`, numbers written the way `lang` writes them.
    pub fn new(lang: Lang, s: &HomeSettings) -> Self {
        Draft {
            eco: number_text(lang, s.eco_setpoint),
            mode: s.mode_entity.clone(),
            indoor: s.indoor_entity.clone(),
            cold: number_text(lang, s.cold_below),
            hot: number_text(lang, s.hot_above),
            off_peak: s
                .off_peak
                .iter()
                .map(|w| (w.start.clone(), w.end.clone()))
                .collect(),
            off_peak_device: s
                .off_peak_device
                .as_ref()
                .map(|d| (d.entity.clone(), d.values.clone())),
        }
    }
}

/// `,` or `.` as the decimal mark; empty is `None`. A text that is no number names its field.
fn parse_number(text: &str, field: Field) -> Result<Option<f64>, Field> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    parse_decimal(text).map(Some).ok_or(field)
}

/// The draft to save: each field from `draft` when it was edited (differs from `base`, the copy the
/// edits started from), else from `latest`, so a change made elsewhere meanwhile is not undone.
pub fn merge(base: &Draft, draft: &Draft, latest: &Draft) -> Draft {
    Draft {
        eco: if draft.eco != base.eco {
            &draft.eco
        } else {
            &latest.eco
        }
        .clone(),
        mode: if draft.mode != base.mode {
            &draft.mode
        } else {
            &latest.mode
        }
        .clone(),
        indoor: if draft.indoor != base.indoor {
            &draft.indoor
        } else {
            &latest.indoor
        }
        .clone(),
        cold: if draft.cold != base.cold {
            &draft.cold
        } else {
            &latest.cold
        }
        .clone(),
        hot: if draft.hot != base.hot {
            &draft.hot
        } else {
            &latest.hot
        }
        .clone(),
        off_peak: if draft.off_peak != base.off_peak {
            &draft.off_peak
        } else {
            &latest.off_peak
        }
        .clone(),
        off_peak_device: if draft.off_peak_device != base.off_peak_device {
            &draft.off_peak_device
        } else {
            &latest.off_peak_device
        }
        .clone(),
    }
}

/// What sits under a field: our own translated text, or the hub's message when unmapped.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // `Raw` is built by the wasm save path
#[derive(Clone, Debug, PartialEq, Eq)]
enum Message {
    Key(HomeText),
    Raw(String),
}

/// The translated text for a hub validation message on `field`; `None` when it is not one of the
/// known rules.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // only the wasm save path calls it
fn message_key(field: Field, message: &str) -> Option<HomeText> {
    if field == Field::OffPeak {
        return if message.contains(RULE_OVERLAP) {
            Some(HomeText::WindowsOverlap)
        } else if message.contains(RULE_START_EQUALS_END) {
            Some(HomeText::StartEqualsEnd)
        } else if message.contains(RULE_BAD_TIME) {
            Some(HomeText::BadTime)
        } else {
            None
        };
    }
    if message.contains(RULE_BETWEEN) {
        match field {
            Field::Eco => Some(HomeText::EcoRange),
            Field::Cold | Field::Hot => Some(HomeText::ThresholdRange),
            _ => None,
        }
    } else if message.contains(RULE_LOWER_THAN) {
        Some(HomeText::OrderRule)
    } else if message.contains(RULE_NOT_A) {
        matches!(field, Field::Mode | Field::Indoor).then_some(HomeText::EntityRule)
    } else {
        None
    }
}

/// The windows to save for the typed `(start, end)` rows: blank rows dropped, times trimmed.
pub fn off_peak_windows(rows: &[(String, String)]) -> Vec<OffPeakWindow> {
    rows.iter()
        .filter(|(start, end)| !(start.trim().is_empty() && end.trim().is_empty()))
        .map(|(start, end)| OffPeakWindow {
            start: start.trim().to_owned(),
            end: end.trim().to_owned(),
        })
        .collect()
}

/// The device to save: `None` without an entity; values trimmed, blanks and repeats dropped.
pub fn off_peak_device(draft: &Draft) -> Option<OffPeakDevice> {
    let (entity, typed) = draft.off_peak_device.as_ref()?;
    if entity.is_empty() {
        return None;
    }
    let mut values: Vec<String> = Vec::new();
    for v in typed.iter().map(|v| v.trim()).filter(|v| !v.is_empty()) {
        if !values.iter().any(|kept| kept == v) {
            values.push(v.to_owned());
        }
    }
    Some(OffPeakDevice {
        entity: entity.clone(),
        values,
    })
}

/// The values worth offering for `e` as off-peak: what discovery says it reports, then its
/// current reading, as the hub matches them (`on`/`off`, plain numbers, the text).
pub fn device_choices(e: &EntityView) -> Vec<String> {
    use nidoo_core::energy::reading_text;
    use nidoo_core::flow::Scalar;
    let mut out: Vec<String> = Vec::new();
    if let ValueInput::List(choices) = value_input(Lang::En, Some(e)) {
        out.extend(
            choices
                .iter()
                .filter_map(|c| Scalar::from_json(&c.value))
                .map(|v| reading_text(&v)),
        );
    }
    if let Some(now) = Scalar::of_state(&e.state.value).map(|v| reading_text(&v))
        && !matches!(now.as_str(), "" | "unknown" | "unavailable")
        && !out.contains(&now)
    {
        out.push(now);
    }
    out
}

/// The values preselected when `e` is picked: `on` for an on/off entity, none otherwise.
fn device_defaults(e: &EntityView) -> Vec<String> {
    match value_input(Lang::En, Some(e)) {
        ValueInput::List(choices) if choices.iter().any(|c| c.value == serde_json::json!(true)) => {
            vec!["on".to_owned()]
        }
        _ => Vec::new(),
    }
}

/// The settings document the draft stands for, or the first number that does not parse.
pub fn draft_to_settings(draft: &Draft) -> Result<HomeSettings, Field> {
    Ok(HomeSettings {
        eco_setpoint: parse_number(&draft.eco, Field::Eco)?,
        mode_entity: draft.mode.clone(),
        indoor_entity: draft.indoor.clone(),
        cold_below: parse_number(&draft.cold, Field::Cold)?,
        hot_above: parse_number(&draft.hot, Field::Hot)?,
        off_peak: off_peak_windows(&draft.off_peak),
        off_peak_history: Vec::new(),
        off_peak_device: off_peak_device(draft),
        // Not on this panel: Save puts the stored values back (see `keep_stored_fields`).
        energy_view: Default::default(),
        assist_min_c: None,
        assist_max_c: None,
        wake_up: None,
    })
}

/// `edited` with the fields this panel does not edit taken from `stored`: the Energy view and
/// the assistant's range and the Wake-up switch.
pub fn keep_stored_fields(stored: &HomeSettings, edited: HomeSettings) -> HomeSettings {
    HomeSettings {
        energy_view: stored.energy_view,
        assist_min_c: stored.assist_min_c,
        assist_max_c: stored.assist_max_c,
        wake_up: stored.wake_up,
        ..edited
    }
}

/// The field a hub validation message names; `None` when it names none of them.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // only the wasm save path calls it
fn field_of_message(message: &str) -> Option<Field> {
    [
        ("eco_setpoint", Field::Eco),
        ("mode_entity", Field::Mode),
        ("indoor_entity", Field::Indoor),
        ("cold_below", Field::Cold),
        ("hot_above", Field::Hot),
        ("off_peak", Field::OffPeak),
    ]
    .into_iter()
    .find(|(name, _)| message.contains(name))
    .map(|(_, field)| field)
}

/// The off-peak window list: one row per `(start, end)` pair of `draft`, with add and remove.
/// `children` render under the list, where the caller shows its validation message.
#[component]
pub fn OffPeakEditor(draft: RwSignal<Draft>, children: Children) -> impl IntoView {
    let lang = use_context::<RwSignal<Lang>>();
    let t = move |k: HomeText| home_text(lang.map_or(Lang::En, |l| l.get()), k);
    let has_device = move || draft.with(|d| d.off_peak_device.is_some());
    view! {
        <div class="climate-field home-settings__off-peak">
            <span class="climate-field__label">
                {move || {
                    t(if has_device() { HomeText::OffPeakBackup } else { HomeText::OffPeakHours })
                }}
            </span>
            <p class="settings__hint">{move || t(HomeText::OffPeakHint)}</p>
            {move || {
                let rows = draft.with(|d| d.off_peak.len());
                (0..rows)
                    .map(|i| {
                        let get = move |end: bool| {
                            move || {
                                draft.with(|d| {
                                    d.off_peak
                                        .get(i)
                                        .map(|(a, b)| if end { b.clone() } else { a.clone() })
                                        .unwrap_or_default()
                                })
                            }
                        };
                        let set = move |end: bool| {
                            move |ev| {
                                let v = event_target_value(&ev);
                                draft.update(|d| {
                                    if let Some(row) = d.off_peak.get_mut(i) {
                                        if end { row.1 = v } else { row.0 = v }
                                    }
                                });
                            }
                        };
                        view! {
                            <div class="home-settings__window">
                                <input
                                    class="field"
                                    type="time"
                                    aria-label=move || format!("{} {}", t(HomeText::OffPeakHours), i + 1)
                                    prop:value=get(false)
                                    on:change=set(false)
                                />
                                <span aria-hidden="true">"–"</span>
                                <input
                                    class="field"
                                    type="time"
                                    prop:value=get(true)
                                    on:change=set(true)
                                />
                                <button
                                    type="button"
                                    class="btn btn--compact"
                                    on:click=move |_| {
                                        draft.update(|d| {
                                            if i < d.off_peak.len() {
                                                d.off_peak.remove(i);
                                            }
                                        })
                                    }
                                >
                                    {move || t(HomeText::RemoveWindow)}
                                </button>
                            </div>
                        }
                    })
                    .collect_view()
            }}
            <button
                type="button"
                class="btn btn--compact home-settings__add"
                on:click=move |_| draft.update(|d| d.off_peak.push((String::new(), String::new())))
            >
                {move || t(HomeText::AddWindow)}
            </button>
            <OffPeakDeviceRow draft=draft />
            {children()}
        </div>
    }
}

/// "Follow a device": the entity picker, then the values that mean off-peak as chips (what the
/// entity reports and its current reading, plus any typed by hand).
#[component]
fn OffPeakDeviceRow(draft: RwSignal<Draft>) -> impl IntoView {
    let lang = use_context::<RwSignal<Lang>>();
    let t = move |k: HomeText| home_text(lang.map_or(Lang::En, |l| l.get()), k);
    let house = use_context::<House>();
    let entity_of = move |id: &str| house.and_then(|h| h.entities.with(|m| m.get(id).cloned()));
    let entity = Signal::derive(move || {
        draft.with(|d| d.off_peak_device.as_ref().map(|(id, _)| id.clone()))
    });
    let pick = Callback::new(move |id: Option<String>| {
        let defaults = id
            .as_deref()
            .and_then(entity_of)
            .map(|e| device_defaults(&e))
            .unwrap_or_default();
        draft.update(|d| d.off_peak_device = id.map(|id| (id, defaults)));
    });
    let values = move || {
        draft.with(|d| {
            d.off_peak_device
                .as_ref()
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        })
    };
    let toggle = move |value: String| {
        draft.update(|d| {
            if let Some((_, values)) = d.off_peak_device.as_mut() {
                match values.iter().position(|v| *v == value) {
                    Some(i) => {
                        values.remove(i);
                    }
                    None => values.push(value),
                }
            }
        })
    };
    let typed = RwSignal::new(String::new());
    let add = move || {
        let value = typed.get_untracked().trim().to_owned();
        if value.is_empty() {
            return;
        }
        draft.update(|d| {
            if let Some((_, values)) = d.off_peak_device.as_mut()
                && !values.contains(&value)
            {
                values.push(value);
            }
        });
        typed.set(String::new());
    };
    view! {
        <div class="home-settings__device">
            <EntityPickField
                label=Signal::derive(move || t(HomeText::OffPeakDevice).to_owned())
                value=entity
                on_change=pick
                pinned=Some(Signal::derive(move || t(HomeText::NoneChoice).to_owned()))
            />
            <p class="settings__hint">{move || t(HomeText::OffPeakDeviceHint)}</p>
            {move || {
                let id = entity.get()?;
                let mut offered = entity_of(&id).map(|e| device_choices(&e)).unwrap_or_default();
                for v in values() {
                    if !offered.contains(&v) {
                        offered.push(v);
                    }
                }
                let chosen = values();
                Some(view! {
                    <span class="climate-field__label">{move || t(HomeText::OffPeakValues)}</span>
                    <div class="chip-row" role="group">
                        {offered
                            .into_iter()
                            .map(|v| {
                                let on = chosen.contains(&v);
                                let label = v.clone();
                                view! {
                                    <button
                                        type="button"
                                        class="chip"
                                        class:chip--selected=on
                                        aria-pressed=if on { "true" } else { "false" }
                                        on:click=move |_| toggle(v.clone())
                                    >
                                        {label}
                                    </button>
                                }
                            })
                            .collect_view()}
                    </div>
                    <div class="home-settings__window">
                        <input
                            class="field"
                            type="text"
                            aria-label=move || t(HomeText::OffPeakAddValue)
                            prop:value=move || typed.get()
                            on:input=move |ev| typed.set(event_target_value(&ev))
                        />
                        <button type="button" class="btn btn--compact" on:click=move |_| add()>
                            {move || t(HomeText::OffPeakAddValue)}
                        </button>
                    </div>
                    <button
                        type="button"
                        class="btn btn--compact home-settings__device-clear"
                        on:click=move |_| draft.update(|d| d.off_peak_device = None)
                    >
                        {move || t(HomeText::OffPeakClearDevice)}
                    </button>
                })
            }}
        </div>
    }
}

#[component]
pub fn HomeSettingsPanel() -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_context::<RwSignal<Lang>>();
    let t = move |k: HomeText| home_text(lang.map_or(Lang::En, |l| l.get()), k);

    let current = move || lang.map_or(Lang::En, |l| l.get());
    let fresh_draft = house
        .home_settings
        .with_untracked(|s| Draft::new(untrack(current), s));
    let draft = RwSignal::new(fresh_draft.clone());
    // The copy the edits started from, to tell edited fields from untouched ones on Save.
    let base = RwSignal::new(fresh_draft);
    let error = RwSignal::new(None::<(Field, Message)>);
    let saving = RwSignal::new(false);

    // A change from elsewhere (another tab, the MCP, our own save) refills the form, unless
    // there are edits in progress.
    Effect::new(move |_| {
        // Reads the language too, so an untouched form rewrites its numbers on a toggle.
        let l = current();
        let fresh = house.home_settings.with(|s| Draft::new(l, s));
        if draft.get_untracked() == base.get_untracked() {
            draft.set(fresh.clone());
            base.set(fresh);
        }
    });

    let save = move |_| {
        let latest = house
            .home_settings
            .with_untracked(|s| Draft::new(untrack(current), s));
        let merged = merge(&base.get_untracked(), &draft.get_untracked(), &latest);
        let settings = match draft_to_settings(&merged) {
            Ok(s) => house
                .home_settings
                .with_untracked(|stored| keep_stored_fields(stored, s)),
            Err(field) => {
                error.set(Some((field, Message::Key(HomeText::NotANumber))));
                return;
            }
        };
        error.set(None);
        saving.set(true);
        #[cfg(target_arch = "wasm32")]
        {
            let toasts = use_context::<Toasts>();
            spawn_local(async move {
                match crate::api::home::put_home_settings(&settings).await {
                    Ok(stored) => {
                        let fresh = Draft::new(untrack(current), &stored);
                        draft.try_set(fresh.clone());
                        base.try_set(fresh);
                        house.home_settings.set(stored);
                        if let Some(toasts) = toasts {
                            toasts.push(t(HomeText::Saved));
                        }
                    }
                    Err(e) => match field_of_message(&e.error) {
                        Some(field) => {
                            let message = message_key(field, &e.error)
                                .map_or(Message::Raw(e.error), Message::Key);
                            error.try_set(Some((field, message)));
                        }
                        None => {
                            if let Some(toasts) = toasts {
                                toasts.push(format!("{}: {}", t(HomeText::CouldNotSave), e.error));
                            }
                        }
                    },
                }
                saving.try_set(false);
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = (settings, use_context::<Toasts>());
    };

    let error_for = move |field: Field| {
        move || {
            error.with(|e| {
                e.as_ref().filter(|(f, _)| *f == field).map(|(_, message)| {
                    let text = match message {
                        Message::Key(k) => t(*k).to_owned(),
                        Message::Raw(raw) => raw.clone(),
                    };
                    view! { <p class="home-settings__error">{text}</p> }
                })
            })
        }
    };

    let number = move |field: Field,
                       label: HomeText,
                       placeholder: String,
                       get: fn(&Draft) -> String,
                       set: fn(&mut Draft, String)| {
        view! {
            <div class="climate-field">
                <label class="climate-field__label" for=format!("home-{field:?}")>
                    {move || t(label)}
                </label>
                <input
                    id=format!("home-{field:?}")
                    class="field"
                    type="text"
                    inputmode="decimal"
                    placeholder=placeholder
                    prop:value=move || draft.with(get)
                    on:input=move |ev| draft.update(|d| set(d, event_target_value(&ev)))
                />
                {error_for(field)}
            </div>
        }
    };

    let mode_value = Signal::derive(move || draft.with(|d| d.mode.clone()));

    view! {
        <section class="sheet__panel home-settings">
            <h2 class="sheet__heading">{move || t(HomeText::Title)}</h2>
            <p class="settings__hint">{move || t(HomeText::Hint)}</p>
            {number(
                Field::Eco,
                HomeText::EcoSetpoint,
                String::new(),
                |d| d.eco.clone(),
                |d, v| d.eco = v,
            )}
            <div class="climate-field">
                <EntityPickField
                    label=Signal::derive(move || t(HomeText::ModeSelector).to_owned())
                    value=mode_value
                    on_change=Callback::new(move |v| draft.update(|d| d.mode = v))
                    pool=Some(keep(|e| e.kind == Kind::Select))
                    pinned=Some(Signal::derive(move || t(HomeText::NoneChoice).to_owned()))
                />
                {error_for(Field::Mode)}
            </div>
            <div class="home-settings__field">
                <ThermometerField
                    value=Signal::derive(move || draft.with(|d| d.indoor.clone()))
                    auto=Signal::derive(|| None)
                    on_change=Callback::new(move |v| draft.update(|d| d.indoor = v))
                    label=Signal::derive(move || t(HomeText::IndoorSensor).to_owned())
                    auto_text=Signal::derive(move || t(HomeText::AutoMean).to_owned())
                />
                {error_for(Field::Indoor)}
            </div>
            {number(
                Field::Cold,
                HomeText::ColdBelow,
                COLD_DEFAULT.to_string(),
                |d| d.cold.clone(),
                |d, v| d.cold = v,
            )}
            {number(
                Field::Hot,
                HomeText::HotAbove,
                HOT_DEFAULT.to_string(),
                |d| d.hot.clone(),
                |d, v| d.hot = v,
            )}
            <OffPeakEditor draft=draft>{error_for(Field::OffPeak)}</OffPeakEditor>
            <button
                type="button"
                class="chip home-settings__save"
                prop:disabled=move || saving.get()
                on:click=save
            >
                {move || t(HomeText::Save)}
            </button>
        </section>
        <EnergyPanel />
    }
}

#[cfg(test)]
mod tests;
