//! Settings → Alarm: the native alarm's sensors, sirens, remotes, presence and timing behind
//! one Save, plus the pure helpers the Security hero, the settings list and the device sheet
//! share.

use std::collections::BTreeSet;

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use nidoo_alarm::doc::{
    AlarmSettings, ArmMode, Entry, MqttSiren, Remote, RemoteAction, SensorKind, SensorRule, Siren,
    default_sensor, default_signals,
};
use nidoo_alarm::view::AlarmView;
use nidoo_api::EntityView;
use nidoo_core::model::Kind;

use crate::api::alarm as api;
use crate::components::toast::Toasts;
use crate::entity_picker::EntityAddButton;
use crate::entity_picker::needs::{is_remote, is_siren, is_tracker, keep};
use crate::format::entity_title;
use crate::house::House;
use crate::i18n::{AlarmText, Lang, alarm_text, delayed_with, use_lang};

/// What `device_class` says about a binary sensor the alarm can watch.
fn sensor_kind_of(device_class: Option<&str>) -> Option<SensorKind> {
    match device_class? {
        "door" | "window" | "opening" | "garage_door" => Some(SensorKind::Opening),
        "motion" | "occupancy" | "presence" => Some(SensorKind::Motion),
        _ => None,
    }
}

/// The binary sensors the alarm could watch, with the kind their `device_class` implies. The
/// ones already in `listed` are left out.
pub fn sensor_candidates(
    entities: &[EntityView],
    listed: &[SensorRule],
) -> Vec<(String, SensorKind)> {
    entities
        .iter()
        .filter(|e| e.kind == Kind::BinarySensor && !listed.iter().any(|r| r.entity == e.id))
        .filter_map(|e| Some((e.id.clone(), sensor_kind_of(e.device_class.as_deref())?)))
        .collect()
}

/// Whether the alarm can watch `e`: a binary sensor whose `device_class` is an opening or motion.
fn is_alarm_sensor(e: &EntityView) -> bool {
    e.kind == Kind::BinarySensor && sensor_kind_of(e.device_class.as_deref()).is_some()
}

fn mode_text(mode: ArmMode) -> AlarmText {
    match mode {
        ArmMode::Away => AlarmText::ModeAway,
        ArmMode::Home => AlarmText::ModeHome,
        ArmMode::Night => AlarmText::ModeNight,
    }
}

