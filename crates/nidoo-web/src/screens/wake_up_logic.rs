//! The pure half of the Wake-up page: the editor's draft, how it becomes a `WakeUp`, and the
//! summary of a saved alarm.

use nidoo_core::flow::wake_up::{WakeSpeaker, WakeUp, default_name};
use nidoo_core::model::MediaPlayerState;

use crate::i18n::Lang;
use crate::playlist_fields::{fade_label, playlist_options};
use crate::schedule::week_chips;
use crate::screens::sound_settings::Satellite;

/// The output id that plays through the Nidoo speakers.
pub const NIDOO_SOURCE: &str = "nidoo";
/// The volume a speaker gets when the alarm has none stored for it.
pub const DEFAULT_VOLUME: f64 = 30.0;

/// One speaker row of the editor.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeakerRow {
    pub name: String,
    pub volume_entity: String,
    pub mute_entity: Option<String>,
    pub volume: f64,
    /// Checked: unmuted and set to `volume`; unchecked: muted during the alarm.
    pub on: bool,
}

/// Everything the editor holds, as plain values.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    /// The name the user typed; none while the default name (from the time) applies.
    pub name: Option<String>,
    pub enabled: bool,
    pub hour: u8,
    pub minute: u8,
    pub days: Vec<u8>,
    pub playlist: String,
    pub shuffle: bool,
    pub source: Option<String>,
    pub fade_in_s: u32,
    pub speakers: Vec<SpeakerRow>,
}

/// The `nidoo` output when the player lists it.
pub fn default_output(state: &MediaPlayerState) -> Option<String> {
    state
        .source_list
        .iter()
        .any(|c| c.id == NIDOO_SOURCE)
        .then(|| NIDOO_SOURCE.to_owned())
}

/// One row per satellite that has a music volume entity. A saved alarm decides which are
/// checked and at what volume; a satellite it does not mention is unchecked.
pub fn speaker_rows(sats: &[Satellite], saved: Option<&WakeUp>) -> Vec<SpeakerRow> {
    sats.iter()
        .filter_map(|sat| {
            let volume = sat.volume.as_ref()?;
            let mute_entity = sat.mute.as_ref().map(|m| m.id.clone());
            let kept = saved.and_then(|w| w.speakers.iter().find(|s| s.volume_entity == volume.id));
            let (on, level) = match (saved, kept) {
                (None, _) => (true, DEFAULT_VOLUME),
                (Some(_), Some(s)) => (true, s.volume),
                (Some(_), None) => (false, DEFAULT_VOLUME),
            };
            Some(SpeakerRow {
                name: sat.name.clone(),
                volume_entity: volume.id.clone(),
                mute_entity,
                volume: level,
                on,
            })
        })
        .collect()
}

impl Draft {
    /// The editor of a new alarm: the settings' defaults, the default output, every speaker on.
    pub fn new_alarm(state: &MediaPlayerState, sats: &[Satellite]) -> Self {
        let defaults = WakeUp::default();
        Self {
            name: None,
            enabled: true,
            hour: defaults.hour,
            minute: defaults.minute,
            days: defaults.days,
            playlist: defaults.playlist,
            shuffle: defaults.shuffle,
            source: default_output(state),
            fade_in_s: defaults.fade_in_s,
            speakers: speaker_rows(sats, None),
        }
    }

    /// The editor of a saved alarm.
    pub fn from_saved(name: &str, enabled: bool, w: &WakeUp, sats: &[Satellite]) -> Self {
        Self {
            name: Some(name.to_owned()),
            enabled,
            hour: w.hour,
            minute: w.minute,
            days: w.days.clone(),
            playlist: w.playlist.clone(),
            shuffle: w.shuffle,
            source: w.source.clone(),
            fade_in_s: w.fade_in_s,
            speakers: speaker_rows(sats, Some(w)),
        }
    }

