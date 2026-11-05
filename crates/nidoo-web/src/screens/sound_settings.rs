//! The Settings → Assistant → Sound page: the speaker satellites the sound service publishes, one row each,
//! with the switch that approves a new one, its mute, its volume, its room, a button that removes
//! it (after an inline "are you sure?") and, once allowed, a button that says a test sentence on
//! that satellite alone.

use std::collections::BTreeMap;

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use nidoo_api::{AreaView, EntityView, MAX_SPEECH_CHARS, OverridePatch, SoundSpeakRequest};
use nidoo_core::model::{
    Availability, Command, EntityState, SOUND_DEVICE_PREFIX, StateValue, UNASSIGNED_AREA,
};

use crate::cards::Sender;
use crate::components::card_frame::sheet_path;
use crate::components::confirm::{ConfirmBar, Confirming};
use crate::format::area_name;
use crate::house::House;
use crate::i18n::{Lang, SoundText, said_on, sound_text, speaker_test, use_lang, use_sound_text};
use crate::icons::{SPEECH_BUBBLE, TRASH};
use crate::screens::alarm_settings::SwitchRow;

/// One speaker satellite, gathered from its entities.
#[derive(Clone, Debug, PartialEq)]
pub struct Satellite {
    pub device_id: String,
    pub name: String,
    /// The room id of the satellite: its Volume entity's (the hub's rule), else any entity's.
    pub area: String,
    pub allowed: Option<EntityView>,
    pub mute: Option<EntityView>,
    pub volume: Option<EntityView>,
    pub connected: Option<EntityView>,
}

impl Satellite {
    fn entities(&self) -> impl Iterator<Item = &EntityView> {
        [&self.allowed, &self.mute, &self.volume, &self.connected]
            .into_iter()
            .flatten()
    }

    /// True while the sound service is publishing: every entity of the satellite is online.
    pub fn service_online(&self) -> bool {
        self.entities()
            .all(|e| e.state.availability == Availability::Online)
    }

    /// True once the owner has switched Allowed on.
    pub fn is_allowed(&self) -> bool {
        self.allowed
            .as_ref()
            .is_some_and(|e| e.state.value == StateValue::OnOff(true))
    }

    /// True when the player is connected and the service is up.
    pub fn is_connected(&self) -> bool {
        self.service_online()
            && self
                .connected
                .as_ref()
                .is_some_and(|e| e.state.value == StateValue::OnOff(true))
    }

    /// The short id the hub takes to speak on this satellite alone: the device id without
    /// `nidoo_sound_`.
    pub fn short_id(&self) -> &str {
        self.device_id
            .strip_prefix(SOUND_DEVICE_PREFIX)
            .unwrap_or(&self.device_id)
    }

    /// The entity whose page the satellite's name opens: its Volume, the card the others fold
    /// into, whose room the room lists and the hub read.
    fn page_entity(&self) -> Option<&EntityView> {
        self.volume.as_ref().or(self.allowed.as_ref())
    }
}

/// The satellites among `entities`, sorted by name.
pub fn satellites(entities: &BTreeMap<String, EntityView>) -> Vec<Satellite> {
    gather(entities.values())
}

/// The satellite of device `device_id`, when `entities` holds any of its entities.
pub fn satellite_of(entities: &BTreeMap<String, EntityView>, device_id: &str) -> Option<Satellite> {
    gather(
        entities
            .values()
            .filter(|e| e.device_id.as_deref() == Some(device_id)),
    )
    .pop()
}

fn gather<'a>(entities: impl Iterator<Item = &'a EntityView>) -> Vec<Satellite> {
    let mut found: BTreeMap<String, Satellite> = BTreeMap::new();
    for e in entities {
        let Some(device_id) = e
            .device_id
            .as_deref()
            .filter(|d| d.starts_with(SOUND_DEVICE_PREFIX))
        else {
            continue;
        };
        let sat = found
            .entry(device_id.to_owned())
            .or_insert_with(|| Satellite {
                device_id: device_id.to_owned(),
                name: e
                    .device_name
                    .clone()
                    .unwrap_or_else(|| device_id.to_owned()),
                area: e.area.clone(),
                allowed: None,
                mute: None,
                volume: None,
                connected: None,
            });
        let slot = [
            ("_allowed", &mut sat.allowed),
            ("_mute", &mut sat.mute),
            ("_volume", &mut sat.volume),
            ("_connected", &mut sat.connected),
        ]
        .into_iter()
        .find(|(suffix, _)| e.id.ends_with(suffix))
        .map(|(_, slot)| slot);
        if let Some(slot) = slot {
            *slot = Some(e.clone());
        }
        // The satellite's room is its Volume's, as the hub reads it to pick the speakers.
        if e.id.ends_with("_volume") {
            sat.area = e.area.clone();
        }
    }
    let mut list: Vec<Satellite> = found.into_values().collect();
    list.sort_by_cached_key(|s| (s.name.to_lowercase(), s.device_id.clone()));
    list
}

