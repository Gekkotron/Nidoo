//! Which values the history chart shows: the chips, the default pick, the `?show=` parameter.

use nidoo_api::{FieldTrack, TrackData};
use nidoo_core::model::Kind;

use crate::components::history_chart::format::state_axis_labels;
use crate::components::history_chart::periods::track_label;
use crate::i18n::Lang;
use crate::i18n::history::{HistoryText, history_text};
use crate::i18n::states::device_class_word;

/// Key of the headline value's chip.
pub const HEADLINE_KEY: &str = "state";

/// Where a chip's value is drawn: at most one of each is shown at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Number,
    Word,
}

/// One selectable value. `track` indexes `HistoryFields::tracks`; `None` is the headline number,
/// drawn from `/history/series`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    pub key: String,
    pub slot: Slot,
    pub track: Option<usize>,
}

/// The chips of an entity: a headline number first when no track carries the headline, then one
/// per track (numeric tracks are numbers, every other one is a word).
pub fn chips(tracks: &[FieldTrack]) -> Vec<Chip> {
    let mut out = Vec::with_capacity(tracks.len() + 1);
    if tracks.iter().all(|t| t.field.is_some()) {
        out.push(Chip {
            key: HEADLINE_KEY.to_owned(),
            slot: Slot::Number,
            track: None,
        });
    }
    for (i, t) in tracks.iter().enumerate() {
        out.push(Chip {
            key: t.field.clone().unwrap_or_else(|| HEADLINE_KEY.to_owned()),
            slot: match t.data {
                TrackData::Number { .. } => Slot::Number,
                TrackData::Steps { .. } => Slot::Word,
            },
            track: Some(i),
        });
    }
    out
}

/// The chips shown now: one number and one word, either may be absent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub number: Option<String>,
    pub word: Option<String>,
}

impl Selection {
    fn slot_mut(&mut self, slot: Slot) -> &mut Option<String> {
        match slot {
            Slot::Number => &mut self.number,
            Slot::Word => &mut self.word,
        }
    }
}

/// The first chip in its slot; when that is a word, the first number too.
pub fn default_selection(chips: &[Chip]) -> Selection {
    let mut sel = Selection::default();
    let Some(first) = chips.first() else {
        return sel;
    };
    *sel.slot_mut(first.slot) = Some(first.key.clone());
    if first.slot == Slot::Word
        && let Some(n) = chips.iter().find(|c| c.slot == Slot::Number)
    {
        sel.number = Some(n.key.clone());
    }
    sel
}

/// The selection of a `?show=a,b` value: the first valid key per slot, else the default.
pub fn parse_show(raw: Option<&str>, chips: &[Chip]) -> Selection {
    let mut sel = Selection::default();
    for key in raw.unwrap_or("").split(',') {
        if let Some(c) = chips.iter().find(|c| c.key == key) {
            let slot = sel.slot_mut(c.slot);
            if slot.is_none() {
                *slot = Some(c.key.clone());
            }
        }
    }
    if sel == Selection::default() {
        default_selection(chips)
    } else {
        sel
    }
}

