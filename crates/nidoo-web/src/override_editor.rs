//! The settings form of the device sheet: only the controls that apply to the entity (see
//! [`crate::settings_rules`]), saved as a partial patch of the hub's `UiOverride`.

use std::sync::Arc;

use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_api::{EntityView, OverridePatch};
use nidoo_core::energy::{ConsumerIcon, EnergyMeters};
use nidoo_core::model::{ENERGY_ROLES, Kind, LIGHT_ON_PRESETS, SwitchType, UiOverride};
use serde_json::json;

use crate::api::{clear_override, set_override};
use crate::components::toast::Toasts;
use crate::energy::energy_entities;
use crate::entity_picker::{EntityPickField, Keep};
use crate::format::word;
use crate::house::House;
use crate::i18n::devices::{
    DeviceText, clear_setting, device_text, key_signal, key_value, presets_stored, presets_text,
    text_signal, use_device_text,
};
use crate::i18n::states::switch_type_word;
use crate::i18n::{Key, Lang, current_lang, text, use_lang, use_text};
use crate::icons::consumer_icon;
use crate::screens::alarm_settings::AlarmRoleLine;
use crate::settings_rules::{Shown, clear_field, other_saved, set_type, shown};

/// The fields of `after` that differ from `before`, as a patch the hub accepts: camelCase keys,
/// a cleared `Option` as `null`, flags as booleans, `order` and the brightness as numbers.
/// `extra` (fields written by a newer app) is not editable here and is ignored.
pub fn dirty_patch(before: &UiOverride, after: &UiOverride) -> OverridePatch {
    let mut patch = OverridePatch::new();
    macro_rules! diff {
        ($($field:ident => $key:literal),* $(,)?) => {
            $(
                if before.$field != after.$field {
                    patch.insert($key.to_owned(), json!(after.$field));
                }
            )*
        };
    }
    diff! {
        name => "name",
        area => "area",
        hidden => "hidden",
        order => "order",
        card_style => "cardStyle",
        favorite => "favorite",
        energy_role => "energyRole",
        energy_invert => "energyInvert",
        security_hidden => "securityHidden",
        energy_icon => "energyIcon",
        energy_meter => "energyMeter",
        home_pinned => "homePinned",
        security_action => "securityAction",
        security_ask => "securityAsk",
        ungrouped => "ungrouped",
        show_as => "showAs",
        light_on_preset => "lightOnPreset",
        light_on_brightness => "lightOnBrightness",
        climate_presets => "climatePresets",
        main_control => "mainControl",
        invert => "invert",
    }
    patch
}

type Draft = RwSignal<UiOverride>;
type Text = Option<String>;

/// Blank input is "not set".
fn non_blank(s: String) -> Text {
    (!s.trim().is_empty()).then_some(s)
}

#[component]
fn TextField(
    #[prop(into)] label: Signal<String>,
    draft: Draft,
    get: fn(&UiOverride) -> Text,
    set: fn(&mut UiOverride, Text),
    #[prop(optional, into)] hint: Option<Signal<String>>,
) -> impl IntoView {
    view! {
        <label class="override__field">
            <span class="override__label">{move || label.get()}</span>
            <input
                class="field"
                type="text"
                prop:value=move || draft.with(|d| get(d).unwrap_or_default())
                on:input=move |ev| {
                    let value = non_blank(event_target_value(&ev));
                    draft.update(|d| set(d, value));
                }
            />
            {hint.map(|h| view! { <span class="override__hint">{move || h.get()}</span> })}
        </label>
    }
}