/// `(id, name)` of the rooms that hold at least one of `satellites`, in the order of `areas`;
/// the catch-all area is left out (the hub refuses it as a room to speak in).
pub fn satellite_rooms(satellites: &[Satellite], areas: &[AreaView]) -> Vec<(String, String)> {
    areas
        .iter()
        .filter(|a| a.id != UNASSIGNED_AREA && satellites.iter().any(|s| s.area == a.id))
        .map(|a| (a.id.clone(), a.name.clone()))
        .collect()
}

/// The Settings list's status for this page: how many satellites wait for approval.
pub fn status_label(satellites: &[Satellite], lang: Lang) -> String {
    let waiting = satellites
        .iter()
        .filter(|s| s.allowed.is_some() && !s.is_allowed())
        .count();
    if waiting == 0 {
        String::new()
    } else {
        format!("{waiting} {}", sound_text(lang, SoundText::Waiting))
    }
}

/// `(id, name)` of the rooms a satellite can be put in, in the order of `areas`; the catch-all
/// area is the picker's "No room".
pub fn room_choices(areas: &[AreaView]) -> Vec<(String, String)> {
    areas
        .iter()
        .filter(|a| a.id != UNASSIGNED_AREA)
        .map(|a| (a.id.clone(), a.name.clone()))
        .collect()
}

/// Every entity of device `device_id`, sorted: the room picker moves them all, as the device
/// sheet does.
pub fn device_entity_ids(entities: &BTreeMap<String, EntityView>, device_id: &str) -> Vec<String> {
    entities
        .values()
        .filter(|e| e.device_id.as_deref() == Some(device_id))
        .map(|e| e.id.clone())
        .collect()
}

/// The hub route that forgets `satellite`.
pub fn remove_path(satellite: &Satellite) -> String {
    format!("/api/v1/sound/satellites/{}", satellite.short_id())
}

/// The one-field patch that moves an entity to `area`; `None` clears the field.
fn area_patch(area: Option<&str>) -> OverridePatch {
    let mut patch = OverridePatch::new();
    patch.insert("area".to_owned(), serde_json::json!(area));
    patch
}

/// How long a failed action's reason stays on screen.
const TEST_ERROR_MS: u32 = 5_000;

/// Shows `reason` through `show` and clears it [`TEST_ERROR_MS`] later, unless a newer failure
/// replaced it (`shown` counts the failures shown).
async fn flash_error(shown: StoredValue<u32>, show: impl Fn(Option<String>), reason: String) {
    let Some(mine) = shown.try_update_value(|n| {
        *n = n.wrapping_add(1);
        *n
    }) else {
        return;
    };
    show(Some(reason));
    TimeoutFuture::new(TEST_ERROR_MS).await;
    if shown.try_get_value() == Some(mine) {
        show(None);
    }
}

/// The request the test button sends: the test sentence, on `satellite` alone.
pub fn test_request(satellite: &Satellite, lang: Lang) -> SoundSpeakRequest {
    SoundSpeakRequest {
        text: speaker_test(lang, &satellite.name),
        satellite: Some(satellite.short_id().to_owned()),
        ..Default::default()
    }
}

/// Sends `req`; a failure's reason goes to `show` and is cleared [`TEST_ERROR_MS`] later, unless
/// a newer failure replaced it (`shown` counts the failures shown). Both the dashboard card and
/// the Settings row use it.
pub fn say_test(
    req: SoundSpeakRequest,
    shown: StoredValue<u32>,
    show: impl Fn(Option<String>) + 'static,
) {
    spawn_local(async move {
        let Err(err) = crate::api::tts::sound_speak(&req).await else {
            return;
        };
        flash_error(shown, show, err.error).await;
    });
}

