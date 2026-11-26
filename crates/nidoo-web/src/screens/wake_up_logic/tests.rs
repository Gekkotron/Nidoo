use nidoo_api::EntityView;
use nidoo_core::model::{
    Capabilities, EntityCategory, EntityState, Kind, MediaChoice, StateValue, UiOverride,
};

use super::*;

fn entity(id: &str) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: id.into(),
        device_name: None,
        label: None,
        kind: Kind::Switch,
        component: "switch".into(),
        protocol_kind: Kind::Switch,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Config,
        enabled_by_default: true,
        area: "room".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState {
            value: StateValue::Unknown,
            ..Default::default()
        },
    }
}

fn sat(name: &str) -> Satellite {
    let slug = name.to_lowercase();
    Satellite {
        device_id: format!("sound-{slug}"),
        name: name.into(),
        area: "room".into(),
        allowed: None,
        mute: Some(entity(&format!("{slug}_mute"))),
        volume: Some(entity(&format!("{slug}_volume"))),
        speech: None,
        connected: None,
    }
}

fn state() -> MediaPlayerState {
    MediaPlayerState {
        playlists: vec![
            MediaChoice {
                id: "favorites".into(),
                name: "Favorites".into(),
            },
            MediaChoice {
                id: "calm".into(),
                name: "Calm morning".into(),
            },
        ],
        source_list: vec![
            MediaChoice {
                id: "cast".into(),
                name: "Cast".into(),
            },
            MediaChoice {
                id: "nidoo".into(),
                name: "Nidoo speakers".into(),
            },
        ],
        ..Default::default()
    }
}

#[test]
fn default_output_prefers_nidoo() {
    assert_eq!(default_output(&state()).as_deref(), Some("nidoo"));
    let no_nidoo = MediaPlayerState {
        source_list: vec![MediaChoice {
            id: "cast".into(),
            name: "Cast".into(),
        }],
        ..Default::default()
    };
    assert_eq!(default_output(&no_nidoo), None);
    assert_eq!(default_output(&MediaPlayerState::default()), None);
}

#[test]
fn new_alarm_starts_from_the_defaults() {
    let d = Draft::new_alarm(&state(), &[sat("Bedroom"), sat("Living")]);
    assert_eq!(d.days, vec![1, 2, 3, 4, 5]);
    assert_eq!((d.hour, d.minute), (7, 0));
    assert_eq!(d.playlist, "favorites");
    assert!(d.shuffle && d.enabled);
    assert_eq!(d.fade_in_s, 60);
    assert_eq!(d.source.as_deref(), Some("nidoo"));
    assert_eq!(d.name, None);
    assert_eq!(d.speakers.len(), 2);
    assert!(
        d.speakers
            .iter()
            .all(|s| s.on && s.volume == DEFAULT_VOLUME)
    );
}

#[test]
fn name_defaults_from_the_time_until_typed() {
    let mut d = Draft::new_alarm(&state(), &[]);
    assert_eq!(d.display_name(Lang::En), "Wake-up 07:00");
    d.hour = 6;
    d.minute = 30;
    assert_eq!(d.display_name(Lang::Fr), "Réveil 06:30");
    d.name = Some("Gym".into());
    assert_eq!(d.display_name(Lang::En), "Gym");
    // A blank name falls back to the default.
    d.name = Some("  ".into());
    assert_eq!(d.display_name(Lang::En), "Wake-up 06:30");
}

#[test]
fn checked_speakers_are_set_and_unchecked_ones_muted() {
    let mut d = Draft::new_alarm(&state(), &[sat("Bedroom"), sat("Living")]);
    d.speakers[1].on = false;
    d.speakers[0].volume = 25.0;
    let w = d.to_wake_up("music-player");
    assert_eq!(w.player, "music-player");
    assert_eq!(
        w.speakers,
        vec![WakeSpeaker {
            volume_entity: "bedroom_volume".into(),
            mute_entity: Some("bedroom_mute".into()),
            volume: 25.0,
        }]
    );
    assert_eq!(w.muted, vec!["living_mute".to_owned()]);
    assert_eq!(nidoo_core::flow::wake_up::validate(&w), Ok(()));
}

#[test]
fn speakers_count_only_on_the_nidoo_output() {
    let mut d = Draft::new_alarm(&state(), &[sat("Bedroom"), sat("Living")]);
    d.source = Some("cast".into());
    let w = d.to_wake_up("music-player");
    assert!(w.speakers.is_empty() && w.muted.is_empty());
    assert_eq!(w.source.as_deref(), Some("cast"));
    d.source = None;
    let w = d.to_wake_up("music-player");
    assert!(w.speakers.is_empty() && w.muted.is_empty());
}

#[test]
fn days_come_out_sorted_and_toggle() {
    let mut d = Draft::new_alarm(&state(), &[]);
    d.toggle_day(0);
    d.toggle_day(3);
    assert_eq!(d.days, vec![1, 2, 4, 5, 0]);
    assert_eq!(d.to_wake_up("p").days, vec![0, 1, 2, 4, 5]);
    for day in [0, 1, 2, 4, 5] {
        d.toggle_day(day);
    }
    assert!(d.days.is_empty());
}

