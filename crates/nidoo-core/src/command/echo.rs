//! Which state report confirms a command. Most kinds take any report on their state topics;
//! a media player only the one its command asked for, since its state also changes when
//! someone else uses it.

use crate::model::{Command, Kind, MediaPlayerState, StateValue};

/// Whether `after` (a report that came after `cmd` was published) confirms `cmd`, given the
/// value `before` it. Always true for a kind other than the media player.
pub fn echo_confirms(kind: Kind, cmd: &Command, before: &StateValue, after: &StateValue) -> bool {
    if kind != Kind::MediaPlayer {
        return true;
    }
    let StateValue::MediaPlayer(after) = after else {
        return false;
    };
    let empty = MediaPlayerState::default();
    let before = match before {
        StateValue::MediaPlayer(before) => before,
        _ => &empty,
    };
    let playing = after.state.as_deref() == Some("playing");
    // A new song, or the same one restarted: a new title, or a `position_at` that moved
    // (null while the new song loads).
    let news = after.title != before.title || after.position_at_ms != before.position_at_ms;
    match cmd {
        Command::MediaAction { action } => match action.as_str() {
            "play" => playing,
            "pause" => after.state.as_deref() == Some("paused"),
            "next" | "previous" => news,
            _ => true,
        },
        Command::PlayMedia { .. } => playing && news,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(state: &str, title: &str, at_ms: Option<i64>) -> StateValue {
        StateValue::MediaPlayer(MediaPlayerState {
            state: Some(state.to_owned()),
            title: Some(title.to_owned()),
            position_at_ms: at_ms,
            ..MediaPlayerState::default()
        })
    }

    fn action(action: &str) -> Command {
        Command::MediaAction {
            action: action.to_owned(),
        }
    }

    fn confirms(cmd: &Command, before: &StateValue, after: &StateValue) -> bool {
        echo_confirms(Kind::MediaPlayer, cmd, before, after)
    }

    #[test]
    fn play_is_confirmed_by_playing() {
        let before = p("paused", "A", Some(1000));
        assert!(confirms(
            &action("play"),
            &before,
            &p("playing", "A", Some(2000))
        ));
        assert!(!confirms(
            &action("play"),
            &before,
            &p("paused", "A", Some(1000))
        ));
    }

    #[test]
    fn pause_is_confirmed_by_paused() {
        let before = p("playing", "A", Some(1000));
        assert!(confirms(
            &action("pause"),
            &before,
            &p("paused", "A", Some(2000))
        ));
        assert!(!confirms(
            &action("pause"),
            &before,
            &p("playing", "B", Some(2000))
        ));
    }

    #[test]
    fn next_needs_a_new_title_or_a_changed_position_at() {
        let before = p("playing", "A", Some(1000));
        let next = action("next");
        assert!(!confirms(&next, &before, &p("paused", "A", Some(1000))));
        assert!(confirms(&next, &before, &p("playing", "B", Some(1000))));
        assert!(confirms(&next, &before, &p("playing", "A", Some(2000))));
        assert!(confirms(
            &next,
            &p("playing", "A", None),
            &p("playing", "A", Some(2000))
        ));
        assert!(confirms(&next, &before, &p("playing", "A", None)));
    }

    #[test]
    fn previous_restarting_the_same_song_is_confirmed() {
        let before = p("playing", "A", Some(1000));
        assert!(confirms(
            &action("previous"),
            &before,
            &p("playing", "A", Some(5000))
        ));
    }

    #[test]
    fn play_media_needs_playing_and_news() {
        let before = p("playing", "A", Some(1000));
        let play = Command::PlayMedia {
            query: "x".to_owned(),
        };
        assert!(!confirms(&play, &before, &p("paused", "B", Some(2000))));
        assert!(!confirms(&play, &before, &p("playing", "A", Some(1000))));
        assert!(confirms(&play, &before, &p("playing", "A", Some(2000))));
        assert!(confirms(&play, &before, &p("playing", "B", None)));
    }

    #[test]
    fn before_unknown_counts_as_empty() {
        assert!(confirms(
            &action("next"),
            &StateValue::Unknown,
            &p("playing", "B", None)
        ));
    }

    #[test]
    fn other_kinds_always_confirm() {
        assert!(echo_confirms(
            Kind::Switch,
            &Command::TurnOff,
            &StateValue::OnOff(true),
            &StateValue::OnOff(true)
        ));
    }
}