/// The climate presets: stored as comma-separated numbers, typed in the language's own form
/// (French uses `;` between values, since `,` is its decimal mark). The text is kept apart from
/// the draft so typing a trailing separator is not rewritten under the cursor.
#[component]
fn PresetsField(draft: Draft) -> impl IntoView {
    let lang = use_lang();
    let t = use_device_text();
    let stored = move || draft.with(|d| d.climate_presets.clone().unwrap_or_default());
    let text = RwSignal::new(presets_text(current_lang(), &stored()));
    // Follow an outside change and a language change, unless the text already says the same.
    Effect::new(move |previous: Option<Lang>| {
        let (now, current) = (stored(), lang());
        let switched = previous.is_some_and(|p| p != current);
        if switched || text.with_untracked(|t| presets_stored(t)) != now {
            text.set(presets_text(current, &now));
        }
        current
    });
    view! {
        <label class="override__field">
            <span class="override__label">{move || t(DeviceText::TargetPresets)}</span>
            <input
                class="field"
                type="text"
                prop:value=move || text.get()
                on:input=move |ev| {
                    let raw = event_target_value(&ev);
                    let value = non_blank(presets_stored(&raw));
                    draft.update(|d| d.climate_presets = value);
                    text.set(raw);
                }
            />
            <span class="override__hint">{move || t(DeviceText::PresetsHint)}</span>
        </label>
    }
}

#[component]
fn FlagField(
    #[prop(into)] label: Signal<String>,
    draft: Draft,
    get: fn(&UiOverride) -> bool,
    set: fn(&mut UiOverride, bool),
) -> impl IntoView {
    let on = move || draft.with(get);
    view! {
        <label class="override__flag">
            <span class="override__label">{move || label.get()}</span>
            <button
                type="button"
                role="switch"
                class="switch"
                class:switch--on=on
                aria-checked=move || on().to_string()
                on:click=move |_| {
                    let next = !draft.with_untracked(get);
                    draft.update(|d| set(d, next));
                }
            >
                <span class="switch__knob"></span>
            </button>
        </label>
    }
}

/// The text of a chip, resolved in the language when it is drawn.
#[derive(Clone)]
enum ChipLabel {
    Text(DeviceText),
    Key(Key),
    /// What a switch is shown as.
    Type(SwitchType),
    /// A control the device announced: its name in words, "Lock" for `locked`.
    Control(String),
}

impl ChipLabel {
    fn resolve(&self, lang: Lang) -> String {
        match self {
            ChipLabel::Text(k) => device_text(lang, *k).to_owned(),
            ChipLabel::Key(k) => text(lang, *k).to_owned(),
            ChipLabel::Type(t) => switch_type_word(lang, *t).to_owned(),
            ChipLabel::Control(c) => control_label(lang, c),
        }
    }
}

/// One chip of a [`ChipField`]: its text, the value it stands for (`None`: the field's default)
/// and an optional glyph shown before the text.
#[derive(Clone)]
struct ChipOption {
    label: ChipLabel,
    value: Option<String>,
    glyph: Option<&'static str>,
}

fn chip(label: ChipLabel, value: Option<&str>) -> ChipOption {
    ChipOption {
        label,
        value: value.map(str::to_owned),
        glyph: None,
    }
}

/// A row of single-choice chips. A stored value outside `options` (written by a newer app or by
/// hand) shows as one more selected chip with its raw text, so it is never silently replaced.
#[component]
fn ChipField(
    #[prop(into)] label: Signal<String>,
    draft: Draft,
    options: Vec<ChipOption>,
    get: fn(&UiOverride) -> Text,
    on_pick: fn(&mut UiOverride, Option<&str>),
) -> impl IntoView {
    let known: Vec<Option<String>> = options.iter().map(|o| o.value.clone()).collect();
    let unknown = move || {
        draft
            .with(get)
            .filter(|v| !known.contains(&Some(v.clone())))
    };
    let lang = use_lang();
    view! {
        <div class="override__field" role="group" aria-label=move || label.get()>
            <span class="override__label">{move || label.get()}</span>
            <div class="override__chips">
                {options
                    .into_iter()
                    .map(|o| {
                        let value = o.value.clone();
                        let pick = o.value.clone();
                        let option_label = o.label.clone();
                        let selected =
                            Signal::derive(move || draft.with(get).as_deref() == value.as_deref());
                        view! {
                            <button
                                type="button"
                                class="chip"
                                class:chip--selected=selected
                                aria-pressed=move || selected.get().to_string()
                                on:click=move |_| draft.update(|d| on_pick(d, pick.as_deref()))
                            >
                                {o.glyph.map(|g| view! { <span class="chip__glyph" inner_html=g></span> })}
                                {move || option_label.resolve(lang())}
                            </button>
                        }
                    })
                    .collect_view()}
                {move || {
                    unknown()
                        .map(|v| {
                            view! {
                                <button type="button" class="chip chip--selected" aria-pressed="true">
                                    {v}
                                </button>
                            }
                        })
                }}
            </div>
        </div>
    }
}

