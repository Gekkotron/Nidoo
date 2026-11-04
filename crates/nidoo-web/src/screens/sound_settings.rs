//! The Settings → Sound page: the speaker satellites the sound service publishes, one row each,
//! with the switch that approves a new one, its mute and its volume.

use std::collections::BTreeMap;

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use nidoo_api::{AreaView, EntityView, MAX_SPEECH_CHARS, SoundSpeakRequest};
use nidoo_core::model::{
    Availability, Command, EntityState, SOUND_DEVICE_PREFIX, StateValue, UNASSIGNED_AREA,
};

use crate::cards::Sender;
use crate::components::card_frame::sheet_path;
use crate::format::{area_name, room_name};
use crate::house::House;
use crate::i18n::{Lang, SoundText, said_on, sound_text, use_lang, use_sound_text};
use crate::screens::alarm_settings::SwitchRow;

/// One speaker satellite, gathered from its entities.
#[derive(Clone, Debug, PartialEq)]
pub struct Satellite {
    pub device_id: String,
    pub name: String,
    /// The room id of the satellite's entities.
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

    /// The entity whose page the satellite's name opens.
    fn page_entity(&self) -> Option<&EntityView> {
        self.allowed.as_ref().or(self.volume.as_ref())
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

/// A guess of the state after `value`, for the optimistic write.
fn guess(entity: &EntityView, value: StateValue) -> EntityState {
    EntityState {
        value,
        ..entity.state.clone()
    }
}

/// One satellite: its name, room, connection dot, approval badge and controls.
#[component]
fn SatelliteRow(satellite: Satellite) -> impl IntoView {
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
    let room = house.areas.with(|areas| {
        room_name(areas, &satellite.area).map(|name| {
            let id = satellite.area.clone();
            area_name(lang(), &id, &name)
        })
    });
    let waiting = satellite.allowed.is_some() && !satellite.is_allowed();

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
            </div>
            {room.map(|r| view! { <p class="settings__hint">{r}</p> })}
            <div class="alarm-row__body">{allowed} {mute} {volume}</div>
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

/// The Settings → Sound page.
#[component]
pub fn SoundSettingsPage() -> impl IntoView {
    let house = expect_context::<House>();
    let t = use_sound_text();
    let list = Memo::new(move |_| house.entities.with(satellites));
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
                                .map(|s| view! { <SatelliteRow satellite=s /> })
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
