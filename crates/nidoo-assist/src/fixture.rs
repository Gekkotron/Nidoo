//! A fictional house for tests: generic room names, never a real home.

use std::collections::{BTreeMap, BTreeSet};

use nidoo_api::{AreaView, EntityView};
use nidoo_climate::doc::{Decision, Setpoint, Source, ZoneSettings, ZoneState};
use nidoo_climate::view::{LearnStatus, ZoneView};
use nidoo_core::model::{
    Availability, Capabilities, CoverState, EntityCategory, EntityState, Kind, LightState,
    MediaPlayerState, StateValue,
};

use crate::house::{HeatRange, House};

/// The fixed "now" of the fixture, Unix seconds.
pub const NOW: u64 = 1_800_000_000;

/// The owned parts of a [`House`].
#[derive(Clone, Debug)]
pub struct Fixture {
    pub areas: Vec<AreaView>,
    pub entities: Vec<EntityView>,
    pub zones: Vec<ZoneView>,
    pub outdoor_entity: Option<String>,
    pub stale_after_min: u32,
    pub range: HeatRange,
    pub now: u64,
}

impl Fixture {
    pub fn house(&self) -> House<'_> {
        House {
            areas: &self.areas,
            entities: &self.entities,
            zones: &self.zones,
            outdoor_entity: self.outdoor_entity.as_deref(),
            stale_after_min: self.stale_after_min,
            range: self.range,
            now: self.now,
        }
    }

    pub fn entity_mut(&mut self, id: &str) -> &mut EntityView {
        self.entities.iter_mut().find(|e| e.id == id).unwrap()
    }

    pub fn zone_mut(&mut self, id: &str) -> &mut ZoneView {
        self.zones.iter_mut().find(|z| z.zone == id).unwrap()
    }
}

pub fn area(id: &str, name: &str) -> AreaView {
    AreaView {
        id: id.into(),
        name: name.into(),
        icon: None,
        order: None,
        entity_count: 0,
        primary_count: 0,
        group_names: BTreeMap::new(),
        group_favorites: BTreeSet::new(),
    }
}

pub fn entity(id: &str, name: &str, kind: Kind, area: &str) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: Some(format!("dev_{id}")),
        name: name.into(),
        device_name: None,
        label: None,
        kind,
        component: kind.component().to_string(),
        protocol_kind: kind,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: area.into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: Default::default(),
        state: EntityState {
            availability: Availability::Online,
            last_update: Some(NOW - 60),
            ..Default::default()
        },
    }
}

pub fn light(id: &str, name: &str, area: &str, on: bool) -> EntityView {
    let mut e = entity(id, name, Kind::Light, area);
    e.state.value = StateValue::Light(LightState {
        on,
        ..Default::default()
    });
    e
}

/// A light that takes a colour (Zigbee2MQTT's `xy`).
pub fn coloured(mut light: EntityView) -> EntityView {
    light.capabilities.color_modes.insert("xy".into());
    light
}

/// A shutter, open; `can_position` adds a position (0–100 %) to open/close.
pub fn cover(id: &str, name: &str, area: &str, can_position: bool) -> EntityView {
    let mut e = entity(id, name, Kind::Cover, area);
    e.capabilities.open_close = true;
    e.capabilities.stop = true;
    e.capabilities.position = can_position;
    e.state.value = StateValue::Cover(CoverState {
        state: Some("open".into()),
        position: can_position.then_some(100),
    });
    e
}

/// A switch, off, shown as `show_as` (`tv`, `plug`, `siren`...); `None` is a plain switch.
pub fn switch(id: &str, name: &str, area: &str, show_as: Option<&str>) -> EntityView {
    let mut e = entity(id, name, Kind::Switch, area);
    e.ui.show_as = show_as.map(str::to_string);
    e.state.value = StateValue::OnOff(false);
    e
}

/// A media player with every feature, playing "Get Lucky" by "Daft Punk".
pub fn media_player(id: &str, name: &str, area: &str) -> EntityView {
    let mut e = entity(id, name, Kind::MediaPlayer, area);
    e.capabilities.media_features = ["play", "pause", "next", "previous", "play_media"]
        .iter()
        .map(|f| (*f).to_string())
        .collect();
    e.state.value = StateValue::MediaPlayer(MediaPlayerState {
        state: Some("playing".into()),
        title: Some("Get Lucky".into()),
        artist: Some("Daft Punk".into()),
        ..Default::default()
    });
    e
}

/// A satellite's music volume (`device_class: volume`), at `level`.
pub fn speaker_volume(id: &str, area: &str, level: f64) -> EntityView {
    let mut e = entity(id, "Volume musique", Kind::Number, area);
    e.device_class = Some("volume".into());
    e.state.value = StateValue::Number(level);
    e
}

/// A satellite's mute switch (`device_class: mute`), not muted.
pub fn speaker_mute(id: &str, area: &str) -> EntityView {
    let mut e = entity(id, "Muet", Kind::Switch, area);
    e.device_class = Some("mute".into());
    e.state.value = StateValue::OnOff(false);
    e
}

pub fn thermometer(id: &str, area: &str, value: f64) -> EntityView {
    let mut e = entity(id, "Température", Kind::Sensor, area);
    e.device_class = Some("temperature".into());
    e.unit = Some("°C".into());
    e.state.value = StateValue::Number(value);
    e
}

pub fn climate(id: &str, area: &str) -> EntityView {
    let mut e = entity(id, "Climatisation", Kind::Climate, area);
    e.capabilities.temp_step = Some(0.5);
    e.capabilities.min_temp = Some(10.0);
    e.capabilities.max_temp = Some(30.0);
    e
}