/// The last failed action on the page (a test, a removal, a room change): the satellite's
/// device id and the reason. Held by the page so it survives the rows being redrawn on a state
/// change.
#[derive(Clone, Copy)]
struct RowError {
    error: RwSignal<Option<(String, String)>>,
    shown: StoredValue<u32>,
}

impl RowError {
    /// Shows `reason` under `device`'s row for a while.
    fn flash(self, device: String, reason: String) {
        spawn_local(async move {
            flash_error(
                self.shown,
                move |r| {
                    self.error.try_set(r.map(|r| (device.clone(), r)));
                },
                reason,
            )
            .await;
        });
    }
}

/// A guess of the state after `value`, for the optimistic write.
fn guess(entity: &EntityView, value: StateValue) -> EntityState {
    EntityState {
        value,
        ..entity.state.clone()
    }
}

/// One satellite: its name, connection dot, approval badge, controls and room.
#[component]
fn SatelliteRow(
    satellite: Satellite,
    row_error: RowError,
    confirming: Confirming,
) -> impl IntoView {
    let house = expect_context::<House>();
    let sender = Sender::from_context();
    let lang = use_lang();
    let t = use_sound_text();

    let dot = if satellite.is_connected() {
        "dot dot--online"
    } else {
        "dot dot--offline"
    };
    let dot_label = if satellite.is_connected() {
        SoundText::Connected
    } else {
        SoundText::Disconnected
    };
    let waiting = satellite.allowed.is_some() && !satellite.is_allowed();

    let test = satellite.is_allowed().then(|| {
        let satellite = satellite.clone();
        let on_click = move |_| {
            let device = satellite.device_id.clone();
            say_test(
                test_request(&satellite, lang()),
                row_error.shown,
                move |reason| {
                    row_error.error.try_set(reason.map(|r| (device.clone(), r)));
                },
            );
        };
        view! {
            <button
                type="button"
                class="btn btn--icon satellite__speak"
                aria-label=move || t(SoundText::TestSpeaker)
                title=move || t(SoundText::TestSpeaker)
                inner_html=SPEECH_BUBBLE
                on:click=on_click
            ></button>
        }
    });
    let remove_button = {
        let device = satellite.device_id.clone();
        view! {
            <button
                type="button"
                class="btn btn--icon satellite__remove"
                aria-label=move || t(SoundText::Remove)
                title=move || t(SoundText::Remove)
                inner_html=TRASH
                on:click=move |_| confirming.ask(&device)
            ></button>
        }
    };
    let asking = {
        let satellite = satellite.clone();
        move || {
            confirming.is_asking(&satellite.device_id).then(|| {
                let device = satellite.device_id.clone();
                let path = remove_path(&satellite);
                let on_yes = Callback::new(move |()| {
                    confirming.clear();
                    let (device, path) = (device.clone(), path.clone());
                    spawn_local(async move {
                        // The entities go once the sound service has cleared them.
                        if let Err(err) = crate::api::tts::remove_satellite(&path).await {
                            row_error.flash(device, err.error);
                        }
                    });
                });
                view! {
                    <ConfirmBar
                        question=Signal::derive(move || t(SoundText::RemoveQuestion).to_owned())
                        yes=Signal::derive(move || t(SoundText::Remove).to_owned())
                        on_yes=on_yes
                        on_cancel=Callback::new(move |()| confirming.clear())
                    />
                }
            })
        }
    };
    let room_picker = {
        let device = satellite.device_id.clone();
        let current = satellite.area.clone();
        let on_change = move |ev: leptos::ev::Event| {
            let value = event_target_value(&ev);
            let ids = house
                .entities
                .with_untracked(|m| device_entity_ids(m, &device));
            let device = device.clone();
            spawn_local(async move {
                let area = (!value.is_empty()).then_some(value.as_str());
                let mut first_error = None;
                for id in ids {
                    match crate::api::set_override(&id, area_patch(area)).await {
                        // The reply is the entity as the hub now has it.
                        Ok(view) => {
                            house.entities.try_update(|m| {
                                m.insert(view.id.clone(), view);
                            });
                        }
                        Err(e) => {
                            first_error.get_or_insert(e.error);
                        }
                    }
                }
                if let Some(error) = first_error {
                    row_error.flash(device, error);
                }
            });
        };
        let choices = house.areas.with(|areas| room_choices(areas));
        let none_selected = !choices.iter().any(|(id, _)| *id == current);
        let options = choices
            .into_iter()
            .map(|(id, name)| {
                let selected = id == current;
                let label = area_name(lang(), &id, &name);
                view! { <option value=id selected=selected>{label}</option> }
            })
            .collect_view();
        view! {
            <label class="override__flag sound-room-field">
                <span class="override__label">{move || t(SoundText::Room)}</span>
                <select class="select sound-room" on:change=on_change>
                    <option value="" selected=none_selected>
                        {move || t(SoundText::NoRoom)}
                    </option>
                    {options}
                </select>
            </label>
        }
    };
    let failed = {
        let device = satellite.device_id.clone();
        move || {
            row_error.error.with(|e| {
                e.as_ref()
                    .filter(|(d, _)| *d == device)
                    .map(|(_, reason)| reason.clone())
            })
        }
    };

    let name = match satellite.page_entity() {
        Some(e) => {
            view! { <A href=sheet_path(&e.id) attr:class="alarm-row__name">{satellite.name.clone()}</A> }
                .into_any()
        }
        None => view! { <span class="alarm-row__name">{satellite.name.clone()}</span> }.into_any(),
    };

    let toggle = |entity: &Option<EntityView>, label: SoundText| {
        let entity = entity.clone()?;
        let on = matches!(entity.state.value, StateValue::OnOff(true));
        let on_toggle = {
            let entity = entity.clone();
            Callback::new(move |to: bool| {
                let cmd = if to {
                    Command::TurnOn {
                        brightness: None,
                        color: None,
                        effect: None,
                    }
                } else {
                    Command::TurnOff
                };
                sender.send(&entity.id, cmd, Some(guess(&entity, StateValue::OnOff(to))));
            })
        };
        Some(view! {
            <SwitchRow
                label=Signal::derive(move || t(label).to_owned())
                on=on
                on_toggle=on_toggle
                busy=sender.pending(&entity.id)
            />
        })
    };
    let allowed = toggle(&satellite.allowed, SoundText::Allowed);
    let mute = toggle(&satellite.mute, SoundText::Mute);

    let volume = satellite.volume.clone().map(|entity| {
        let caps = &entity.capabilities;
        let (min, max, step) = (
            caps.min.unwrap_or(0.0),
            caps.max.unwrap_or(100.0),
            caps.step.unwrap_or(1.0),
        );
        let now = match entity.state.value {
            StateValue::Number(n) => n,
            _ => min,
        };
        let on_change = {
            let entity = entity.clone();
            move |ev: leptos::ev::Event| {
                let Ok(value) = event_target_value(&ev).parse::<f64>() else {
                    return;
                };
                sender.send(
                    &entity.id,
                    Command::SetNumber { value },
                    Some(guess(&entity, StateValue::Number(value))),
                );
            }
        };
        view! {
            <label class="override__flag sound-volume">
                <span class="override__label">{move || t(SoundText::Volume)}</span>
                <input
                    class="slider"
                    type="range"
                    min=min.to_string()
                    max=max.to_string()
                    step=step.to_string()
                    prop:value=now.to_string()
                    on:change=on_change
                />
            </label>
        }
    });

    view! {
        <li class="alarm-row sound-row">
            <div class="alarm-row__head">
                <span class="settings__state">
                    <span class=dot role="img" aria-label=move || t(dot_label)></span>
                </span>
                {name}
                {waiting
                    .then(|| {
                        view! {
                            <span class="chip sound-waiting">
                                {move || t(SoundText::WaitingForApproval)}
                            </span>
                        }
                    })}
                {test}
                {remove_button}
            </div>
            {asking}
            <div class="alarm-row__body">{allowed} {mute} {volume} {room_picker}</div>
            {move || failed().map(|e| view! { <p class="satellite__error" role="alert">{e}</p> })}
        </li>
    }
}