fn modes_text(lang: Lang, modes: &BTreeSet<ArmMode>) -> String {
    modes
        .iter()
        .map(|m| alarm_text(lang, mode_text(*m)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// How `entity` takes part in the alarm, for the device sheet: "In alarm: delayed opening ·
/// away, home, night". `None` when the alarm does not use it.
pub fn alarm_line(settings: &AlarmSettings, entity: &str, lang: Lang) -> Option<String> {
    let t = |k| alarm_text(lang, k);
    let mut roles: Vec<String> = Vec::new();
    if let Some(rule) = settings.sensor(entity) {
        let what = match (rule.kind, rule.entry) {
            (SensorKind::Motion, _) => t(AlarmText::Motion).to_owned(),
            (SensorKind::Opening, entry) => {
                let entry = t(match entry {
                    Entry::Delayed => AlarmText::Delayed,
                    Entry::Instant => AlarmText::Instant,
                });
                // French puts the adjective after the noun.
                if lang == Lang::Fr {
                    format!("{} {entry}", t(AlarmText::Opening))
                } else {
                    format!("{entry} {}", t(AlarmText::Opening))
                }
            }
        };
        roles.push(format!("{what} · {}", modes_text(lang, &rule.modes)));
    }
    if settings
        .sirens
        .iter()
        .any(|s| matches!(s, Siren::Entity { entity: e, .. } if e == entity))
    {
        roles.push(t(AlarmText::RoleSiren).to_owned());
    }
    if settings.remotes.iter().any(|r| r.entity == entity) {
        roles.push(t(AlarmText::RoleRemote).to_owned());
    }
    if settings.presence.trackers.iter().any(|e| e == entity) {
        roles.push(t(AlarmText::RolePresence).to_owned());
    }
    (!roles.is_empty()).then(|| format!("{}{}", t(AlarmText::InAlarm), roles.join(" · ")))
}

/// Minutes and seconds left until `deadline`: "0:27". A deadline in the past reads "0:00".
pub fn countdown_text(deadline: u64, now: u64) -> String {
    let left = deadline.saturating_sub(now);
    format!("{}:{:02}", left / 60, left % 60)
}

/// The Settings list row's status: "Off" while disabled, else the panel state.
pub fn settings_status(view: &AlarmView, lang: Lang) -> String {
    let t = |k| alarm_text(lang, k).to_owned();
    if !view.enabled {
        return t(AlarmText::StatusOff);
    }
    match view.state.as_str() {
        "disarmed" => t(AlarmText::StateDisarmed),
        "arming" => t(AlarmText::StateArming),
        "pending" => t(AlarmText::StatePending),
        "triggered" => t(AlarmText::StateTriggered),
        s if s.starts_with("armed_") => {
            let mode = match s {
                "armed_away" => Some(ArmMode::Away),
                "armed_home" => Some(ArmMode::Home),
                "armed_night" => Some(ArmMode::Night),
                _ => None,
            };
            match mode {
                Some(m) => format!(
                    "{} · {}",
                    alarm_text(lang, AlarmText::StateArmed),
                    alarm_text(lang, mode_text(m))
                ),
                None => t(AlarmText::StateArmed),
            }
        }
        other => other.to_owned(),
    }
}

fn siren_has_chirp(siren: &Siren) -> bool {
    match siren {
        Siren::Entity { chirp_ms, .. } => chirp_ms.is_some(),
        Siren::Mqtt { mqtt } => mqtt.chirp.as_deref().is_some_and(|c| !c.is_empty()),
    }
}

/// A signal name for a new row: the first announced one the remote does not map yet, else a
/// placeholder that is not taken.
fn next_signal(
    announced: &[String],
    taken: &std::collections::BTreeMap<String, RemoteAction>,
) -> String {
    if let Some(name) = announced.iter().find(|n| !taken.contains_key(*n)) {
        return name.clone();
    }
    let mut name = "signal".to_owned();
    let mut n = 2;
    while taken.contains_key(&name) {
        name = format!("signal{n}");
        n += 1;
    }
    name
}

fn action_text(a: RemoteAction) -> AlarmText {
    match a {
        RemoteAction::Away => AlarmText::ActionAway,
        RemoteAction::Home => AlarmText::ActionHome,
        RemoteAction::Night => AlarmText::ActionNight,
        RemoteAction::Disarm => AlarmText::ActionDisarm,
        RemoteAction::Sos => AlarmText::ActionSos,
    }
}

const ACTIONS: [(RemoteAction, &str); 5] = [
    (RemoteAction::Away, "away"),
    (RemoteAction::Home, "home"),
    (RemoteAction::Night, "night"),
    (RemoteAction::Disarm, "disarm"),
    (RemoteAction::Sos, "sos"),
];

fn action_from_key(key: &str) -> Option<RemoteAction> {
    ACTIONS.iter().find(|(_, k)| *k == key).map(|(a, _)| *a)
}

/// The settings to save: each field from `draft` when it was edited (differs from `base`, the
/// copy the edits started from), else from `latest`, so a change made elsewhere meanwhile is not
/// undone.
pub fn merge(base: &AlarmSettings, draft: &AlarmSettings, latest: &AlarmSettings) -> AlarmSettings {
    fn pick<T: Clone + PartialEq>(base: &T, draft: &T, latest: &T) -> T {
        if draft != base {
            draft.clone()
        } else {
            latest.clone()
        }
    }
    AlarmSettings {
        enabled: pick(&base.enabled, &draft.enabled, &latest.enabled),
        entry_delay_s: pick(
            &base.entry_delay_s,
            &draft.entry_delay_s,
            &latest.entry_delay_s,
        ),
        exit_delay_s: pick(
            &base.exit_delay_s,
            &draft.exit_delay_s,
            &latest.exit_delay_s,
        ),
        siren_s: pick(&base.siren_s, &draft.siren_s, &latest.siren_s),
        motion: pick(&base.motion, &draft.motion, &latest.motion),
        sensors: pick(&base.sensors, &draft.sensors, &latest.sensors),
        sirens: pick(&base.sirens, &draft.sirens, &latest.sirens),
        remotes: pick(&base.remotes, &draft.remotes, &latest.remotes),
        presence: pick(&base.presence, &draft.presence, &latest.presence),
    }
}

type Draft = RwSignal<AlarmSettings>;
/// The title of `id`, or the id itself when the house no longer has it.
fn label_of(house: House, lang: Lang, id: &str) -> String {
    house
        .entities
        .with(|m| m.get(id).map(|e| entity_title(lang, e)))
        .unwrap_or_else(|| id.to_owned())
}

/// The name of a chosen entity: its title, and the device it belongs to on a second line.
#[component]
fn EntityName(house: House, id: String) -> impl IntoView {
    let lang = use_lang();
    let device = {
        let id = id.clone();
        move || {
            house
                .entities
                .with(|m| m.get(&id).and_then(|e| e.device_name.clone()))
        }
    };
    view! {
        <span class="alarm-row__name">
            {move || label_of(house, lang(), &id)}
            {move || {
                device().map(|d| view! { <span class="alarm-row__device">{d}</span> })
            }}
        </span>
    }
}

/// An on/off switch row.
#[component]
pub(crate) fn SwitchRow(
    #[prop(into)] label: Signal<String>,
    #[prop(into)] on: Signal<bool>,
    on_toggle: Callback<bool>,
    /// While true the switch ignores clicks.
    #[prop(into, optional)]
    busy: Signal<bool>,
) -> impl IntoView {
    view! {
        <label class="override__flag">
            <span class="override__label">{move || label.get()}</span>
            <button
                type="button"
                role="switch"
                class="switch"
                class:switch--on=move || on.get()
                aria-checked=move || on.get().to_string()
                disabled=move || busy.get()
                on:click=move |_| on_toggle.run(!on.get_untracked())
            >
                <span class="switch__knob"></span>
            </button>
        </label>
    }
}

/// A whole-number field that commits when it loses focus.
#[component]
fn NumField(
    draft: Draft,
    label: AlarmText,
    get: fn(&AlarmSettings) -> u32,
    set: fn(&mut AlarmSettings, u32),
) -> impl IntoView {
    let lang = use_lang();
    view! {
        <label class="climate-field">
            <span class="climate-field__label">{move || alarm_text(lang(), label)}</span>
            <input
                class="field"
                type="number"
                inputmode="numeric"
                min="0"
                prop:value=move || draft.with(|d| get(d).to_string())
                on:change=move |ev| {
                    if let Ok(v) = event_target_value(&ev).trim().parse::<u32>() {
                        draft.update(|d| set(d, v));
                    }
                }
            />
        </label>
    }
}

/// A text field of an MQTT siren, on siren `i`.
#[component]
fn MqttField(
    draft: Draft,
    i: usize,
    label: AlarmText,
    get: fn(&MqttSiren) -> String,
    set: fn(&mut MqttSiren, String),
) -> impl IntoView {
    let lang = use_lang();
    view! {
        <label class="climate-field">
            <span class="climate-field__label">{move || alarm_text(lang(), label)}</span>
            <input
                class="field"
                type="text"
                autocomplete="off"
                spellcheck="false"
                prop:value=move || {
                    draft
                        .with(|d| match d.sirens.get(i) {
                            Some(Siren::Mqtt { mqtt }) => get(mqtt),
                            _ => String::new(),
                        })
                }
                on:input=move |ev| {
                    let v = event_target_value(&ev);
                    draft
                        .update(|d| {
                            if let Some(Siren::Mqtt { mqtt }) = d.sirens.get_mut(i) {
                                set(mqtt, v);
                            }
                        });
                }
            />
        </label>
    }
}

#[component]
fn SensorsSection(draft: Draft, house: House) -> impl IntoView {
    let lang = use_lang();
    let t = move |k: AlarmText| alarm_text(lang(), k);
    let taken = Signal::derive(move || {
        draft.with(|d| {
            d.sensors
                .iter()
                .map(|r| r.entity.clone())
                .collect::<Vec<_>>()
        })
    });
    let count = Memo::new(move |_| draft.with(|d| d.sensors.len()));
    let toggle = Callback::new(move |id: String| {
        if draft.with_untracked(|d| d.sensors.iter().any(|r| r.entity == id)) {
            draft.update(|d| d.sensors.retain(|r| r.entity != id));
            return;
        }
        let kind = house.entities.with_untracked(|m| {
            m.get(&id)
                .and_then(|e| sensor_kind_of(e.device_class.as_deref()))
        });
        if let Some(kind) = kind {
            draft.update(|d| d.sensors.push(default_sensor(&id, kind)));
        }
    });
    view! {
        <section class="sheet__panel alarm-section">
            <h2 class="sheet__heading">{move || t(AlarmText::SensorsTitle)}</h2>
            <p class="settings__hint">{move || t(AlarmText::SensorsHint)}</p>
            <Show when=move || count.get() == 0>
                <p class="settings__empty">{move || t(AlarmText::SensorsEmpty)}</p>
            </Show>
            <ul class="alarm-rows">
                {move || {
                    (0..count.get())
                        .map(|i| {
                            let rule = draft.with_untracked(|d| d.sensors[i].clone());
                            let name = view! { <EntityName house=house id=rule.entity.clone() /> };
                            let kind_word = match rule.kind {
                                SensorKind::Opening => AlarmText::Opening,
                                SensorKind::Motion => AlarmText::Motion,
                            };
                            let modes = [ArmMode::Away, ArmMode::Home, ArmMode::Night]
                                .into_iter()
                                .map(|m| {
                                    let checked = move || {
                                        draft
                                            .with(|d| {
                                                d.sensors.get(i).is_some_and(|r| r.modes.contains(&m))
                                            })
                                    };
                                    view! {
                                        <label class="alarm-check">
                                            <input
                                                type="checkbox"
                                                prop:checked=checked
                                                on:change=move |ev| {
                                                    let on = event_target_checked(&ev);
                                                    draft
                                                        .update(|d| {
                                                            if let Some(r) = d.sensors.get_mut(i) {
                                                                if on {
                                                                    r.modes.insert(m);
                                                                } else {
                                                                    r.modes.remove(&m);
                                                                }
                                                            }
                                                        });
                                                }
                                            />
                                            <span>{move || t(mode_text(m))}</span>
                                        </label>
                                    }
                                })
                                .collect_view();
                            let entry = (rule.kind == SensorKind::Opening)
                                .then(|| {
                                    let option = move |value: Entry, word: AlarmText| {
                                        let on = move || {
                                            draft
                                                .with(|d| d.sensors.get(i).is_some_and(|r| r.entry == value))
                                        };
                                        view! {
                                            <button
                                                type="button"
                                                class="chip segmented__option"
                                                class:chip--selected=on
                                                aria-pressed=move || on().to_string()
                                                title=move || {
                                                    (value == Entry::Delayed)
                                                        .then(|| t(AlarmText::DelayedHint))
                                                }
                                                on:click=move |_| {
                                                    draft
                                                        .update(|d| {
                                                            if let Some(r) = d.sensors.get_mut(i) {
                                                                r.entry = value;
                                                            }
                                                        });
                                                }
                                            >
                                                {move || match value {
                                                    Entry::Delayed => {
                                                        delayed_with(
                                                            lang(),
                                                            draft.with(|d| d.entry_delay_s),
                                                        )
                                                    }
                                                    _ => t(word).to_owned(),
                                                }}
                                            </button>
                                        }
                                    };
                                    view! {
                                        <div class="segmented">
                                            {option(Entry::Instant, AlarmText::Instant)}
                                            {option(Entry::Delayed, AlarmText::Delayed)}
                                        </div>
                                    }
                                });
                            view! {
                                <li class="alarm-row">
                                    <div class="alarm-row__head">
                                        {name}
                                        <span class="alarm-row__kind">{move || t(kind_word)}</span>
                                        <button
                                            type="button"
                                            class="btn btn--compact"
                                            on:click=move |_| {
                                                draft
                                                    .update(|d| {
                                                        if i < d.sensors.len() {
                                                            d.sensors.remove(i);
                                                        }
                                                    });
                                            }
                                        >
                                            {move || t(AlarmText::Remove)}
                                        </button>
                                    </div>
                                    <div class="alarm-row__body">
                                        <div class="alarm-row__modes">{modes}</div>
                                        {entry}
                                    </div>
                                </li>
                            }
                        })
                        .collect_view()
                }}
            </ul>
            <EntityAddButton
                label=Signal::derive(move || t(AlarmText::AddSensor).to_owned())
                taken=taken
                on_toggle=toggle
                pool=Some(keep(is_alarm_sensor))
            />
        </section>
    }
}

#[component]
fn SirensSection(draft: Draft, house: House, saved: Signal<Vec<Siren>>) -> impl IntoView {
    let lang = use_lang();
    let t = move |k: AlarmText| alarm_text(lang(), k);
    let taken = Signal::derive(move || {
        draft.with(|d| {
            d.sirens
                .iter()
                .filter_map(|s| match s {
                    Siren::Entity { entity, .. } => Some(entity.clone()),
                    Siren::Mqtt { .. } => None,
                })
                .collect::<Vec<_>>()
        })
    });
    let count = Memo::new(move |_| draft.with(|d| d.sirens.len()));
    let unsaved = Memo::new(move |_| draft.with(|d| saved.with(|s| d.sirens != *s)));
    let toasts = use_context::<Toasts>();
    view! {
        <section class="sheet__panel alarm-section">
            <h2 class="sheet__heading">{move || t(AlarmText::SirensTitle)}</h2>
            <p class="settings__hint">{move || t(AlarmText::SirensHint)}</p>
            <Show when=move || count.get() == 0>
                <p class="settings__empty">{move || t(AlarmText::NoSirens)}</p>
            </Show>
            <ul class="alarm-rows">
                {move || {
                    (0..count.get())
                        .map(|i| {
                            let siren = draft.with_untracked(|d| d.sirens[i].clone());
                            let has_chirp = move || {
                                draft.with(|d| d.sirens.get(i).is_some_and(siren_has_chirp))
                            };
                            let test = view! {
                                <button
                                    type="button"
                                    class="btn btn--compact"
                                    prop:disabled=move || unsaved.get() || !has_chirp()
                                    on:click=move |_| {
                                        spawn_local(async move {
                                            let text = match api::test_chirp(i).await {
                                                Ok(()) => t(AlarmText::TestChirpSent).to_owned(),
                                                Err(e) => e.error,
                                            };
                                            if let Some(toasts) = toasts {
                                                toasts.push(text);
                                            }
                                        });
                                    }
                                >
                                    {move || t(AlarmText::TestChirp)}
                                </button>
                            };
                            let remove = view! {
                                <button
                                    type="button"
                                    class="btn btn--compact"
                                    on:click=move |_| {
                                        draft
                                            .update(|d| {
                                                if i < d.sirens.len() {
                                                    d.sirens.remove(i);
                                                }
                                            });
                                    }
                                >
                                    {move || t(AlarmText::Remove)}
                                </button>
                            };
                            match siren {
                                Siren::Entity { entity, .. } => {
                                    view! {
                                        <li class="alarm-row">
                                            <div class="alarm-row__head">
                                                <EntityName house=house id=entity />
                                                {remove}
                                            </div>
                                            <div class="alarm-row__body">
                                                <label class="climate-field">
                                                    <span class="climate-field__label">
                                                        {move || t(AlarmText::ChirpMs)}
                                                    </span>
                                                    <input
                                                        class="field"
                                                        type="number"
                                                        inputmode="numeric"
                                                        min="0"
                                                        prop:value=move || {
                                                            draft
                                                                .with(|d| match d.sirens.get(i) {
                                                                    Some(Siren::Entity { chirp_ms: Some(ms), .. }) => {
                                                                        ms.to_string()
                                                                    }
                                                                    _ => String::new(),
                                                                })
                                                        }
                                                        on:change=move |ev| {
                                                            let text = event_target_value(&ev);
                                                            let text = text.trim();
                                                            let value = if text.is_empty() {
                                                                Some(None)
                                                            } else {
                                                                text.parse::<u32>().ok().map(Some)
                                                            };
                                                            if let Some(value) = value {
                                                                draft
                                                                    .update(|d| {
                                                                        if let Some(Siren::Entity { chirp_ms, .. }) = d
                                                                            .sirens
                                                                            .get_mut(i)
                                                                        {
                                                                            *chirp_ms = value;
                                                                        }
                                                                    });
                                                            }
                                                        }
                                                    />
                                                </label>
                                                {test}
                                            </div>
                                        </li>
                                    }
                                        .into_any()
                                }
                                Siren::Mqtt { .. } => {
                                    view! {
                                        <li class="alarm-row">
                                            <div class="alarm-row__head">
                                                <span class="alarm-row__name">
                                                    {move || t(AlarmText::MqttSiren)}
                                                </span>
                                                {remove}
                                            </div>
                                            <div class="alarm-row__body alarm-row__body--form">
                                                <MqttField
                                                    draft=draft
                                                    i=i
                                                    label=AlarmText::Topic
                                                    get=|m| m.topic.clone()
                                                    set=|m, v| m.topic = v
                                                />
                                                <MqttField
                                                    draft=draft
                                                    i=i
                                                    label=AlarmText::PayloadOn
                                                    get=|m| m.on.clone()
                                                    set=|m, v| m.on = v
                                                />
                                                <MqttField
                                                    draft=draft
                                                    i=i
                                                    label=AlarmText::PayloadOff
                                                    get=|m| m.off.clone()
                                                    set=|m, v| m.off = v
                                                />
                                                <MqttField
                                                    draft=draft
                                                    i=i
                                                    label=AlarmText::PayloadChirp
                                                    get=|m| m.chirp.clone().unwrap_or_default()
                                                    set=|m, v| m.chirp = (!v.is_empty()).then_some(v)
                                                />
                                                {test}
                                            </div>
                                        </li>
                                    }
                                        .into_any()
                                }
                            }
                        })
                        .collect_view()
                }}
            </ul>
            <Show when=move || unsaved.get() && (count.get() > 0)>
                <p class="settings__hint">{move || t(AlarmText::TestChirpUnsaved)}</p>
            </Show>
            <div class="alarm-section__actions">
                <EntityAddButton
                    label=Signal::derive(move || t(AlarmText::AddSiren).to_owned())
                    taken=taken
                    on_toggle=Callback::new(move |id: String| {
                        draft
                            .update(|d| {
                                let at = d
                                    .sirens
                                    .iter()
                                    .position(|s| matches!(s, Siren::Entity { entity, .. } if *entity == id));
                                match at {
                                    Some(at) => {
                                        d.sirens.remove(at);
                                    }
                                    None => d.sirens.push(Siren::Entity { entity: id, chirp_ms: None }),
                                }
                            });
                    })
                    pool=Some(keep(is_siren))
                />
                <button
                    type="button"
                    class="btn btn--compact"
                    on:click=move |_| {
                        draft
                            .update(|d| {
                                d.sirens
                                    .push(Siren::Mqtt {
                                        mqtt: MqttSiren {
                                            topic: String::new(),
                                            on: String::new(),
                                            off: String::new(),
                                            chirp: None,
                                        },
                                    });
                            });
                    }
                >
                    {move || t(AlarmText::AddMqttSiren)}
                </button>
            </div>
        </section>
    }
}

#[component]
fn RemotesSection(draft: Draft, house: House) -> impl IntoView {
    let lang = use_lang();
    let t = move |k: AlarmText| alarm_text(lang(), k);
    let taken = Signal::derive(move || {
        draft.with(|d| {
            d.remotes
                .iter()
                .map(|r| r.entity.clone())
                .collect::<Vec<_>>()
        })
    });
    let any_remote = Memo::new(move |_| house.entities.with(|m| m.values().any(is_remote)));
    let count = Memo::new(move |_| draft.with(|d| d.remotes.len()));
    let announced = move |id: &str| -> Vec<String> {
        house.entities.with(|m| {
            m.get(id)
                .and_then(|e| e.capabilities.generic.as_ref())
                .map(|g| g.signals.clone())
                .unwrap_or_default()
        })
    };
    view! {
        <section class="sheet__panel alarm-section">
            <h2 class="sheet__heading">{move || t(AlarmText::RemotesTitle)}</h2>
            <Show when=move || !any_remote.get()>
                <p class="settings__hint">{move || t(AlarmText::RemotesEmpty)}</p>
            </Show>
            <ul class="alarm-rows">
                {move || {
                    (0..count.get())
                        .map(|i| {
                            let entity = draft.with_untracked(|d| d.remotes[i].entity.clone());
                            let id = entity.clone();
                            let signal_count = Memo::new(move |_| {
                                draft.with(|d| d.remotes.get(i).map_or(0, |r| r.signals.len()))
                            });
                            let rows = move || {
                                (0..signal_count.get())
                                    .map(|j| {
                                        let name = move || {
                                            draft
                                                .with(|d| {
                                                    d.remotes
                                                        .get(i)
                                                        .and_then(|r| r.signals.keys().nth(j).cloned())
                                                        .unwrap_or_default()
                                                })
                                        };
                                        let action = move || {
                                            draft
                                                .with(|d| {
                                                    d.remotes
                                                        .get(i)
                                                        .and_then(|r| r.signals.values().nth(j).copied())
                                                })
                                        };
                                        view! {
                                            <div class="alarm-signal">
                                                <input
                                                    class="field"
                                                    type="text"
                                                    autocomplete="off"
                                                    spellcheck="false"
                                                    aria-label=move || t(AlarmText::Signal)
                                                    prop:value=name
                                                    on:change=move |ev| {
                                                        let new = event_target_value(&ev).trim().to_owned();
                                                        draft
                                                            .update(|d| {
                                                                let Some(r) = d.remotes.get_mut(i) else {
                                                                    return;
                                                                };
                                                                let Some(old) = r.signals.keys().nth(j).cloned() else {
                                                                    return;
                                                                };
                                                                if new.is_empty() || new == old
                                                                    || r.signals.contains_key(&new)
                                                                {
                                                                    return;
                                                                }
                                                                if let Some(a) = r.signals.remove(&old) {
                                                                    r.signals.insert(new, a);
                                                                }
                                                            });
                                                    }
                                                />
                                                <select
                                                    class="field"
                                                    aria-label=name
                                                    on:change=move |ev| {
                                                        let Some(a) = action_from_key(&event_target_value(&ev)) else {
                                                            return;
                                                        };
                                                        draft
                                                            .update(|d| {
                                                                if let Some(r) = d.remotes.get_mut(i)
                                                                    && let Some(key) = r.signals.keys().nth(j).cloned()
                                                                {
                                                                    r.signals.insert(key, a);
                                                                }
                                                            });
                                                    }
                                                >
                                                    {ACTIONS
                                                        .into_iter()
                                                        .map(|(a, key)| {
                                                            view! {
                                                                <option value=key selected=move || action() == Some(a)>
                                                                    {move || t(action_text(a))}
                                                                </option>
                                                            }
                                                        })
                                                        .collect_view()}
                                                </select>
                                                <button
                                                    type="button"
                                                    class="btn btn--compact"
                                                    on:click=move |_| {
                                                        draft
                                                            .update(|d| {
                                                                if let Some(r) = d.remotes.get_mut(i)
                                                                    && let Some(key) = r.signals.keys().nth(j).cloned()
                                                                {
                                                                    r.signals.remove(&key);
                                                                }
                                                            });
                                                    }
                                                >
                                                    {move || t(AlarmText::Remove)}
                                                </button>
                                            </div>
                                        }
                                    })
                                    .collect_view()
                            };
                            view! {
                                <li class="alarm-row">
                                    <div class="alarm-row__head">
                                        <EntityName house=house id=entity />
                                        <button
                                            type="button"
                                            class="btn btn--compact"
                                            on:click=move |_| {
                                                draft
                                                    .update(|d| {
                                                        if i < d.remotes.len() {
                                                            d.remotes.remove(i);
                                                        }
                                                    });
                                            }
                                        >
                                            {move || t(AlarmText::Remove)}
                                        </button>
                                    </div>
                                    <div class="alarm-row__body alarm-row__body--form">
                                        {rows}
                                        <button
                                            type="button"
                                            class="btn btn--compact alarm-row__add"
                                            on:click=move |_| {
                                                let names = announced(&id);
                                                draft
                                                    .update(|d| {
                                                        if let Some(r) = d.remotes.get_mut(i) {
                                                            let name = next_signal(&names, &r.signals);
                                                            r.signals.insert(name, RemoteAction::Away);
                                                        }
                                                    });
                                            }
                                        >
                                            {move || t(AlarmText::AddSignal)}
                                        </button>
                                    </div>
                                </li>
                            }
                        })
                        .collect_view()
                }}
            </ul>
            <EntityAddButton
                label=Signal::derive(move || t(AlarmText::AddRemote).to_owned())
                taken=taken
                on_toggle=Callback::new(move |id: String| {
                    draft
                        .update(|d| {
                            if d.remotes.iter().any(|r| r.entity == id) {
                                d.remotes.retain(|r| r.entity != id);
                            } else {
                                d.remotes.push(Remote { entity: id, signals: default_signals() });
                            }
                        });
                })
                pool=Some(keep(is_remote))
            />
        </section>
    }
}

#[component]
fn PresenceSection(draft: Draft, house: House) -> impl IntoView {
    let lang = use_lang();
    let t = move |k: AlarmText| alarm_text(lang(), k);
    let taken = Signal::derive(move || draft.with(|d| d.presence.trackers.clone()));
    view! {
        <section class="sheet__panel alarm-section">
            <h2 class="sheet__heading">{move || t(AlarmText::PresenceTitle)}</h2>
            <div class="climate-field">
                <span class="climate-field__label">{move || t(AlarmText::Trackers)}</span>
                <div class="chips climate-chips">
                    {move || {
                        draft
                            .with(|d| d.presence.trackers.clone())
                            .into_iter()
                            .map(|id| {
                                let text = label_of(house, lang(), &id);
                                view! {
                                    <button
                                        type="button"
                                        class="chip"
                                        title=move || t(AlarmText::Remove)
                                        on:click=move |_| {
                                            draft.update(|d| d.presence.trackers.retain(|i| i != &id));
                                        }
                                    >
                                        {text}
                                        " ×"
                                    </button>
                                }
                            })
                            .collect_view()
                    }}
                </div>
                <EntityAddButton
                    label=Signal::derive(move || t(AlarmText::Trackers).to_owned())
                    taken=taken
                    on_toggle=Callback::new(move |id: String| {
                        draft
                            .update(|d| {
                                if d.presence.trackers.contains(&id) {
                                    d.presence.trackers.retain(|i| i != &id);
                                } else {
                                    d.presence.trackers.push(id);
                                }
                            });
                    })
                    pool=Some(keep(is_tracker))
                />
            </div>
            <SwitchRow
                label=Signal::derive(move || t(AlarmText::PresenceArm).to_owned())
                on=Signal::derive(move || draft.with(|d| d.presence.arm))
                on_toggle=Callback::new(move |v| draft.update(|d| d.presence.arm = v))
            />
            <SwitchRow
                label=Signal::derive(move || t(AlarmText::PresenceDisarm).to_owned())
                on=Signal::derive(move || draft.with(|d| d.presence.disarm))
                on_toggle=Callback::new(move |v| draft.update(|d| d.presence.disarm = v))
            />
            <NumField
                draft=draft
                label=AlarmText::ArmAfter
                get=|d| d.presence.arm_after_s
                set=|d, v| d.presence.arm_after_s = v
            />
        </section>
    }
}

#[component]
fn TimingSection(draft: Draft) -> impl IntoView {
    let lang = use_lang();
    view! {
        <section class="sheet__panel alarm-section">
            <h2 class="sheet__heading">{move || alarm_text(lang(), AlarmText::TimingTitle)}</h2>
            <div class="alarm-timing">
                <NumField
                    draft=draft
                    label=AlarmText::EntryDelay
                    get=|d| d.entry_delay_s
                    set=|d, v| d.entry_delay_s = v
                />
                <NumField
                    draft=draft
                    label=AlarmText::ExitDelay
                    get=|d| d.exit_delay_s
                    set=|d, v| d.exit_delay_s = v
                />
                <NumField
                    draft=draft
                    label=AlarmText::SirenDuration
                    get=|d| d.siren_s
                    set=|d, v| d.siren_s = v
                />
                <NumField
                    draft=draft
                    label=AlarmText::MotionCount
                    get=|d| u32::from(d.motion.count)
                    set=|d, v| d.motion.count = u8::try_from(v).unwrap_or(u8::MAX)
                />
                <NumField
                    draft=draft
                    label=AlarmText::MotionWindow
                    get=|d| d.motion.window_s
                    set=|d, v| d.motion.window_s = v
                />
            </div>
        </section>
    }
}

/// The Settings → Alarm page.
#[component]
pub fn AlarmSettingsPage() -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let t = move |k: AlarmText| alarm_text(lang(), k);

    let view_sig = RwSignal::new(None::<AlarmView>);
    let failed = RwSignal::new(None::<String>);
    let draft: Draft = RwSignal::new(AlarmSettings::default());
    // The copy the edits started from, to tell edited fields from untouched ones on Save.
    let base = RwSignal::new(AlarmSettings::default());
    let error = RwSignal::new(None::<String>);
    let saving = RwSignal::new(false);

    let reload = move || {
        spawn_local(async move {
            match api::get_alarm().await {
                Ok(v) => {
                    failed.try_set(None);
                    view_sig.try_set(Some(v));
                }
                Err(e) => {
                    failed.try_set(Some(e.error));
                }
            }
        });
    };
    // Refetch on mount and whenever the hub says the alarm changed.
    Effect::new(move |_| {
        house.alarm_rev.track();
        reload();
    });
    // A change from elsewhere (another tab, the MCP, our own save) refills the form, unless
    // there are edits in progress.
    Effect::new(move |_| {
        let Some(fresh) = view_sig.with(|v| v.as_ref().map(|v| v.settings.clone())) else {
            return;
        };
        if draft.get_untracked() == base.get_untracked() {
            draft.set(fresh.clone());
            base.set(fresh);
        }
    });

    let saved_sirens = Signal::derive(move || {
        view_sig.with(|v| {
            v.as_ref()
                .map(|v| v.settings.sirens.clone())
                .unwrap_or_default()
        })
    });
    let toasts = use_context::<Toasts>();
    let save = move |_| {
        let Some(latest) = view_sig.with_untracked(|v| v.as_ref().map(|v| v.settings.clone()))
        else {
            return;
        };
        let merged = merge(&base.get_untracked(), &draft.get_untracked(), &latest);
        error.set(None);
        saving.set(true);
        spawn_local(async move {
            match api::put_alarm_settings(&merged).await {
                Ok(()) => {
                    draft.try_set(merged.clone());
                    base.try_set(merged);
                    if let Some(toasts) = toasts {
                        toasts.push(alarm_text(lang(), AlarmText::Saved));
                    }
                    reload();
                }
                Err(e) => {
                    error.try_set(Some(e.error));
                }
            }
            saving.try_set(false);
        });
    };

    view! {
        <div class="alarm-settings">
            {move || failed.get().map(|e| view! { <p class="home-settings__error">{e}</p> })}
            <Show
                when=move || view_sig.with(Option::is_some)
                fallback=move || view! { <p class="settings__hint">{t(AlarmText::Loading)}</p> }
            >
                {move || {
                    view_sig
                        .with(|v| v.as_ref().and_then(|v| v.settings_invalid.clone()))
                        .map(|reason| {
                            view! {
                                <p class="home-settings__error">
                                    {format!("{}: {reason}", t(AlarmText::LastRefused))}
                                </p>
                            }
                        })
                }}
                {move || {
                    let warnings = view_sig
                        .with(|v| v.as_ref().map(|v| v.warnings.clone()).unwrap_or_default());
                    (!warnings.is_empty())
                        .then(|| {
                            view! {
                                <p class="settings__hint">
                                    {format!("{}: {}", t(AlarmText::Warnings), warnings.join(", "))}
                                </p>
                            }
                        })
                }}
                <section class="sheet__panel alarm-section">
                    <SwitchRow
                        label=Signal::derive(move || t(AlarmText::EnableLabel).to_owned())
                        on=Signal::derive(move || draft.with(|d| d.enabled))
                        on_toggle=Callback::new(move |v| draft.update(|d| d.enabled = v))
                    />
                    <p class="settings__hint">{move || t(AlarmText::EnableHint)}</p>
                </section>
                <SensorsSection draft=draft house=house />
                <SirensSection draft=draft house=house saved=saved_sirens />
                <RemotesSection draft=draft house=house />
                <PresenceSection draft=draft house=house />
                <TimingSection draft=draft />
                <div class="alarm-settings__save">
                    {move || error.get().map(|e| view! { <p class="home-settings__error">{e}</p> })}
                    <button
                        type="button"
                        class="chip home-settings__save"
                        prop:disabled=move || saving.get()
                        on:click=save
                    >
                        {move || t(AlarmText::Save)}
                    </button>
                </div>
            </Show>
        </div>
    }
}

/// The device sheet's read-only "In alarm: …" line, linking to the alarm page; nothing for an
/// entity the alarm does not use.
#[component]
pub fn AlarmRoleLine(id: String) -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let settings = RwSignal::new(None::<AlarmSettings>);
    Effect::new(move |_| {
        house.alarm_rev.track();
        spawn_local(async move {
            if let Ok(v) = api::get_alarm().await {
                settings.try_set(Some(v.settings));
            }
        });
    });
    view! {
        {move || {
            settings
                .with(|s| s.as_ref().and_then(|s| alarm_line(s, &id, lang())))
                .map(|line| {
                    view! {
                        <p class="override__alarm">
                            <A href="/settings/alarm">{line}</A>
                        </p>
                    }
                })
        }}
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use nidoo_core::model::EntityCategory;
    use nidoo_core::model::{Capabilities, EntityState, UiOverride};

    use super::*;

    fn entity(id: &str, kind: Kind, class: Option<&str>) -> EntityView {
        EntityView {
            id: id.into(),
            device_id: None,
            name: id.into(),
            device_name: None,
            label: None,
            kind,
            component: kind.component().to_string(),
            protocol_kind: kind,
            device_class: class.map(Into::into),
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: "salon".into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride::default(),
            state: EntityState::default(),
        }
    }

    #[test]
    fn sensor_candidates_classifies_by_device_class() {
        let entities = vec![
            entity("a_door", Kind::BinarySensor, Some("door")),
            entity("a_window", Kind::BinarySensor, Some("window")),
            entity("a_opening", Kind::BinarySensor, Some("opening")),
            entity("a_garage", Kind::BinarySensor, Some("garage_door")),
            entity("a_motion", Kind::BinarySensor, Some("motion")),
            entity("a_occupancy", Kind::BinarySensor, Some("occupancy")),
            entity("a_presence", Kind::BinarySensor, Some("presence")),
            entity("a_smoke", Kind::BinarySensor, Some("smoke")),
            entity("a_plain", Kind::BinarySensor, None),
            entity("a_switch", Kind::Switch, Some("door")),
        ];
        let got = sensor_candidates(&entities, &[]);
        let kinds: Vec<(&str, SensorKind)> = got.iter().map(|(i, k)| (i.as_str(), *k)).collect();
        assert_eq!(
            kinds,
            [
                ("a_door", SensorKind::Opening),
                ("a_window", SensorKind::Opening),
                ("a_opening", SensorKind::Opening),
                ("a_garage", SensorKind::Opening),
                ("a_motion", SensorKind::Motion),
                ("a_occupancy", SensorKind::Motion),
                ("a_presence", SensorKind::Motion),
            ]
        );
        let listed = vec![default_sensor("a_door", SensorKind::Opening)];
        let rest = sensor_candidates(&entities, &listed);
        assert!(rest.iter().all(|(id, _)| id != "a_door"));
        assert_eq!(rest.len(), 6);
    }

    fn settings_with_door() -> AlarmSettings {
        AlarmSettings {
            sensors: vec![SensorRule {
                entity: "front_door".into(),
                kind: SensorKind::Opening,
                modes: BTreeSet::from([ArmMode::Away, ArmMode::Home, ArmMode::Night]),
                entry: Entry::Delayed,
            }],
            ..AlarmSettings::default()
        }
    }

    #[test]
    fn alarm_line_en_fr() {
        let s = settings_with_door();
        assert_eq!(
            alarm_line(&s, "front_door", Lang::En).as_deref(),
            Some("In alarm: delayed opening · away, home, night")
        );
        assert_eq!(
            alarm_line(&s, "front_door", Lang::Fr).as_deref(),
            Some("Dans l'alarme : ouverture temporisée · absent, présent, nuit")
        );
        assert_eq!(alarm_line(&s, "other", Lang::En), None);

        let mut s = settings_with_door();
        s.sensors[0].entry = Entry::Instant;
        assert_eq!(
            alarm_line(&s, "front_door", Lang::Fr).as_deref(),
            Some("Dans l'alarme : ouverture immédiate · absent, présent, nuit")
        );
        s.sensors.push(default_sensor("hall", SensorKind::Motion));
        assert_eq!(
            alarm_line(&s, "hall", Lang::En).as_deref(),
            Some("In alarm: motion · away")
        );
        s.sirens.push(Siren::Entity {
            entity: "plug".into(),
            chirp_ms: None,
        });
        s.remotes.push(Remote {
            entity: "fob".into(),
            signals: BTreeMap::new(),
        });
        s.presence.trackers.push("phone".into());
        assert_eq!(
            alarm_line(&s, "plug", Lang::En).as_deref(),
            Some("In alarm: siren")
        );
        assert_eq!(
            alarm_line(&s, "fob", Lang::Fr).as_deref(),
            Some("Dans l'alarme : télécommande")
        );
        assert_eq!(
            alarm_line(&s, "phone", Lang::En).as_deref(),
            Some("In alarm: presence tracker")
        );
    }

    #[test]
    fn countdown_formats_minutes_seconds() {
        assert_eq!(countdown_text(127, 100), "0:27");
        assert_eq!(countdown_text(175, 100), "1:15");
        assert_eq!(countdown_text(100, 160), "0:00");
        assert_eq!(countdown_text(100, 100), "0:00");
        assert_eq!(countdown_text(100 + 605, 100), "10:05");
    }

    fn view(enabled: bool, state: &str) -> AlarmView {
        AlarmView {
            enabled,
            entity_id: "nidoo_alarm_house".into(),
            state: state.into(),
            mode: None,
            source: None,
            since: 0,
            deadline: None,
            cause: vec![],
            open_at_arm: vec![],
            settings: AlarmSettings::default(),
            settings_invalid: None,
            warnings: vec![],
        }
    }

    #[test]
    fn settings_status_off_when_disabled() {
        assert_eq!(settings_status(&view(false, "disarmed"), Lang::En), "Off");
        assert_eq!(
            settings_status(&view(false, "armed_away"), Lang::Fr),
            "Désactivée"
        );
        assert_eq!(
            settings_status(&view(true, "disarmed"), Lang::En),
            "Disarmed"
        );
        assert_eq!(
            settings_status(&view(true, "armed_away"), Lang::En),
            "Armed · away"
        );
        assert_eq!(
            settings_status(&view(true, "armed_night"), Lang::Fr),
            "Armée · nuit"
        );
        assert_eq!(
            settings_status(&view(true, "triggered"), Lang::Fr),
            "Déclenchée"
        );
    }

    #[test]
    fn merge_takes_edits_and_the_latest_elsewhere() {
        let base = AlarmSettings::default();
        let mut draft = base.clone();
        draft.exit_delay_s = 90;
        let mut latest = base.clone();
        latest.entry_delay_s = 45;
        latest.exit_delay_s = 20;
        let got = merge(&base, &draft, &latest);
        assert_eq!(got.exit_delay_s, 90, "the edit wins");
        assert_eq!(
            got.entry_delay_s, 45,
            "the untouched field follows the latest"
        );
        assert_eq!(merge(&base, &base, &latest), latest);
    }

    #[test]
    fn next_signal_prefers_announced_names() {
        let mut taken = default_signals();
        let announced = vec!["disarm".to_owned(), "tamper".to_owned()];
        assert_eq!(next_signal(&announced, &taken), "tamper");
        taken.insert("tamper".into(), RemoteAction::Sos);
        assert_eq!(next_signal(&announced, &taken), "signal");
        taken.insert("signal".into(), RemoteAction::Sos);
        assert_eq!(next_signal(&[], &taken), "signal2");
    }

    #[test]
    fn chirp_is_testable_only_when_the_siren_has_one() {
        let mqtt = |chirp: Option<&str>| Siren::Mqtt {
            mqtt: MqttSiren {
                topic: "t".into(),
                on: "1".into(),
                off: "0".into(),
                chirp: chirp.map(Into::into),
            },
        };
        assert!(siren_has_chirp(&mqtt(Some("b"))));
        assert!(!siren_has_chirp(&mqtt(None)));
        assert!(!siren_has_chirp(&mqtt(Some(""))));
        assert!(siren_has_chirp(&Siren::Entity {
            entity: "e".into(),
            chirp_ms: Some(200)
        }));
        assert!(!siren_has_chirp(&Siren::Entity {
            entity: "e".into(),
            chirp_ms: None
        }));
    }
}