    /// The name shown and saved: the typed one, else `Wake-up 07:00`.
    pub fn display_name(&self, lang: Lang) -> String {
        match self.name.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => name.to_owned(),
            _ => default_name(lang == Lang::Fr, self.hour, self.minute),
        }
    }

    /// The settings to save for media player `player`. Speakers count only on the `nidoo`
    /// output; unchecked ones go to `muted`.
    pub fn to_wake_up(&self, player: &str) -> WakeUp {
        let mut days = self.days.clone();
        days.sort_unstable();
        days.dedup();
        let nidoo = self.source.as_deref() == Some(NIDOO_SOURCE);
        let (speakers, muted) = if nidoo {
            let speakers = self
                .speakers
                .iter()
                .filter(|s| s.on)
                .map(|s| WakeSpeaker {
                    volume_entity: s.volume_entity.clone(),
                    mute_entity: s.mute_entity.clone(),
                    volume: s.volume,
                })
                .collect();
            let muted = self
                .speakers
                .iter()
                .filter(|s| !s.on)
                .filter_map(|s| s.mute_entity.clone())
                .collect();
            (speakers, muted)
        } else {
            (Vec::new(), Vec::new())
        };
        WakeUp {
            version: 1,
            hour: self.hour,
            minute: self.minute,
            days,
            player: player.to_owned(),
            playlist: self.playlist.clone(),
            shuffle: self.shuffle,
            source: self.source.clone(),
            fade_in_s: self.fade_in_s,
            speakers,
            muted,
        }
    }

    /// Adds or removes a weekday (0 = Sunday).
    pub fn toggle_day(&mut self, day: u8) {
        if let Some(at) = self.days.iter().position(|d| *d == day) {
            self.days.remove(at);
        } else {
            self.days.push(day);
        }
    }
}

/// `07:05`.
pub fn time_text(hour: u8, minute: u8) -> String {
    format!("{hour:02}:{minute:02}")
}

/// What a list row shows of an alarm.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub time: String,
    /// Monday first: `(chip text, on)`.
    pub days: Vec<(String, bool)>,
    pub playlist: String,
    /// `Bedroom 25 %`, one per checked speaker (none off the `nidoo` output).
    pub speakers: Vec<String>,
    pub fade: String,
}

/// The row of alarm `w`: playlist and speaker names as the player and the satellites give them.
pub fn summary(lang: Lang, w: &WakeUp, state: &MediaPlayerState, sats: &[Satellite]) -> Summary {
    let playlist = playlist_options(lang, state, &w.playlist)
        .into_iter()
        .find(|o| o.id == w.playlist)
        .map_or_else(|| w.playlist.clone(), |o| o.label);
    let speakers = if w.source.as_deref() == Some(NIDOO_SOURCE) {
        w.speakers
            .iter()
            .map(|s| {
                let name = sats
                    .iter()
                    .find(|sat| sat.volume.as_ref().is_some_and(|v| v.id == s.volume_entity))
                    .map_or(s.volume_entity.as_str(), |sat| sat.name.as_str());
                format!("{name} {} %", s.volume.round())
            })
            .collect()
    } else {
        Vec::new()
    };
    Summary {
        time: time_text(w.hour, w.minute),
        days: week_chips(lang)
            .iter()
            .map(|(d, text)| {
                let short: String = text.chars().take(1).collect();
                (short, w.days.contains(d))
            })
            .collect(),
        playlist,
        speakers,
        fade: fade_label(lang, w.fade_in_s),
    }
}

#[cfg(test)]
mod tests {
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
        let w = d.to_wake_up("pinson");
        assert_eq!(w.player, "pinson");
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
        let w = d.to_wake_up("pinson");
        assert!(w.speakers.is_empty() && w.muted.is_empty());
        assert_eq!(w.source.as_deref(), Some("cast"));
        d.source = None;
        let w = d.to_wake_up("pinson");
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
            player: "pinson".into(),
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
    fn vanished_speakers_are_dropped_on_save() {
        // The alarm still names a speaker whose entities are gone, and a muted one.
        let saved = WakeUp {
            player: "pinson".into(),
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
        let w = d.to_wake_up("pinson");
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
            player: "pinson".into(),
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
}
