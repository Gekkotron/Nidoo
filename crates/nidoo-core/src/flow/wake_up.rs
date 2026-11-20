//! The wake-up alarm: its settings, and the flow they generate.
//! An alarm is stored as a flow (`wake-up-<n>`, label `wake-up`) whose nodes are regenerated from
//! the settings kept in `extra["wake_up"]`; the web page and the tests share this one generator.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::model::{CommandParams, Edge, Flow, Node, NodeKind, Position, TriggerScheduleParams};
use crate::model::Command;

/// The label every alarm flow carries.
pub const LABEL: &str = "wake-up";
/// The `Flow.extra` key that holds the settings (a top-level `wake_up` key in the flow JSON).
pub const EXTRA_KEY: &str = "wake_up";
/// The fade-in lengths the page offers, in seconds.
pub const FADE_CHOICES: [u32; 5] = [0, 30, 60, 120, 300];

/// Horizontal gap between two nodes on the canvas.
const NODE_STEP_X: f64 = 280.0;

/// One alarm's settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WakeUp {
    /// Schema version, `1`.
    pub version: u32,
    pub hour: u8,
    pub minute: u8,
    /// 0 = Sunday … 6 = Saturday, sorted, deduplicated, never empty.
    pub days: Vec<u8>,
    /// The media player entity id.
    pub player: String,
    /// `favorites` or a player playlist id.
    pub playlist: String,
    pub shuffle: bool,
    /// An id from the player's `source_list`, or the player's current output.
    pub source: Option<String>,
    /// Seconds of fade-in, one of [`FADE_CHOICES`].
    pub fade_in_s: u32,
    /// The checked speakers: unmuted and set to their volume.
    pub speakers: Vec<WakeSpeaker>,
    /// The mute switch entity ids of the unchecked speakers.
    pub muted: Vec<String>,
}

/// A speaker the alarm unmutes and sets to a volume.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WakeSpeaker {
    /// The number entity that holds the speaker's music volume.
    pub volume_entity: String,
    /// The speaker's mute switch, when it has one.
    pub mute_entity: Option<String>,
    pub volume: f64,
}

impl Default for WakeUp {
    fn default() -> Self {
        Self {
            version: 1,
            hour: 7,
            minute: 0,
            days: vec![1, 2, 3, 4, 5],
            player: String::new(),
            playlist: "favorites".to_owned(),
            shuffle: true,
            source: None,
            fade_in_s: 60,
            speakers: Vec::new(),
            muted: Vec::new(),
        }
    }
}

/// Why an alarm cannot be saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WakeUpError {
    /// No weekday is picked (or one is out of range).
    NoDay,
    /// The hour is above 23 or the minute above 59.
    BadTime,
    /// The playlist id is blank.
    BlankPlaylist,
    /// No media player is set.
    NoPlayer,
}

impl fmt::Display for WakeUpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            WakeUpError::NoDay => "pick at least one day",
            WakeUpError::BadTime => "the time is not a valid hour and minute",
            WakeUpError::BlankPlaylist => "the playlist is blank",
            WakeUpError::NoPlayer => "no media player is set",
        })
    }
}

impl std::error::Error for WakeUpError {}