/// The area: a `<select>` of the hub's areas, or free text through "Other...".
#[component]
fn AreaField(draft: Draft) -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let t = use_device_text();
    let known = move |value: &str| house.areas.with(|a| a.iter().any(|a| a.id == value));
    // Free text mode starts on when the stored area is not one of the hub's.
    let other =
        RwSignal::new(draft.with_untracked(|d| d.area.as_deref().is_some_and(|a| !known(a))));
    let selected = move || {
        if other.get() {
            OTHER.to_owned()
        } else {
            draft.with(|d| d.area.clone().unwrap_or_default())
        }
    };
    const OTHER: &str = "__other__";
    view! {
        <div class="override__field">
            <label class="override__label" for="override-area">{move || t(DeviceText::Room)}</label>
            <select
                id="override-area"
                class="select"
                on:change=move |ev| {
                    let value = event_target_value(&ev);
                    if value == OTHER {
                        other.set(true);
                    } else {
                        other.set(false);
                        let area = non_blank(value);
                        draft.update(|d| d.area = area);
                    }
                }
            >
                <option value="" prop:selected=move || selected().is_empty()>{move || t(DeviceText::Default)}</option>
                <For
                    each=move || house.areas.get()
                    key=|a| (a.id.clone(), a.name.clone())
                    children=move |area| {
                        let id = area.id.clone();
                        view! {
                            <option value=id.clone() prop:selected=move || selected() == id>
                                {move || crate::format::area_name(lang(), &area.id, &area.name)}
                            </option>
                        }
                    }
                />
                <option value=OTHER prop:selected=move || other.get()>{move || t(DeviceText::Other)}</option>
            </select>
            <Show when=move || other.get()>
                <input
                    class="field"
                    type="text"
                    aria-label=move || t(DeviceText::RoomName)
                    prop:value=move || draft.with(|d| d.area.clone().unwrap_or_default())
                    on:input=move |ev| {
                        let area = non_blank(event_target_value(&ev));
                        draft.update(|d| d.area = area);
                    }
                />
            </Show>
        </div>
    }
}

/// The energy counter shown on a power sensor's card: "Automatic" or one of the house's
/// counters. A stored id that no longer names a counter stays selected under its raw id.
#[component]
fn CounterField(draft: Draft) -> impl IntoView {
    let house = expect_context::<House>();
    let t = use_device_text();
    let counters = Memo::new(move |_| {
        let all = house.entities.with(energy_entities);
        let mut list: Vec<(String, String)> = EnergyMeters::counters(&all)
            .into_iter()
            .map(|e| (e.id.clone(), e.name.clone()))
            .collect();
        list.sort_by_cached_key(|(id, name)| (name.to_lowercase(), id.clone()));
        list
    });
    let pool: Keep = Arc::new(move |e| counters.with(|c| c.iter().any(|(id, _)| *id == e.id)));
    view! {
        <EntityPickField
            label=Signal::derive(move || t(DeviceText::EnergyCounter).to_owned())
            value=Signal::derive(move || draft.with(|d| d.energy_meter.clone()))
            on_change=Callback::new(move |v| draft.update(|d| d.energy_meter = v))
            pool=Some(pool)
            pinned=Some(Signal::derive(move || t(DeviceText::Automatic).to_owned()))
            class=Some("override__field".to_owned())
        />
    }
}