/// The `?show=` value: `number,word`, skipping an empty slot.
pub fn show_param(sel: &Selection) -> String {
    [sel.number.as_deref(), sel.word.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(",")
}

/// A click on a chip: a selected one clears its slot (unless it is the only one shown), an
/// unselected one takes its slot.
pub fn toggle(sel: &Selection, chip: &Chip) -> Selection {
    let mut next = sel.clone();
    let slot = next.slot_mut(chip.slot);
    if slot.as_deref() == Some(chip.key.as_str()) {
        *slot = None;
        if next == Selection::default() {
            return sel.clone();
        }
    } else {
        *slot = Some(chip.key.clone());
    }
    next
}

/// The text of a chip: a track's row label; for the headline number its device class in words,
/// else "Value"; for an on/off headline "State".
pub fn chip_label(
    lang: Lang,
    kind: Kind,
    device_class: Option<&str>,
    tracks: &[FieldTrack],
    chip: &Chip,
) -> String {
    match chip.track.and_then(|i| tracks.get(i)) {
        Some(t) => track_label(lang, kind, t.field.as_deref()),
        None if state_axis_labels(lang, None, &format!("{kind:?}")).is_some() => {
            history_text(lang, HistoryText::State).to_owned()
        }
        None => device_class
            .and_then(|dc| device_class_word(lang, dc))
            .unwrap_or_else(|| history_text(lang, HistoryText::Value))
            .to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(field: Option<&str>, number: bool) -> FieldTrack {
        FieldTrack {
            field: field.map(str::to_owned),
            unit: None,
            data: if number {
                TrackData::Number {
                    window: 0,
                    points: vec![],
                }
            } else {
                TrackData::Steps { changes: vec![] }
            },
        }
    }

    fn vacuum() -> Vec<FieldTrack> {
        vec![
            track(None, false),
            track(Some("battery"), true),
            track(Some("fan_speed"), false),
        ]
    }

    #[test]
    fn a_vacuum_gets_word_headline_then_its_fields() {
        let c = chips(&vacuum());
        assert_eq!(
            c.iter()
                .map(|c| (c.key.as_str(), c.slot))
                .collect::<Vec<_>>(),
            [
                ("state", Slot::Word),
                ("battery", Slot::Number),
                ("fan_speed", Slot::Word)
            ]
        );
    }

    #[test]
    fn a_numeric_headline_comes_first_from_the_series() {
        let c = chips(&[track(Some("battery"), true)]);
        assert_eq!(
            c[0],
            Chip {
                key: "state".into(),
                slot: Slot::Number,
                track: None
            }
        );
        assert_eq!(c[1].track, Some(0));
    }

    #[test]
    fn a_single_chip_hides_the_row() {
        assert_eq!(chips(&[]).len(), 1);
    }

    #[test]
    fn vacuum_default_is_activity_and_battery() {
        assert_eq!(
            default_selection(&chips(&vacuum())),
            Selection {
                number: Some("battery".into()),
                word: Some("state".into())
            }
        );
    }

    #[test]
    fn numeric_headline_default_is_the_line_alone() {
        assert_eq!(
            default_selection(&chips(&[])),
            Selection {
                number: Some("state".into()),
                word: None
            }
        );
    }

    #[test]
    fn parse_show_drops_unknown_keys_and_falls_back() {
        let c = chips(&vacuum());
        assert_eq!(
            parse_show(Some("battery,fan_speed,state"), &c),
            Selection {
                number: Some("battery".into()),
                word: Some("fan_speed".into())
            }
        );
        assert_eq!(
            parse_show(Some("ghost,fan_speed"), &c),
            Selection {
                number: None,
                word: Some("fan_speed".into())
            }
        );
        assert_eq!(parse_show(Some("ghost"), &c), default_selection(&c));
        assert_eq!(parse_show(None, &c), default_selection(&c));
    }

    #[test]
    fn show_param_round_trips() {
        let s = Selection {
            number: Some("battery".into()),
            word: Some("state".into()),
        };
        assert_eq!(show_param(&s), "battery,state");
        assert_eq!(parse_show(Some(&show_param(&s)), &chips(&vacuum())), s);
    }

    #[test]
    fn toggle_swaps_a_slot_clears_it_and_never_empties() {
        let c = chips(&vacuum());
        let s = default_selection(&c);
        let fan = &c[2];
        assert_eq!(toggle(&s, fan).word.as_deref(), Some("fan_speed"));
        let battery = &c[1];
        let no_line = toggle(&s, battery);
        assert_eq!(
            no_line,
            Selection {
                number: None,
                word: Some("state".into())
            }
        );
        assert_eq!(
            toggle(&no_line, &c[0]),
            no_line,
            "the last selected chip stays"
        );
    }

    #[test]
    fn chip_labels() {
        let t = vacuum();
        let c = chips(&t);
        assert_eq!(
            chip_label(Lang::Fr, Kind::Vacuum, None, &t, &c[0]),
            "Activité"
        );
        assert_eq!(
            chip_label(Lang::Fr, Kind::Vacuum, None, &t, &c[2]),
            "Vitesse d'aspiration"
        );
        let s = chips(&[]);
        assert_eq!(
            chip_label(Lang::Fr, Kind::Sensor, None, &[], &s[0]),
            "Valeur"
        );
        assert_eq!(
            chip_label(Lang::Fr, Kind::Sensor, Some("temperature"), &[], &s[0]),
            "Température"
        );
        assert_eq!(
            chip_label(Lang::En, Kind::Sensor, Some("temperature"), &[], &s[0]),
            "Temperature"
        );
        assert_eq!(
            chip_label(Lang::Fr, Kind::Sensor, Some("unheard_of"), &[], &s[0]),
            "Valeur"
        );
        // On/off headlines are a state, whatever the device class.
        assert_eq!(chip_label(Lang::Fr, Kind::Light, None, &[], &s[0]), "État");
        assert_eq!(
            chip_label(Lang::En, Kind::Switch, Some("outlet"), &[], &s[0]),
            "State"
        );
    }
}