/// `1-5` for three or more consecutive days, else a comma list.
pub fn days_field(days: &[u8]) -> String {
    let mut sorted: Vec<u8> = days.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    if sorted.len() >= 3 && sorted.windows(2).all(|w| w[1] == w[0] + 1) {
        return format!("{}-{}", sorted[0], sorted[sorted.len() - 1]);
    }
    sorted
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// The cron expression of the alarm: `M H * * <days>`.
pub fn cron(w: &WakeUp) -> String {
    format!("{} {} * * {}", w.minute, w.hour, days_field(&w.days))
}

/// Checks the settings can make a working flow.
pub fn validate(w: &WakeUp) -> Result<(), WakeUpError> {
    if w.days.is_empty() || w.days.iter().any(|d| *d > 6) {
        return Err(WakeUpError::NoDay);
    }
    if w.hour > 23 || w.minute > 59 {
        return Err(WakeUpError::BadTime);
    }
    if w.playlist.trim().is_empty() {
        return Err(WakeUpError::BlankPlaylist);
    }
    if w.player.trim().is_empty() {
        return Err(WakeUpError::NoPlayer);
    }
    Ok(())
}

/// The flow for an alarm: schedule, then volume of each checked speaker, their unmute, the mute
/// of the unchecked ones, and the playlist, as one chain. Entity ids are used as stored; none is
/// checked against the registry.
pub fn to_flow(id: &str, name: &str, enabled: bool, w: &WakeUp) -> Flow {
    let mut nodes: Vec<Node> = Vec::new();
    let mut push = |node_id: String, kind: NodeKind| {
        nodes.push(Node {
            id: node_id,
            name: None,
            position: Some(Position {
                x: NODE_STEP_X * nodes.len() as f64,
                y: 0.0,
            }),
            kind,
            extra: Default::default(),
        });
    };
    let command = |entity: &str, command: Command| {
        NodeKind::Command(CommandParams {
            entity: entity.to_owned(),
            command,
            value_from: None,
            extra: Default::default(),
        })
    };
    push(
        "trigger".to_owned(),
        NodeKind::TriggerSchedule(TriggerScheduleParams {
            cron: cron(w),
            extra: Default::default(),
        }),
    );
    for (i, speaker) in w.speakers.iter().enumerate() {
        push(
            format!("volume-{i}"),
            command(
                &speaker.volume_entity,
                Command::SetNumber {
                    value: speaker.volume,
                },
            ),
        );
    }
    for (i, speaker) in w.speakers.iter().enumerate() {
        if let Some(mute) = &speaker.mute_entity {
            push(format!("unmute-{i}"), command(mute, Command::TurnOff));
        }
    }
    for (i, mute) in w.muted.iter().enumerate() {
        push(
            format!("mute-{i}"),
            command(
                mute,
                Command::TurnOn {
                    brightness: None,
                    color: None,
                    effect: None,
                },
            ),
        );
    }
    push(
        "play".to_owned(),
        command(
            &w.player,
            Command::PlayPlaylist {
                playlist: w.playlist.clone(),
                shuffle: w.shuffle,
                source: w.source.clone(),
                fade_in_s: w.fade_in_s,
            },
        ),
    );
    let edges = nodes
        .windows(2)
        .map(|pair| Edge {
            from: pair[0].id.clone(),
            to: pair[1].id.clone(),
            port: None,
        })
        .collect();
    let mut flow = Flow {
        id: id.to_owned(),
        name: name.to_owned(),
        labels: vec![LABEL.to_owned()],
        enabled,
        nodes,
        edges,
        ..Flow::default()
    };
    if let Ok(value) = serde_json::to_value(w) {
        flow.extra.insert(EXTRA_KEY.to_owned(), value);
    }
    flow
}

/// The settings kept in a flow: `None` when it is not a wake-up flow, `Some(Err)` with serde's
/// message when its `wake_up` key does not parse.
pub fn read(flow: &Flow) -> Option<Result<WakeUp, String>> {
    let value: &Value = flow.extra.get(EXTRA_KEY)?;
    Some(serde_json::from_value(value.clone()).map_err(|e| e.to_string()))
}

/// Whether the page manages this flow (it has a `wake_up` key, readable or not).
pub fn is_managed(flow: &Flow) -> bool {
    flow.extra.contains_key(EXTRA_KEY)
}

/// The lowest free `wake-up-<n>` id, `n` from 1.
pub fn next_id(taken: &[&str]) -> String {
    (1..)
        .map(|n| format!("wake-up-{n}"))
        .find(|id| !taken.contains(&id.as_str()))
        .unwrap_or_else(|| "wake-up-1".to_owned())
}

/// The name a new alarm gets: `Réveil 07:00` or `Wake-up 07:00`.
pub fn default_name(lang_is_fr: bool, hour: u8, minute: u8) -> String {
    let word = if lang_is_fr { "Réveil" } else { "Wake-up" };
    format!("{word} {hour:02}:{minute:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn alarm() -> WakeUp {
        WakeUp {
            hour: 7,
            minute: 5,
            player: "pinson-player".to_owned(),
            ..WakeUp::default()
        }
    }

    #[test]
    fn cron_for_weekdays() {
        assert_eq!(cron(&alarm()), "5 7 * * 1-5");
        let weekend = WakeUp {
            days: vec![0, 6],
            ..alarm()
        };
        assert_eq!(cron(&weekend), "5 7 * * 0,6");
        let all = WakeUp {
            days: (0..=6).collect(),
            ..alarm()
        };
        assert_eq!(cron(&all), "5 7 * * 0-6");
    }

    fn full() -> WakeUp {
        WakeUp {
            source: Some("kitchen".to_owned()),
            speakers: vec![
                WakeSpeaker {
                    volume_entity: "number.a_music".to_owned(),
                    mute_entity: Some("switch.a_mute".to_owned()),
                    volume: 25.0,
                },
                WakeSpeaker {
                    volume_entity: "number.b_music".to_owned(),
                    mute_entity: None,
                    volume: 25.0,
                },
            ],
            muted: vec!["switch.c_mute".to_owned()],
            ..alarm()
        }
    }

    #[test]
    fn to_flow_chains_schedule_speakers_mutes_play() {
        let flow = to_flow("wake-up-1", "Wake-up 07:05", true, &full());
        assert!(flow.enabled);
        assert_eq!(flow.labels, [LABEL]);
        let ids: Vec<&str> = flow.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "trigger", "volume-0", "volume-1", "unmute-0", "mute-0", "play"
            ]
        );
        let NodeKind::TriggerSchedule(trigger) = &flow.nodes[0].kind else {
            panic!("first node is the schedule");
        };
        assert_eq!(trigger.cron, "5 7 * * 1-5");
        let command_of = |i: usize| match &flow.nodes[i].kind {
            NodeKind::Command(c) => (c.entity.clone(), c.command.clone()),
            other => panic!("not a command: {other:?}"),
        };
        assert_eq!(
            command_of(1),
            ("number.a_music".into(), Command::SetNumber { value: 25.0 })
        );
        assert_eq!(
            command_of(2),
            ("number.b_music".into(), Command::SetNumber { value: 25.0 })
        );
        assert_eq!(command_of(3), ("switch.a_mute".into(), Command::TurnOff));
        assert!(matches!(
            command_of(4),
            (e, Command::TurnOn { .. }) if e == "switch.c_mute"
        ));
        assert_eq!(
            command_of(5),
            (
                "pinson-player".into(),
                Command::PlayPlaylist {
                    playlist: "favorites".into(),
                    shuffle: true,
                    source: Some("kitchen".into()),
                    fade_in_s: 60,
                }
            )
        );
        let chain: Vec<(&str, &str)> = flow
            .edges
            .iter()
            .map(|e| (e.from.as_str(), e.to.as_str()))
            .collect();
        assert_eq!(
            chain,
            [
                ("trigger", "volume-0"),
                ("volume-0", "volume-1"),
                ("volume-1", "unmute-0"),
                ("unmute-0", "mute-0"),
                ("mute-0", "play"),
            ]
        );
        for (i, node) in flow.nodes.iter().enumerate() {
            let p = node.position.expect("position");
            assert_eq!((p.x, p.y), (280.0 * i as f64, 0.0));
        }
        assert_eq!(read(&flow), Some(Ok(full())));
        let off = to_flow("wake-up-1", "x", false, &full());
        assert!(!off.enabled);
        // The generated flow survives the wire and still reads back.
        let again = Flow::decode(flow.encode().as_bytes()).expect("decode");
        assert_eq!(read(&again), Some(Ok(full())));
        assert!(is_managed(&again));
    }

    #[test]
    fn flow_keeps_stored_speaker_ids() {
        let w = WakeUp {
            speakers: vec![WakeSpeaker {
                volume_entity: "number.gone_music".to_owned(),
                mute_entity: Some("switch.gone_mute".to_owned()),
                volume: 10.0,
            }],
            muted: vec!["switch.deleted".to_owned()],
            ..alarm()
        };
        let flow = to_flow("wake-up-2", "x", true, &w);
        let entities: Vec<&str> = flow
            .nodes
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::Command(c) => Some(c.entity.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            entities,
            [
                "number.gone_music",
                "switch.gone_mute",
                "switch.deleted",
                "pinson-player"
            ]
        );
    }

    #[test]
    fn read_none_for_plain_flow() {
        let flow = Flow::default();
        assert!(read(&flow).is_none());
        assert!(!is_managed(&flow));
    }

    #[test]
    fn unparsable_settings_are_an_error() {
        let mut flow = Flow::default();
        flow.extra
            .insert(EXTRA_KEY.to_owned(), json!({"hour": "x"}));
        assert!(matches!(read(&flow), Some(Err(_))));
        assert!(is_managed(&flow));
    }

    #[test]
    fn validate_rejects_no_day_and_blank_playlist() {
        assert_eq!(validate(&alarm()), Ok(()));
        let no_day = WakeUp {
            days: vec![],
            ..alarm()
        };
        assert_eq!(validate(&no_day), Err(WakeUpError::NoDay));
        let blank = WakeUp {
            playlist: "  ".to_owned(),
            ..alarm()
        };
        assert_eq!(validate(&blank), Err(WakeUpError::BlankPlaylist));
        let late = WakeUp {
            hour: 24,
            ..alarm()
        };
        assert_eq!(validate(&late), Err(WakeUpError::BadTime));
        assert_eq!(validate(&WakeUp::default()), Err(WakeUpError::NoPlayer));
    }

    #[test]
    fn next_id_takes_lowest_free() {
        assert_eq!(next_id(&["wake-up-1", "wake-up-3"]), "wake-up-2");
        assert_eq!(next_id(&[]), "wake-up-1");
    }

    #[test]
    fn default_name_pads_the_time() {
        assert_eq!(default_name(true, 7, 0), "Réveil 07:00");
        assert_eq!(default_name(false, 6, 45), "Wake-up 06:45");
    }
}