/// The light-on brightness slider (1..=255, the hub rejects 0) with a reset to "Default".
#[component]
fn BrightnessField(draft: Draft) -> impl IntoView {
    let t = use_device_text();
    let tk = use_text();
    view! {
        <div class="override__field">
            <span class="override__label">{move || tk(Key::Brightness)}</span>
            <div class="override__slider">
                <input
                    class="slider"
                    type="range"
                    min="1"
                    max="255"
                    aria-label=move || t(DeviceText::BrightnessOnTurnOn)
                    prop:value=move || {
                        draft.with(|d| d.light_on_brightness.unwrap_or(255)).to_string()
                    }
                    on:input=move |ev| {
                        if let Ok(n) = event_target_value(&ev).parse::<i32>() {
                            draft.update(|d| d.light_on_brightness = Some(n));
                        }
                    }
                />
                <span class="override__value">
                    {move || {
                        draft
                            .with(|d| d.light_on_brightness)
                            .map_or_else(|| t(DeviceText::Default).to_owned(), |n| n.to_string())
                    }}
                </span>
                <button
                    type="button"
                    class="btn btn--compact override__reset"
                    disabled=move || draft.with(|d| d.light_on_brightness.is_none())
                    on:click=move |_| draft.update(|d| d.light_on_brightness = None)
                >
                    {move || t(DeviceText::Reset)}
                </button>
            </div>
        </div>
    }
}

/// A titled block of the form, left out entirely while it has nothing to show.
#[component]
fn Section(
    #[prop(into)] title: Signal<String>,
    #[prop(into)] when: Signal<bool>,
    children: ChildrenFn,
) -> impl IntoView {
    view! {
        <Show when=move || when.get()>
            <fieldset class="override__group">
                <legend class="override__legend">{move || title.get()}</legend>
                {children()}
            </fieldset>
        </Show>
    }
}

/// Values set for controls the form does not show, each with a button that clears it.
#[component]
fn OtherSaved(draft: Draft, shown: Memo<Shown>) -> impl IntoView {
    let t = use_device_text();
    let lang = use_lang();
    let entries = Memo::new(move |_| draft.with(|d| shown.with(|s| other_saved(d, s))));
    view! {
        <Show when=move || entries.with(|e| !e.is_empty())>
            <div class="override__field">
                <span class="override__label">{move || t(DeviceText::OtherSaved)}</span>
                <ul class="override__saved">
                    <For
                        each=move || entries.get()
                        key=|(key, value)| (*key, value.clone())
                        children=move |(key, value)| {
                            view! {
                                <li class="override__saved-row">
                                    <span>{{
                                        let value = value.clone();
                                        move || key_value(lang(), key, &value)
                                    }}</span>
                                    <button
                                        type="button"
                                        class="btn btn--compact"
                                        aria-label=move || clear_setting(lang(), key)
                                        on:click=move |_| draft.update(|d| clear_field(d, key))
                                    >
                                        "×"
                                    </button>
                                </li>
                            }
                        }
                    />
                </ul>
            </div>
        </Show>
    }
}

fn energy_role_label(role: &str) -> ChipLabel {
    match role {
        "solar" => ChipLabel::Text(DeviceText::Solar),
        "grid" => ChipLabel::Text(DeviceText::Grid),
        "grid_export" => ChipLabel::Text(DeviceText::GridExport),
        "battery" => ChipLabel::Key(Key::Battery),
        "house" => ChipLabel::Key(Key::House),
        "consumer" => ChipLabel::Text(DeviceText::Consumer),
        "water" => ChipLabel::Text(DeviceText::Water),
        _ => ChipLabel::Text(DeviceText::Unknown),
    }
}