/// The request the Say box sends: `room` empty means every room.
pub fn speak_request(text: &str, room: &str) -> SoundSpeakRequest {
    SoundSpeakRequest {
        text: text.trim().to_owned(),
        room: (!room.is_empty()).then(|| room.to_owned()),
        ..Default::default()
    }
}

/// "Say something": a sentence, the room (every room first) and Send. Send waits for a text
/// and is off while a sentence is on its way; the outcome reads under it.
#[component]
fn SayBox(satellites: Memo<Vec<Satellite>>) -> impl IntoView {
    let house = expect_context::<House>();
    let t = use_sound_text();
    let lang = use_lang();
    let text = RwSignal::new(String::new());
    let room = RwSignal::new(String::new());
    let sending = RwSignal::new(false);
    let outcome = RwSignal::new(None::<Result<usize, String>>);
    let rooms =
        move || satellites.with(|found| house.areas.with(|areas| satellite_rooms(found, areas)));
    let send = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if sending.get_untracked() || text.with_untracked(|t| t.trim().is_empty()) {
            return;
        }
        let req = speak_request(&text.get_untracked(), &room.get_untracked());
        sending.set(true);
        outcome.set(None);
        spawn_local(async move {
            let result = crate::api::tts::sound_speak(&req).await;
            sending.set(false);
            match result {
                Ok(reply) => {
                    text.set(String::new());
                    outcome.set(Some(Ok(reply.players)));
                }
                Err(err) => outcome.set(Some(Err(err.error))),
            }
        });
    };
    view! {
        <section class="sheet__panel alarm-section say-box">
            <h2 class="sheet__heading">{move || t(SoundText::SaySomething)}</h2>
            <form class="say-box__form" on:submit=send>
                <input
                    class="field say-box__text"
                    type="text"
                    maxlength=MAX_SPEECH_CHARS.to_string()
                    placeholder=move || t(SoundText::SayPlaceholder)
                    aria-label=move || t(SoundText::SaySomething)
                    prop:value=move || text.get()
                    on:input=move |ev| text.set(event_target_value(&ev))
                />
                <select
                    class="select say-box__room"
                    aria-label=move || t(SoundText::Room)
                    on:change=move |ev| room.set(event_target_value(&ev))
                >
                    <option value="" selected=move || room.get().is_empty()>
                        {move || t(SoundText::AllRooms)}
                    </option>
                    {move || {
                        rooms()
                            .into_iter()
                            .map(|(id, name)| {
                                let chosen = id.clone();
                                view! {
                                    <option value=id selected=move || room.get() == chosen>
                                        {name}
                                    </option>
                                }
                            })
                            .collect_view()
                    }}
                </select>
                <button
                    type="submit"
                    class="btn btn--primary say-box__send"
                    disabled=move || sending.get() || text.with(|t| t.trim().is_empty())
                >
                    {move || t(SoundText::Send)}
                </button>
            </form>
            {move || match outcome.get() {
                Some(Ok(n)) => {
                    view! { <p class="settings__hint say-box__done" role="status">{said_on(lang(), n)}</p> }
                        .into_any()
                }
                Some(Err(e)) => view! { <p class="say-box__error" role="alert">{e}</p> }.into_any(),
                None => ().into_any(),
            }}
        </section>
    }
}