pub fn zone(
    id: &str,
    name: &str,
    state: ZoneState,
    setpoint: Option<Setpoint>,
    reading: Option<f64>,
    entities: &[&str],
) -> ZoneView {
    ZoneView {
        zone: id.into(),
        name: name.into(),
        orphan: false,
        settings: ZoneSettings {
            state,
            ..Default::default()
        },
        settings_invalid: None,
        entities: entities.iter().map(|e| e.to_string()).collect(),
        reading,
        reading_age_s: reading.map(|_| 60),
        room_sensor: None,
        thermometer: None,
        correction_step_at: None,
        presence: None,
        decision: setpoint.map(|setpoint| Decision {
            zone: id.into(),
            mode: None,
            setpoint: Some(setpoint),
            source: Source::Schedule,
            cause: None,
            reason: String::new(),
            reading,
            reading_age_s: reading.map(|_| 60),
            outdoor_mean: None,
            home: true,
            holiday: false,
            applied: true,
            at: NOW - 60,
            entities: BTreeMap::new(),
            eco_restore: BTreeMap::new(),
            adjust: None,
        }),
        overrides: Vec::new(),
        eco: None,
        proposal: None,
        learn: LearnStatus::Off,
    }
}

/// Salon (4 lights, the lamp and the led in colour, a hidden one, plus a group; two shutters with a position; a TV), Salon TV,
/// Bureau (a thermometer, no heating; a plug and a plain switch), Chambre Léa (heated, an
/// open/close-only shutter) and Chambre Tom (heated; a plug), Garage (a siren switch).
pub fn house() -> Fixture {
    let mut hidden = light("light_ambiance_tv", "Ambiance TV", "salon", false);
    hidden.ui.hidden = true;
    let mut group = light("group_salon", "Lumières du salon", "salon", false);
    group.device_id = None;
    let mut weather = entity(
        "weather_temperature",
        "Température extérieure",
        Kind::Sensor,
        "",
    );
    weather.device_class = Some("temperature".into());
    weather.state.value = StateValue::Number(7.0);
    Fixture {
        areas: vec![
            area("salon", "Salon"),
            area("salon_tv", "Salon TV"),
            area("bureau", "Bureau"),
            area("chambre_lea", "Chambre Léa"),
            area("chambre_tom", "Chambre Tom"),
            area("garage", "Garage"),
        ],
        entities: vec![
            light("light_plafonnier", "Plafonnier", "salon", false),
            coloured(light("light_lampe_table", "Lampe table", "salon", true)),
            coloured(light("light_led", "Led", "salon", false)),
            hidden,
            group,
            light("light_lampe_lea", "Lampe", "chambre_lea", false),
            cover("cover_baie", "Baie", "salon", true),
            cover("cover_fenetre", "Fenêtre", "salon", true),
            cover("cover_lea", "Volet", "chambre_lea", false),
            switch("switch_tv", "Écran", "salon", Some("tv")),
            switch("switch_prise_bureau", "Cafetière", "bureau", Some("plug")),
            switch("switch_prise_tom", "Chargeur", "chambre_tom", Some("plug")),
            switch("switch_interrupteur", "Interrupteur", "bureau", None),
            switch("switch_sirene", "Sirène", "garage", Some("siren")),
            thermometer("sensor_salon_temperature", "salon", 18.2),
            thermometer("sensor_bureau_temperature", "bureau", 20.1),
            thermometer("sensor_lea_temperature", "chambre_lea", 16.5),
            weather,
            climate("climate_salon", "salon"),
            climate("climate_lea", "chambre_lea"),
            climate("climate_tom", "chambre_tom"),
            media_player("player_music", "Musique", "salon"),
            speaker_volume("number_salon_volume", "salon", 40.0),
            speaker_mute("switch_salon_mute", "salon"),
            speaker_volume("number_bureau_volume", "bureau", 30.0),
        ],
        zones: vec![
            zone(
                "salon",
                "Salon",
                ZoneState::Active,
                Some(Setpoint::Temp(19.0)),
                Some(18.2),
                &["climate_salon"],
            ),
            zone(
                "chambre_lea",
                "Chambre Léa",
                ZoneState::Active,
                Some(Setpoint::Temp(14.0)),
                Some(16.5),
                &["climate_lea"],
            ),
            zone(
                "chambre_tom",
                "Chambre Tom",
                ZoneState::Pending,
                None,
                None,
                &["climate_tom"],
            ),
        ],
        outdoor_entity: None,
        stale_after_min: 60,
        range: HeatRange::default(),
        now: NOW,
    }
}

/// [`house`] with a second player, in the office.
pub fn two_players() -> Fixture {
    let mut f = house();
    f.entities
        .push(media_player("player_bureau", "Musique bureau", "bureau"));
    f
}

/// [`house`] with the bedrooms named after their person only: area `lea` "Léa" and `tom`
/// "Tom", as a house whose rooms are named for who sleeps there.
pub fn person_rooms() -> Fixture {
    let mut f = house();
    let rename = |id: &str| match id {
        "chambre_lea" => "lea".to_string(),
        "chambre_tom" => "tom".to_string(),
        other => other.to_string(),
    };
    for a in &mut f.areas {
        a.id = rename(&a.id);
        a.name = a.name.trim_start_matches("Chambre ").to_string();
    }
    for e in &mut f.entities {
        e.area = rename(&e.area);
    }
    for z in &mut f.zones {
        z.zone = rename(&z.zone);
        z.name = z.name.trim_start_matches("Chambre ").to_string();
        if let Some(d) = &mut z.decision {
            d.zone = rename(&d.zone);
        }
    }
    f
}