fn light_on_label(preset: &str) -> DeviceText {
    match preset {
        "white" => DeviceText::White,
        "warm" => DeviceText::Warm,
        "red" => DeviceText::Red,
        "green" => DeviceText::Green,
        "blue" => DeviceText::Blue,
        _ => DeviceText::Unknown,
    }
}

/// The consumer icons, in the app's order, with their labels.
const CONSUMER_ICONS: [(&str, DeviceText); 7] = [
    ("water_heater", DeviceText::WaterHeater),
    ("heat_pump", DeviceText::HeatPump),
    ("oven", DeviceText::Oven),
    ("fridge", DeviceText::Fridge),
    ("washer", DeviceText::Washer),
    ("dryer", DeviceText::Dryer),
    ("dishwasher", DeviceText::Dishwasher),
];

fn type_options() -> Vec<ChipOption> {
    SwitchType::ALL
        .into_iter()
        .map(|t| chip(ChipLabel::Type(t), t.key()))
        .collect()
}

/// The entity's announced controls as chips, then "None" (hide the main control).
fn main_control_options(entity: &EntityView) -> Vec<ChipOption> {
    let controls = entity
        .capabilities
        .generic
        .as_ref()
        .map(|g| g.controls.as_slice())
        .unwrap_or_default();
    controls
        .iter()
        .map(|c| ChipOption {
            label: ChipLabel::Control(c.clone()),
            value: Some(c.clone()),
            glyph: None,
        })
        .chain(std::iter::once(chip(
            ChipLabel::Text(DeviceText::NoneChoice),
            Some("none"),
        )))
        .collect()
}

/// `locked` reads "Lock" / "Verrou"; the rest is the snake_case name in sentence case.
fn control_label(lang: Lang, control: &str) -> String {
    match control {
        "locked" => device_text(lang, DeviceText::LockControl).into(),
        other => word(other),
    }
}

fn light_on_options() -> Vec<ChipOption> {
    std::iter::once(chip(ChipLabel::Text(DeviceText::LastState), None))
        .chain(
            LIGHT_ON_PRESETS
                .iter()
                .map(|p| chip(ChipLabel::Text(light_on_label(p)), Some(*p))),
        )
        .collect()
}

fn energy_role_options() -> Vec<ChipOption> {
    std::iter::once(chip(ChipLabel::Text(DeviceText::NoneChoice), None))
        .chain(
            ENERGY_ROLES
                .iter()
                .map(|r| chip(energy_role_label(r), Some(*r))),
        )
        .collect()
}

fn icon_options() -> Vec<ChipOption> {
    std::iter::once(ChipOption {
        label: ChipLabel::Text(DeviceText::Plug),
        value: None,
        glyph: Some(consumer_icon(None)),
    })
    .chain(CONSUMER_ICONS.iter().map(|(key, label)| ChipOption {
        label: ChipLabel::Text(*label),
        value: Some((*key).to_owned()),
        glyph: Some(consumer_icon(ConsumerIcon::parse(key))),
    }))
    .collect()
}