/// The Settings → Assistant → Sound page.
#[component]
pub fn SoundSettingsPage() -> impl IntoView {
    let house = expect_context::<House>();
    let t = use_sound_text();
    let list = Memo::new(move |_| house.entities.with(satellites));
    let row_error = RowError {
        error: RwSignal::new(None),
        shown: StoredValue::new(0),
    };
    let confirming = Confirming::new();
    let service_offline = move || list.with(|l| l.iter().any(|s| !s.service_online()));
    view! {
        <div class="alarm-settings">
            <Show when=service_offline>
                <p class="settings__hint network-hint" role="status">
                    {move || t(SoundText::ServiceOffline)}
                </p>
            </Show>
            <Show when=move || list.with(|l| !l.is_empty())>
                <SayBox satellites=list />
            </Show>
            <section class="sheet__panel alarm-section">
                <Show
                    when=move || list.with(|l| !l.is_empty())
                    fallback=move || {
                        view! {
                            <p class="settings__empty">{move || t(SoundText::Empty)}</p>
                            <p class="settings__hint">{move || t(SoundText::EmptyHint)}</p>
                        }
                    }
                >
                    <ul class="alarm-rows">
                        {move || {
                            list.get()
                                .into_iter()
                                .map(|s| {
                                    view! {
                                        <SatelliteRow
                                            satellite=s
                                            row_error=row_error
                                            confirming=confirming
                                        />
                                    }
                                })
                                .collect_view()
                        }}
                    </ul>
                </Show>
            </section>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_core::model::{Capabilities, EntityCategory, Kind, UiOverride};

    fn entity(id: &str, device: Option<(&str, &str)>, value: StateValue) -> EntityView {
        EntityView {
            id: id.into(),
            device_id: device.map(|d| d.0.into()),
            name: id.into(),
            device_name: device.map(|d| d.1.into()),
            label: None,
            kind: Kind::Switch,
            component: "switch".into(),
            protocol_kind: Kind::Switch,
            device_class: None,
            unit: None,
            icon: None,
            category: EntityCategory::Config,
            enabled_by_default: true,
            area: "kitchen".into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride::default(),
            state: EntityState {
                value,
                availability: Availability::Online,
                ..EntityState::default()
            },
        }
    }

    fn map(list: Vec<EntityView>) -> BTreeMap<String, EntityView> {
        list.into_iter().map(|e| (e.id.clone(), e)).collect()
    }

    fn full(device: &str, name: &str, allowed: bool) -> Vec<EntityView> {
        let d = Some((device, name));
        vec![
            entity(&format!("{device}_volume"), d, StateValue::Number(30.0)),
            entity(&format!("{device}_mute"), d, StateValue::OnOff(false)),
            entity(&format!("{device}_allowed"), d, StateValue::OnOff(allowed)),
            entity(&format!("{device}_connected"), d, StateValue::OnOff(true)),
        ]
    }

    #[test]
    fn groups_the_four_entities_of_a_device_and_sorts_by_name() {
        let mut all = full("nidoo_sound_bbb", "kitchen", true);
        all.extend(full("nidoo_sound_aaa", "Office", false));
        all.push(entity(
            "lamp",
            Some(("zigbee_lamp", "Lamp")),
            StateValue::OnOff(true),
        ));
        all.push(entity("loose", None, StateValue::OnOff(true)));
        let found = satellites(&map(all));
        let names: Vec<_> = found.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["kitchen", "Office"]);
        let office = &found[1];
        assert!(office.allowed.is_some() && office.mute.is_some());
        assert!(office.volume.is_some() && office.connected.is_some());
        assert!(!office.is_allowed());
        assert!(found[0].is_allowed() && found[0].is_connected());
    }

    #[test]
    fn an_offline_service_means_not_connected() {
        let mut all = full("nidoo_sound_aaa", "Office", true);
        for e in &mut all {
            e.state.availability = Availability::Offline;
        }
        let found = satellites(&map(all));
        assert!(!found[0].service_online());
        assert!(!found[0].is_connected());
    }

    #[test]
    fn the_room_is_the_volume_entity_room() {
        // The card opens the Volume entity's sheet, and the hub reads the Volume's room: a room
        // changed there must win over the other entities' rooms.
        let mut all = full("nidoo_sound_aaa", "A", true);
        for e in &mut all {
            e.area = if e.id.ends_with("_volume") {
                "office"
            } else {
                "kitchen"
            }
            .into();
        }
        let found = satellites(&map(all));
        assert_eq!(found[0].area, "office");
    }

    #[test]
    fn satellite_of_picks_one_device() {
        let mut all = full("nidoo_sound_aaa", "A", true);
        all.extend(full("nidoo_sound_bbb", "B", false));
        let all = map(all);
        let b = satellite_of(&all, "nidoo_sound_bbb").expect("b");
        assert_eq!(b.name, "B");
        assert!(!b.is_allowed());
        assert!(b.volume.is_some() && b.mute.is_some() && b.connected.is_some());
        assert_eq!(satellite_of(&all, "nidoo_sound_zzz"), None);
    }

    #[test]
    fn satellite_rooms_lists_rooms_holding_one_in_area_order() {
        let mut all = full("nidoo_sound_aaa", "A", true);
        let mut office = full("nidoo_sound_bbb", "B", true);
        for e in &mut office {
            e.area = "office".into();
        }
        all.extend(office);
        let mut loose = full("nidoo_sound_ccc", "C", true);
        for e in &mut loose {
            e.area = UNASSIGNED_AREA.into();
        }
        all.extend(loose);
        let found = satellites(&map(all));
        let area = |id: &str, name: &str| AreaView {
            id: id.into(),
            name: name.into(),
            icon: None,
            order: None,
            entity_count: 0,
            primary_count: 0,
            group_names: Default::default(),
            group_favorites: Default::default(),
        };
        let areas = vec![
            area("office", "Office"),
            area("bath", "Bath"),
            area("kitchen", "Kitchen"),
            area(UNASSIGNED_AREA, "Unassigned"),
        ];
        assert_eq!(
            satellite_rooms(&found, &areas),
            [
                ("office".to_owned(), "Office".to_owned()),
                ("kitchen".to_owned(), "Kitchen".to_owned())
            ]
        );
    }

    #[test]
    fn speak_request_sends_null_for_every_room() {
        let all = serde_json::to_value(speak_request(" Hello ", "")).unwrap();
        assert_eq!(all["text"], "Hello");
        assert_eq!(all["room"], serde_json::Value::Null);
        let one = serde_json::to_value(speak_request("Hello", "kitchen")).unwrap();
        assert_eq!(one["room"], "kitchen");
        assert_eq!(one["satellite"], serde_json::Value::Null);
    }

    #[test]
    fn the_test_request_names_the_satellite_by_its_short_id() {
        let found = satellites(&map(full("nidoo_sound_aaa", "Desk", true)));
        assert_eq!(found[0].short_id(), "aaa");
        let req = test_request(&found[0], Lang::En);
        assert_eq!(req.satellite.as_deref(), Some("aaa"));
        assert_eq!(req.text, "Speaker test: Desk");
        assert_eq!(req.room, None);
    }

    #[test]
    fn the_name_opens_the_volume_page() {
        // The Volume is the satellite's card: the others fold into it, and its room is the
        // one the room lists and the hub read. A room set on another entity's page is lost.
        let found = satellites(&map(full("nidoo_sound_aaa", "Desk", true)));
        assert_eq!(
            found[0].page_entity().map(|e| e.id.as_str()),
            Some("nidoo_sound_aaa_volume")
        );
    }

    #[test]
    fn remove_path_names_the_satellite_by_its_short_id() {
        let found = satellites(&map(full("nidoo_sound_aaa", "Desk", true)));
        assert_eq!(remove_path(&found[0]), "/api/v1/sound/satellites/aaa");
    }

    #[test]
    fn the_room_moves_every_entity_of_the_satellite() {
        let mut all = full("nidoo_sound_aaa", "Desk", true);
        all.push(entity(
            "nidoo_sound_aaa_address",
            Some(("nidoo_sound_aaa", "Desk")),
            StateValue::Text("192.0.2.1".into()),
        ));
        all.extend(full("nidoo_sound_bbb", "Other", true));
        let ids = device_entity_ids(&map(all), "nidoo_sound_aaa");
        assert_eq!(
            ids,
            [
                "nidoo_sound_aaa_address",
                "nidoo_sound_aaa_allowed",
                "nidoo_sound_aaa_connected",
                "nidoo_sound_aaa_mute",
                "nidoo_sound_aaa_volume",
            ]
        );
    }

    #[test]
    fn room_choices_leave_out_the_catch_all() {
        let area = |id: &str, name: &str| AreaView {
            id: id.into(),
            name: name.into(),
            icon: None,
            order: None,
            entity_count: 0,
            primary_count: 0,
            group_names: Default::default(),
            group_favorites: Default::default(),
        };
        let areas = vec![
            area("office", "Office"),
            area(UNASSIGNED_AREA, "Unassigned"),
            area("kitchen", "Kitchen"),
        ];
        assert_eq!(
            room_choices(&areas),
            [
                ("office".to_owned(), "Office".to_owned()),
                ("kitchen".to_owned(), "Kitchen".to_owned())
            ]
        );
    }

    #[test]
    fn no_satellite_without_the_prefix() {
        assert!(satellites(&map(full("other_aaa", "X", true))).is_empty());
    }

    #[test]
    fn status_counts_the_satellites_waiting() {
        let mut all = full("nidoo_sound_aaa", "A", false);
        all.extend(full("nidoo_sound_bbb", "B", false));
        all.extend(full("nidoo_sound_ccc", "C", true));
        let found = satellites(&map(all));
        assert_eq!(status_label(&found, Lang::En), "2 waiting");
        assert_eq!(status_label(&found, Lang::Fr), "2 en attente");
        assert_eq!(status_label(&found[2..], Lang::En), "");
    }
}
