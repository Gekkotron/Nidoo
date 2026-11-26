//! The pure half of the Wake-up page: the editor's draft, how it becomes a `WakeUp`, and the
//! summary of a saved alarm.

use nidoo_core::flow::wake_up::{WakeSpeaker, WakeUp, default_name};
use nidoo_core::model::MediaPlayerState;

use crate::i18n::Lang;
use crate::i18n::wake_up::copy_name;
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

    /// The editor of a new alarm copied from saved alarm `name`: same settings and switch. A
    /// typed name gets " (copy)"; a default name (`Wake-up 07:00`) stays default, so it follows
    /// the time the copy is moved to.
    pub fn copy_of(lang: Lang, name: &str, enabled: bool, w: &WakeUp, sats: &[Satellite]) -> Self {
        let defaulted = [true, false]
            .iter()
            .any(|fr| default_name(*fr, w.hour, w.minute) == name.trim());
        Self {
            name: (!defaulted).then(|| copy_name(lang, name.trim())),
            ..Self::from_saved(name, enabled, w, sats)
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
mod tests;