#[component]
pub fn OverrideEditor(entity: EntityView) -> impl IntoView {
    let house = expect_context::<House>();
    let toasts = expect_context::<Toasts>();
    let id = entity.id.clone();

    // What the hub currently has; follows changes from outside (the app, MCP).
    let stored = {
        let id = id.clone();
        Memo::new(move |_| {
            house
                .entities
                .with(|m| m.get(&id).map(|e| e.ui.clone()))
                .unwrap_or_default()
        })
    };
    let draft: Draft = RwSignal::new(entity.ui.clone());
    let patch = Memo::new(move |_| stored.with(|s| draft.with(|d| dirty_patch(s, d))));
    let dirty = Memo::new(move |_| patch.with(|p| !p.is_empty()));
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    // Take over an outside change only while the form has no unsaved edits.
    Effect::new(move |previous: Option<UiOverride>| {
        let now = stored.get();
        if let Some(previous) = previous
            && now != previous
            && draft.with_untracked(|d| dirty_patch(&previous, d).is_empty())
        {
            draft.set(now.clone());
        }
        now
    });

    let save = {
        let id = id.clone();
        move |_| {
            let patch = patch.get_untracked();
            if patch.is_empty() || busy.get_untracked() {
                return;
            }
            busy.set(true);
            error.set(None);
            let id = id.clone();
            spawn_local(async move {
                match set_override(&id, patch).await {
                    Ok(view) => {
                        draft.set(view.ui.clone());
                        house.entities.update(|m| {
                            m.insert(view.id.clone(), view);
                        });
                        toasts.push(device_text(current_lang(), DeviceText::Saved));
                    }
                    Err(e) => error.set(Some(e.error)),
                }
                busy.set(false);
            });
        }
    };

    let clear = {
        let id = id.clone();
        move |_| {
            if busy.get_untracked() {
                return;
            }
            busy.set(true);
            error.set(None);
            let id = id.clone();
            spawn_local(async move {
                match clear_override(&id).await {
                    Ok(()) => {
                        draft.set(UiOverride::default());
                        house.entities.update(|m| {
                            if let Some(e) = m.get_mut(&id) {
                                e.ui = UiOverride::default();
                            }
                        });
                        toasts.push(device_text(current_lang(), DeviceText::OverrideCleared));
                    }
                    Err(e) => error.set(Some(e.error)),
                }
                busy.set(false);
            });
        }
    };

    let options_entity = entity.clone();
    let rules_entity = entity.clone();
    let shown = Memo::new(move |_| draft.with(|d| shown(&rules_entity, d)));
    let group_label = text_signal(if entity.protocol_kind == Kind::Cover {
        DeviceText::GroupShutters
    } else {
        DeviceText::GroupLights
    });
    let tt = text_signal;
    let t = use_device_text();
    let on = move |pick: fn(&Shown) -> bool| Signal::derive(move || shown.with(pick));

    view! {
        <AlarmRoleLine id=id.clone() />
        <form class="override" on:submit=|ev| ev.prevent_default()>
            <fieldset class="override__group">
                <legend class="override__legend">{move || t(DeviceText::General)}</legend>
                <TextField
                    label=tt(DeviceText::Name)
                    draft=draft
                    get=|d| d.name.clone()
                    set=|d, v| d.name = v
                    hint=tt(DeviceText::NameHint)
                />
                <AreaField draft=draft />
                <FlagField label=tt(DeviceText::Favorite) draft=draft get=|d| d.favorite set=|d, v| d.favorite = v />
                <Show
                    when=move || shown.with(|s| s.show_on_home)
                    fallback=move || {
                        view! {
                            <FlagField
                                label=tt(DeviceText::HideFromHome)
                                draft=draft
                                get=|d| d.hidden
                                set=|d, v| d.hidden = v
                            />
                        }
                    }
                >
                    <FlagField
                        label=tt(DeviceText::ShowOnHome)
                        draft=draft
                        get=|d| d.home_pinned && !d.hidden
                        set=|d, v| {
                            d.home_pinned = v;
                            if v {
                                d.hidden = false;
                            }
                        }
                    />
                </Show>
            </fieldset>
            <Section title=tt(DeviceText::Type) when=on(|s| s.show_as)>
                <ChipField
                    label=tt(DeviceText::ShowAs)
                    draft=draft
                    options=type_options()
                    get=|d| d.show_as.clone()
                    on_pick=|d, v| {
                        let t = SwitchType::ALL
                            .into_iter()
                            .find(|t| t.key() == v)
                            .unwrap_or(SwitchType::Switch);
                        set_type(d, t);
                    }
                />
                <FlagField
                    label=tt(DeviceText::Inverted)
                    draft=draft
                    get=|d| d.invert
                    set=|d, v| d.invert = v
                />
            </Section>
            <Section title=tt(DeviceText::MainControl) when=on(|s| s.main_control)>
                <ChipField
                    label=tt(DeviceText::MainControl)
                    draft=draft
                    options=main_control_options(&options_entity)
                    get=|d| d.main_control.clone()
                    on_pick=|d, v| d.main_control = v.map(str::to_owned)
                />
            </Section>
            <Section title=tt(DeviceText::Light) when=on(|s| s.group_with_room || s.light_on)>
                <Show when=move || shown.with(|s| s.group_with_room)>
                    <FlagField
                        label=group_label
                        draft=draft
                        get=|d| !d.ungrouped
                        set=|d, v| d.ungrouped = !v
                    />
                </Show>
                <Show when=move || shown.with(|s| s.light_on)>
                    <ChipField
                        label=tt(DeviceText::TurnOnIn)
                        draft=draft
                        options=light_on_options()
                        get=|d| d.light_on_preset.clone()
                        on_pick=|d, v| {
                            d.light_on_preset = v.map(str::to_owned);
                            if v.is_none() {
                                d.light_on_brightness = None;
                            }
                        }
                    />
                </Show>
                <Show when=move || shown.with(|s| s.light_on_brightness)>
                    <BrightnessField draft=draft />
                </Show>
            </Section>
            <Section title=key_signal(Key::Security) when=on(|s| s.security_shown || s.security_action)>
                <Show when=move || shown.with(|s| s.security_shown)>
                    <FlagField
                        label=tt(DeviceText::ShowOnSecurity)
                        draft=draft
                        get=|d| !d.security_hidden
                        set=|d, v| d.security_hidden = !v
                    />
                </Show>
                <Show when=move || shown.with(|s| s.security_action)>
                    <FlagField
                        label=tt(DeviceText::ShowOnSecurity)
                        draft=draft
                        get=|d| d.security_action
                        set=|d, v| d.security_action = v
                    />
                </Show>
                <Show when=move || shown.with(|s| s.security_ask)>
                    <FlagField
                        label=tt(DeviceText::AskBeforeActing)
                        draft=draft
                        get=|d| d.security_ask
                        set=|d, v| d.security_ask = v
                    />
                </Show>
            </Section>
            <Section title=tt(DeviceText::DisplayAndEnergy) when=on(|s| s.card_style || s.energy_role)>
                <Show when=move || shown.with(|s| s.card_style)>
                    <ChipField
                        label=tt(DeviceText::CardStyle)
                        draft=draft
                        options=vec![
                            chip(ChipLabel::Text(DeviceText::Standard), None),
                            chip(ChipLabel::Text(DeviceText::HeaderTemperature), Some("hero_weather")),
                        ]
                        get=|d| d.card_style.clone()
                        on_pick=|d, v| d.card_style = v.map(str::to_owned)
                    />
                </Show>
                <Show when=move || shown.with(|s| s.energy_role)>
                    <ChipField
                        label=tt(DeviceText::EnergyRole)
                        draft=draft
                        options=energy_role_options()
                        get=|d| d.energy_role.clone()
                        on_pick=|d, v| d.energy_role = v.map(str::to_owned)
                    />
                </Show>
                <Show when=move || shown.with(|s| s.unit_warning)>
                    <p class="override__hint override__hint--error">{move || t(DeviceText::NeedsPowerUnit)}</p>
                </Show>
                <Show when=move || shown.with(|s| s.energy_invert)>
                    <FlagField
                        label=tt(DeviceText::InvertSign)
                        draft=draft
                        get=|d| d.energy_invert
                        set=|d, v| d.energy_invert = v
                    />
                </Show>
                <Show when=move || shown.with(|s| s.energy_icon)>
                    <ChipField
                        label=tt(DeviceText::Icon)
                        draft=draft
                        options=icon_options()
                        get=|d| d.energy_icon.clone()
                        on_pick=|d, v| d.energy_icon = v.map(str::to_owned)
                    />
                </Show>
                <Show when=move || shown.with(|s| s.energy_meter)>
                    <CounterField draft=draft />
                </Show>
            </Section>
            <Section title=key_signal(Key::Climate) when=on(|s| s.climate_presets)>
                <PresetsField draft=draft />
            </Section>
            <OtherSaved draft=draft shown=shown />
            <p class="override__error" role="alert" hidden=move || error.with(Option::is_none)>
                {move || error.get().unwrap_or_default()}
            </p>
            <div class="override__actions">
                <button
                    type="button"
                    class="btn override__save"
                    disabled=move || !dirty.get() || busy.get()
                    on:click=save
                >
                    {move || t(DeviceText::Save)}
                </button>
                <button
                    type="button"
                    class="btn"
                    disabled=move || busy.get()
                    on:click=clear
                >
                    {move || t(DeviceText::ResetToDefaults)}
                </button>
            </div>
        </form>
    }
}