#[test]
fn saved_speaker_choices_are_read_back() {
    let sats = [sat("Bedroom"), sat("Living"), sat("Kitchen")];
    let saved = WakeUp {
        player: "music-player".into(),
        source: Some("nidoo".into()),
        speakers: vec![WakeSpeaker {
            volume_entity: "bedroom_volume".into(),
            mute_entity: Some("bedroom_mute".into()),
            volume: 15.0,
        }],
        muted: vec!["living_mute".into()],
        ..WakeUp::default()
    };
    let d = Draft::from_saved("Weekday", false, &saved, &sats);
    assert_eq!(d.name.as_deref(), Some("Weekday"));
    assert!(!d.enabled);
    let on: Vec<(&str, bool, f64)> = d
        .speakers
        .iter()
        .map(|s| (s.name.as_str(), s.on, s.volume))
        .collect();
    assert_eq!(
        on,
        [
            ("Bedroom", true, 15.0),
            ("Living", false, DEFAULT_VOLUME),
            // Not mentioned by the alarm: left unchecked.
            ("Kitchen", false, DEFAULT_VOLUME),
        ]
    );
}

#[test]
fn a_copy_keeps_the_settings_and_marks_a_typed_name() {
    let sats = [sat("Bedroom"), sat("Living")];
    let saved = WakeUp {
        hour: 6,
        minute: 45,
        days: vec![0, 6],
        playlist: "calm".into(),
        player: "music-player".into(),
        source: Some("nidoo".into()),
        speakers: vec![WakeSpeaker {
            volume_entity: "living_volume".into(),
            mute_entity: Some("living_mute".into()),
            volume: 40.0,
        }],
        muted: vec!["bedroom_mute".into()],
        ..WakeUp::default()
    };
    let copy = Draft::copy_of(Lang::En, "Weekend", false, &saved, &sats);
    assert_eq!(copy.name.as_deref(), Some("Weekend (copy)"));
    assert_eq!(
        Draft {
            name: None,
            ..copy.clone()
        },
        Draft {
            name: None,
            ..Draft::from_saved("Weekend", false, &saved, &sats)
        }
    );
    assert_eq!(copy.to_wake_up("music-player"), saved);
    assert_eq!(
        Draft::copy_of(Lang::Fr, "Weekend", true, &saved, &sats)
            .name
            .as_deref(),
        Some("Weekend (copie)")
    );
}

#[test]
fn a_copy_of_a_default_name_follows_its_time() {
    let saved = WakeUp::default();
    for name in ["Wake-up 07:00", "Réveil 07:00"] {
        let mut copy = Draft::copy_of(Lang::En, name, true, &saved, &[]);
        assert_eq!(copy.name, None, "{name}");
        copy.hour = 8;
        assert_eq!(copy.display_name(Lang::En), "Wake-up 08:00");
    }
}

#[test]
fn vanished_speakers_are_dropped_on_save() {
    // The alarm still names a speaker whose entities are gone, and a muted one.
    let saved = WakeUp {
        player: "music-player".into(),
        source: Some("nidoo".into()),
        speakers: vec![
            WakeSpeaker {
                volume_entity: "gone_volume".into(),
                mute_entity: Some("gone_mute".into()),
                volume: 40.0,
            },
            WakeSpeaker {
                volume_entity: "bedroom_volume".into(),
                mute_entity: Some("bedroom_mute".into()),
                volume: 20.0,
            },
        ],
        muted: vec!["old_mute".into()],
        ..WakeUp::default()
    };
    let d = Draft::from_saved("A", true, &saved, &[sat("Bedroom")]);
    let w = d.to_wake_up("music-player");
    assert_eq!(w.speakers.len(), 1);
    assert_eq!(w.speakers[0].volume_entity, "bedroom_volume");
    assert!(w.muted.is_empty());
}

#[test]
fn summary_names_playlist_speakers_and_fade() {
    let sats = [sat("Bedroom"), sat("Living")];
    let w = WakeUp {
        hour: 7,
        minute: 5,
        player: "music-player".into(),
        playlist: "calm".into(),
        source: Some("nidoo".into()),
        fade_in_s: 120,
        speakers: vec![WakeSpeaker {
            volume_entity: "bedroom_volume".into(),
            mute_entity: None,
            volume: 25.0,
        }],
        ..WakeUp::default()
    };
    let s = summary(Lang::En, &w, &state(), &sats);
    assert_eq!(s.time, "07:05");
    assert_eq!(s.playlist, "Calm morning");
    assert_eq!(s.speakers, vec!["Bedroom 25 %".to_owned()]);
    assert_eq!(s.fade, "2 min");
    let on: Vec<bool> = s.days.iter().map(|d| d.1).collect();
    assert_eq!(on, [true, true, true, true, true, false, false]);
    // A saved playlist the player does not list reads as unknown.
    let gone = WakeUp {
        playlist: "x7f".into(),
        ..w.clone()
    };
    assert_eq!(
        summary(Lang::Fr, &gone, &state(), &sats).playlist,
        "playlist inconnue (x7f)"
    );
    // Off the nidoo output no speaker is listed.
    let cast = WakeUp {
        source: Some("cast".into()),
        ..w
    };
    assert!(
        summary(Lang::En, &cast, &state(), &sats)
            .speakers
            .is_empty()
    );
}
