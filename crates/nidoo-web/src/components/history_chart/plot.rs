//! What the history chart draws, decided from the fields reply and the selection. Pure data; the
//! component renders it.

use nidoo_api::{FieldTrack, HistoryFields, HistorySeries, TrackData};

use crate::components::history_chart::Flavor;
use crate::components::history_chart::geom::YScale;
use crate::components::history_chart::selection::{Chip, Selection, Slot, default_selection};

/// The `/history/fields` reply as the view holds it.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldsState {
    Loading,
    Ready(HistoryFields),
    /// The error message.
    Failed(String),
}

/// What the chart draws; `track`/`overlay` index `HistoryFields::tracks`.
#[derive(Clone, Debug, PartialEq)]
pub enum Plot {
    Loading,
    Bars,
    Headline {
        overlay: Option<usize>,
    },
    Field {
        track: usize,
        overlay: Option<usize>,
    },
    Words {
        track: usize,
    },
}

/// Whether the plot can be set against an earlier period: only the bars and the headline series
/// draw a comparison. `None` while the plot is not known yet.
pub fn comparable(plot: &Plot) -> Option<bool> {
    match plot {
        Plot::Loading => None,
        Plot::Bars | Plot::Headline { .. } => Some(true),
        Plot::Field { .. } | Plot::Words { .. } => Some(false),
    }
}

fn find<'a>(chips: &'a [Chip], key: &Option<String>, slot: Slot) -> Option<&'a Chip> {
    let key = key.as_ref()?;
    chips.iter().find(|c| c.slot == slot && &c.key == key)
}

/// What to draw for the selection. A selection matching no chip falls back to the default pick.
pub fn plot_for(flavor: Flavor, fields: &FieldsState, sel: &Selection, chips: &[Chip]) -> Plot {
    if flavor == Flavor::Counter {
        return Plot::Bars;
    }
    match fields {
        FieldsState::Loading => return Plot::Loading,
        FieldsState::Failed(_) => return Plot::Headline { overlay: None },
        FieldsState::Ready(_) => {}
    }
    let mut number = find(chips, &sel.number, Slot::Number);
    let mut word = find(chips, &sel.word, Slot::Word);
    if number.is_none() && word.is_none() {
        let d = default_selection(chips);
        number = find(chips, &d.number, Slot::Number);
        word = find(chips, &d.word, Slot::Word);
    }
    let overlay = word.and_then(|c| c.track);
    match (number, word) {
        (Some(n), _) => match n.track {
            None => Plot::Headline { overlay },
            Some(track) => Plot::Field { track, overlay },
        },
        (
            None,
            Some(Chip {
                track: Some(track), ..
            }),
        ) => Plot::Words { track: *track },
        _ => Plot::Headline { overlay: None },
    }
}

/// The fields failure to show: only when the series has nothing to draw either.
pub fn headline_or_error(fields: &FieldsState, series: &HistorySeries) -> Option<String> {
    match fields {
        FieldsState::Failed(m) if series.points.is_empty() => Some(m.clone()),
        _ => None,
    }
}

/// A number track as a series with its unit; `None` for a steps track.
pub fn field_series(track: &FieldTrack) -> Option<(HistorySeries, Option<String>)> {
    match &track.data {
        TrackData::Number { window, points } => Some((
            HistorySeries {
                window: *window,
                points: points.clone(),
                span: None,
            },
            track.unit.clone(),
        )),
        TrackData::Steps { .. } => None,
    }
}

/// The fixed y extent of a number field track: a percentage always reads 0 to 100. Any other
/// unit, and the entity's own headline value, fit their data.
pub fn percent_scale(unit: Option<&str>) -> Option<YScale> {
    (unit == Some("%")).then_some(YScale {
        min: 0.0,
        max: 100.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::history_chart::selection::chips;

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

    fn vacuum_tracks() -> Vec<FieldTrack> {
        let mut battery = track(Some("battery"), true);
        battery.unit = Some("%".into());
        battery.data = TrackData::Number {
            window: 60,
            points: vec![(0, 50.0, 40.0, 60.0)],
        };
        vec![track(None, false), battery, track(Some("fan_speed"), false)]
    }

    fn ready(tracks: Vec<FieldTrack>) -> FieldsState {
        FieldsState::Ready(HistoryFields {
            from: 0,
            to: 100,
            tracks,
        })
    }

    fn sel(number: Option<&str>, word: Option<&str>) -> Selection {
        Selection {
            number: number.map(str::to_owned),
            word: word.map(str::to_owned),
        }
    }

    #[test]
    fn counters_keep_their_bars() {
        assert_eq!(
            plot_for(
                Flavor::Counter,
                &FieldsState::Loading,
                &Selection::default(),
                &[]
            ),
            Plot::Bars
        );
    }

    #[test]
    fn waits_for_the_fields_reply() {
        assert_eq!(
            plot_for(
                Flavor::Series,
                &FieldsState::Loading,
                &sel(Some("state"), None),
                &[]
            ),
            Plot::Loading
        );
    }

    #[test]
    fn vacuum_default_is_battery_over_activity() {
        let f = ready(vacuum_tracks());
        let c = chips(&vacuum_tracks());
        assert_eq!(
            plot_for(Flavor::Series, &f, &default_selection(&c), &c),
            Plot::Field {
                track: 1,
                overlay: Some(0)
            }
        );
    }

    #[test]
    fn a_word_alone_fills_the_chart() {
        let f = ready(vacuum_tracks());
        let c = chips(&vacuum_tracks());
        assert_eq!(
            plot_for(Flavor::Series, &f, &sel(None, Some("fan_speed")), &c),
            Plot::Words { track: 2 }
        );
    }

    #[test]
    fn a_numeric_headline_draws_from_the_series() {
        let t = vec![track(Some("hvac"), false)];
        let c = chips(&t);
        assert_eq!(
            plot_for(
                Flavor::Series,
                &ready(t),
                &sel(Some("state"), Some("hvac")),
                &c
            ),
            Plot::Headline { overlay: Some(0) }
        );
    }

    #[test]
    fn fields_failure_with_an_empty_series_is_an_error() {
        let failed = FieldsState::Failed("boom".into());
        let empty = HistorySeries {
            window: 3600,
            points: vec![],
            span: None,
        };
        assert_eq!(headline_or_error(&failed, &empty), Some("boom".into()));
        let some = HistorySeries {
            window: 60,
            points: vec![(0, 1.0, 1.0, 1.0)],
            span: None,
        };
        assert_eq!(headline_or_error(&failed, &some), None);
        assert_eq!(headline_or_error(&ready(vec![]), &empty), None);
    }

    #[test]
    fn a_number_track_becomes_a_series_with_its_unit() {
        let (s, unit) = field_series(&vacuum_tracks()[1]).unwrap();
        assert_eq!(unit.as_deref(), Some("%"));
        assert!(!s.points.is_empty());
        assert!(field_series(&vacuum_tracks()[2]).is_none());
    }

    #[test]
    fn an_unknown_selection_falls_back_to_the_default() {
        let f = ready(vacuum_tracks());
        let c = chips(&vacuum_tracks());
        assert_eq!(
            plot_for(Flavor::Series, &f, &sel(Some("gone"), None), &c),
            Plot::Field {
                track: 1,
                overlay: Some(0)
            }
        );
    }

    #[test]
    fn a_percentage_field_reads_zero_to_one_hundred() {
        let s = percent_scale(Some("%")).unwrap();
        assert_eq!((s.min, s.max), (0.0, 100.0));
        assert_eq!(percent_scale(Some("°C")), None);
        assert_eq!(percent_scale(None), None);
    }
}