#[cfg(test)]
mod tests {
    use super::dirty_patch;
    use nidoo_core::model::UiOverride;
    use serde_json::json;

    #[test]
    fn unchanged_fields_are_absent() {
        let before = UiOverride::default();
        let after = UiOverride {
            favorite: true,
            ..UiOverride::default()
        };
        let patch = dirty_patch(&before, &after);
        assert_eq!(
            patch,
            json!({"favorite": true}).as_object().cloned().unwrap()
        );
    }

    #[test]
    fn cleared_text_sends_null() {
        let before = UiOverride {
            name: Some("foo".into()),
            ..UiOverride::default()
        };
        let patch = dirty_patch(&before, &UiOverride::default());
        assert_eq!(patch, json!({"name": null}).as_object().cloned().unwrap());
    }

    #[test]
    fn brightness_and_order_are_numbers() {
        let after = UiOverride {
            light_on_brightness: Some(200),
            order: Some(7),
            ..UiOverride::default()
        };
        let patch = dirty_patch(&UiOverride::default(), &after);
        assert_eq!(
            patch,
            json!({"lightOnBrightness": 200, "order": 7})
                .as_object()
                .cloned()
                .unwrap()
        );
    }

    #[test]
    fn main_control_patch_uses_camel_case() {
        let after = UiOverride {
            main_control: Some("locked".into()),
            ..UiOverride::default()
        };
        let patch = dirty_patch(&UiOverride::default(), &after);
        assert_eq!(
            patch,
            json!({"mainControl": "locked"})
                .as_object()
                .cloned()
                .unwrap()
        );
    }

