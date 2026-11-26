//! The wake-up alarm: its settings, and the flow they generate.
//! An alarm is stored as a flow (`wake-up-<n>`, label `wake-up`) whose nodes are regenerated from
//! the settings kept in `extra["wake_up"]`; the web page and the tests share this one generator.

use std::fmt;

use chrono::{DateTime, TimeDelta, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::model::{CommandParams, Edge, Flow, Node, NodeKind, Position, TriggerScheduleParams};
use super::schedule::Schedule;
use crate::model::Command;

/// The label every alarm flow carries.
pub const LABEL: &str = "wake-up";
/// The `Flow.extra` key that holds the settings (a top-level `wake_up` key in the flow JSON).
pub const EXTRA_KEY: &str = "wake_up";
/// The fade-in lengths the page offers, in seconds.
pub const FADE_CHOICES: [u32; 5] = [0, 30, 60, 120, 300];

/// Horizontal gap between two nodes on the canvas.
const NODE_STEP_X: f64 = 280.0;
/// The vertical gap between the speaker steps on the canvas.
const NODE_STEP_Y: f64 = 120.0;

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

/// The flow for an alarm: the schedule fans out to the volume of each checked speaker, their
/// unmute, the mute of the unchecked ones, and last the playlist. The engine runs the branches
/// in edge order, so the speakers are set before the music starts, and a speaker that does not
/// answer ends only its own branch: the alarm still rings. Entity ids are used as stored; none
/// is checked against the registry.
pub fn to_flow(id: &str, name: &str, enabled: bool, w: &WakeUp) -> Flow {
    let mut nodes: Vec<Node> = Vec::new();
    let mut push = |node_id: String, kind: NodeKind| {
        nodes.push(Node {
            id: node_id,
            name: None,
            position: None,
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
            catch_up_minutes: None,
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
    // The trigger on the left, the speaker steps in a column, the playlist on the right.
    let last = nodes.len() - 1;
    for (i, node) in nodes.iter_mut().enumerate() {
        let (x, y) = match i {
            0 => (0.0, 0.0),
            i if i == last => (2.0 * NODE_STEP_X, 0.0),
            i => (NODE_STEP_X, NODE_STEP_Y * (i - 1) as f64),
        };
        node.position = Some(Position { x, y });
    }
    let edges = nodes[1..]
        .iter()
        .map(|node| Edge {
            from: nodes[0].id.clone(),
            to: node.id.clone(),
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

/// The alarms a wake-up trigger follows: `(flow id, cron, schedule)` of every enabled, managed,
/// readable alarm flow, sorted by id. A non-empty `only` keeps just those flow ids.
pub fn alarm_schedules(flows: &[Flow], only: &[String]) -> Vec<(String, String, Schedule)> {
    let mut out: Vec<(String, String, Schedule)> = flows
        .iter()
        .filter(|f| f.enabled && (only.is_empty() || only.contains(&f.id)))
        .filter_map(|f| {
            let w = read(f)?.ok()?;
            let expr = cron(&w);
            let schedule = Schedule::parse(&expr).ok()?;
            Some((f.id.clone(), expr, schedule))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The next pre-ring strictly after `after`: the earliest over the alarms of the ring that
/// follows `after + before`, minus `before`. Absolute time, so a DST change keeps the gap exact.
pub fn next_pre_ring<Tz: TimeZone>(
    alarms: &[Schedule],
    before_minutes: u32,
    after: &DateTime<Tz>,
) -> Option<DateTime<Tz>> {
    let before = TimeDelta::minutes(i64::from(before_minutes));
    let shifted = after.clone() + before;
    alarms
        .iter()
        .filter_map(|s| s.next_after(&shifted))
        .min()
        .map(|ring| ring - before)
}

/// The earliest ring strictly after `now` that is at most `before_minutes` away, over the
/// alarms: the ring a pre-ring computed from `now` would miss.
pub fn ring_within<Tz: TimeZone>(
    alarms: &[Schedule],
    before_minutes: u32,
    now: &DateTime<Tz>,
) -> Option<DateTime<Tz>> {
    let limit = now.clone() + TimeDelta::minutes(i64::from(before_minutes));
    alarms
        .iter()
        .filter_map(|s| s.next_after(now))
        .min()
        .filter(|ring| *ring <= limit)
}

/// The latest pre-ring in `from..=to`, or `None` when no alarm rings in the shifted window.
pub fn latest_pre_ring_in<Tz: TimeZone>(
    alarms: &[Schedule],
    before_minutes: u32,
    from: &DateTime<Tz>,
    to: &DateTime<Tz>,
) -> Option<DateTime<Tz>> {
    let before = TimeDelta::minutes(i64::from(before_minutes));
    let (from, to) = (from.clone() + before, to.clone() + before);
    alarms
        .iter()
        .filter_map(|s| s.latest_in(&from, &to))
        .max()
        .map(|ring| ring - before)
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

/// Why an alarm cannot be re-timed.
#[derive(Debug, PartialEq)]
pub enum RetimeError {
    /// The flow has no `wake_up` settings.
    NotAnAlarm,
    /// The stored settings do not parse; carries serde's message.
    Unreadable(String),
    /// The new time or days are not valid.
    Invalid(WakeUpError),
}

impl fmt::Display for RetimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RetimeError::NotAnAlarm => f.write_str("the flow is not a wake-up alarm"),
            RetimeError::Unreadable(reason) => {
                write!(f, "the alarm's settings are unreadable: {reason}")
            }
            RetimeError::Invalid(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for RetimeError {}

/// The alarm flow with a new time and weekdays. Everything else (id, area, labels, enabled and
/// the other settings) is kept; the nodes and edges are rebuilt by [`to_flow`]. A name that is
/// the default one for the old time, in either language, follows the new time.
pub fn retime(flow: &Flow, hour: u8, minute: u8, days: &[u8]) -> Result<Flow, RetimeError> {
    let old = read(flow)
        .ok_or(RetimeError::NotAnAlarm)?
        .map_err(RetimeError::Unreadable)?;
    let mut days = days.to_vec();
    days.sort_unstable();
    days.dedup();
    let new = WakeUp {
        hour,
        minute,
        days,
        ..old.clone()
    };
    validate(&new).map_err(RetimeError::Invalid)?;
    let rebuilt = to_flow(&flow.id, &flow.name, flow.enabled, &new);
    let name = [false, true]
        .into_iter()
        .find(|fr| flow.name == default_name(*fr, old.hour, old.minute))
        .map_or_else(
            || flow.name.clone(),
            |fr| default_name(fr, new.hour, new.minute),
        );
    let mut out = flow.clone();
    out.name = name;
    out.nodes = rebuilt.nodes;
    out.edges = rebuilt.edges;
    if let Some(value) = rebuilt.extra.get(EXTRA_KEY) {
        out.extra.insert(EXTRA_KEY.to_owned(), value.clone());
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