    #[test]
    fn clearing_main_control_sends_null() {
        let before = UiOverride {
            main_control: Some("locked".into()),
            ..UiOverride::default()
        };
        let patch = dirty_patch(&before, &UiOverride::default());
        assert_eq!(
            patch,
            json!({"mainControl": null}).as_object().cloned().unwrap()
        );
    }

    #[test]
    fn every_known_field_is_compared() {
        let after = UiOverride {
            name: Some("n".into()),
            area: Some("a".into()),
            hidden: true,
            order: Some(1),
            card_style: Some("c".into()),
            favorite: true,
            energy_role: Some("solar".into()),
            energy_invert: true,
            security_hidden: true,
            energy_icon: Some("i".into()),
            energy_meter: Some("m".into()),
            home_pinned: true,
            security_action: true,
            security_ask: false,
            ungrouped: true,
            show_as: Some("door".into()),
            light_on_preset: Some("warm".into()),
            light_on_brightness: Some(9),
            climate_presets: Some("p".into()),
            main_control: Some("locked".into()),
            invert: true,
            ..UiOverride::default()
        };
        let patch = dirty_patch(&UiOverride::default(), &after);
        let mut keys: Vec<&str> = patch.keys().map(String::as_str).collect();
        let mut known = UiOverride::FIELDS.to_vec();
        keys.sort_unstable();
        known.sort_unstable();
        assert_eq!(keys, known);
        assert!(dirty_patch(&after, &after).is_empty());
    }
}
