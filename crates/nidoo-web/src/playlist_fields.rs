//! The inputs of the "Play a playlist" command: the playlist, shuffle, the output and the
//! fade-in, drawn from what the media player reports. The flow Command card and the ambiance
//! device action show the same fields; the Wake-up page reuses the three option helpers.

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::flow::wake_up::FADE_CHOICES;
use nidoo_core::model::{Command, MediaPlayerState, StateValue};

use crate::command_form::FieldStyle;
use crate::i18n::settings::{SettingsText, settings_text};
use crate::i18n::wake_up::{WakeText, fade_text, unknown_output, unknown_playlist, wake_text};
use crate::i18n::{Lang, use_lang};

/// The id the command stores for the player's own Favorites.
const FAVORITES: &str = "favorites";

/// One entry of a playlist, output or fade menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaylistOption {
    pub id: String,
    pub label: String,
    /// False for a saved id the player no longer lists.
    pub known: bool,
}

fn listed(choices: &[nidoo_core::model::MediaChoice]) -> Vec<PlaylistOption> {
    choices
        .iter()
        .map(|c| PlaylistOption {
            id: c.id.clone(),
            label: c.name.clone(),
            known: true,
        })
        .collect()
}

/// The playlists to pick from, as the player lists them (Favorites first). A `saved` id the
/// player does not list is appended as "unknown playlist (id)" so it stays selected; the
/// default `favorites` reads as Favorites even before the player has sent its lists.
pub fn playlist_options(lang: Lang, state: &MediaPlayerState, saved: &str) -> Vec<PlaylistOption> {
    let mut list = listed(&state.playlists);
    if saved.is_empty() || list.iter().any(|o| o.id == saved) {
        return list;
    }
    if saved == FAVORITES {
        list.insert(
            0,
            PlaylistOption {
                id: FAVORITES.to_owned(),
                label: settings_text(lang, SettingsText::Favorites).to_owned(),
                known: true,
            },
        );
    } else {
        list.push(PlaylistOption {
            id: saved.to_owned(),
            label: unknown_playlist(lang, saved),
            known: false,
        });
    }
    list
}

/// The outputs to pick from: the current output (id `""`, no `source`), the player's list,
/// then a `saved` id the player no longer lists as "unknown output (id)".
pub fn output_options(
    lang: Lang,
    state: &MediaPlayerState,
    saved: Option<&str>,
) -> Vec<PlaylistOption> {
    let mut list = vec![PlaylistOption {
        id: String::new(),
        label: wake_text(lang, WakeText::CurrentOutput).to_owned(),
        known: true,
    }];
    list.extend(listed(&state.source_list));
    if let Some(id) = saved.filter(|id| !id.is_empty() && !list.iter().any(|o| o.id == *id)) {
        list.push(PlaylistOption {
            id: id.to_owned(),
            label: unknown_output(lang, id),
            known: false,
        });
    }
    list
}

/// A fade-in length: "Off" / "Sans", "30 s", "1 min", "2 min", "5 min".
pub fn fade_label(lang: Lang, secs: u32) -> String {
    fade_text(lang, secs)
}

/// The command's four fields, with the command's own defaults when it is another command.
fn fields(c: &Command) -> (String, bool, Option<String>, u32) {
    match c {
        Command::PlayPlaylist {
            playlist,
            shuffle,
            source,
            fade_in_s,
        } => (playlist.clone(), *shuffle, source.clone(), *fade_in_s),
        _ => (FAVORITES.to_owned(), true, None, 60),
    }
}

/// One field of the command, set to a new value.
enum Change {
    Playlist(String),
    Shuffle(bool),
    Source(Option<String>),
    Fade(u32),
}

fn rebuild(c: &Command, change: Change) -> Command {
    let (mut playlist, mut shuffle, mut source, mut fade_in_s) = fields(c);
    match change {
        Change::Playlist(v) => playlist = v,
        Change::Shuffle(v) => shuffle = v,
        Change::Source(v) => source = v,
        Change::Fade(v) => fade_in_s = v,
    }
    Command::PlayPlaylist {
        playlist,
        shuffle,
        source,
        fade_in_s,
    }
}

/// The playlist, shuffle, output and fade-in inputs of a `PlayPlaylist` command, over the
/// lists the selected media player reports. Each write keeps the other three fields.
#[component]
pub fn PlaylistFields(
    command: Signal<Command>,
    entity: Signal<Option<EntityView>>,
    write: Callback<Command>,
    style: FieldStyle,
) -> impl IntoView {
    let lang = use_lang();
    let (wrap, caption, _, select) = style.classes();
    let state = Memo::new(move |_| {
        entity.with(|e| match e.as_ref().map(|e| &e.state.value) {
            Some(StateValue::MediaPlayer(s)) => s.clone(),
            _ => MediaPlayerState::default(),
        })
    });
    let saved = move || command.with(fields);
    let playlists = Memo::new(move |_| {
        let (playlist, ..) = saved();
        state.with(|s| playlist_options(lang(), s, &playlist))
    });
    let outputs = Memo::new(move |_| {
        let (_, _, source, _) = saved();
        state.with(|s| output_options(lang(), s, source.as_deref()))
    });
    let fades = Memo::new(move |_| {
        let (.., fade) = saved();
        let mut list = FADE_CHOICES.to_vec();
        if !list.contains(&fade) {
            list.push(fade);
        }
        list
    });
    let edit = move |change: Change| {
        write.run(command.with_untracked(|c| rebuild(c, change)));
    };
    let unknown_selected = move || {
        let (playlist, ..) = saved();
        playlists.with(|l| l.iter().any(|o| o.id == playlist && !o.known))
    };
    view! {
        <label class=wrap>
            <span class=caption>{move || wake_text(lang(), WakeText::Playlist)}</span>
            <select
                class=format!("{select} playlist-fields__playlist")
                on:change=move |ev| {
                    let v = event_target_value(&ev);
                    edit(Change::Playlist(v));
                }
            >
                {move || {
                    let (current, ..) = saved();
                    playlists
                        .get()
                        .into_iter()
                        .map(|o| {
                            let selected = o.id == current;
                            view! { <option value=o.id selected=selected>{o.label}</option> }
                        })
                        .collect_view()
                }}
            </select>
            {move || {
                unknown_selected()
                    .then(|| {
                        view! {
                            <span class="playlist-fields__hint">
                                {move || wake_text(lang(), WakeText::UnknownPlaylistHint)}
                            </span>
                        }
                    })
            }}
        </label>
        <div class="playlist-fields__shuffle">
            <span class=caption>{move || wake_text(lang(), WakeText::Shuffle)}</span>
            <button
                type="button"
                role="switch"
                class="switch"
                class:switch--on=move || saved().1
                aria-checked=move || saved().1.to_string()
                aria-label=move || wake_text(lang(), WakeText::Shuffle)
                on:click=move |_| {
                    let on = !saved().1;
                    edit(Change::Shuffle(on));
                }
            >
                <span class="switch__knob"></span>
            </button>
        </div>
        <label class=wrap>
            <span class=caption>{move || wake_text(lang(), WakeText::Output)}</span>
            <select
                class=format!("{select} playlist-fields__output")
                on:change=move |ev| {
                    let v = event_target_value(&ev);
                    edit(Change::Source((!v.is_empty()).then_some(v)));
                }
            >
                {move || {
                    let current = saved().2.unwrap_or_default();
                    outputs
                        .get()
                        .into_iter()
                        .map(|o| {
                            let selected = o.id == current;
                            view! { <option value=o.id selected=selected>{o.label}</option> }
                        })
                        .collect_view()
                }}
            </select>
        </label>
        <label class=wrap>
            <span class=caption>{move || wake_text(lang(), WakeText::FadeIn)}</span>
            <select
                class=format!("{select} playlist-fields__fade")
                on:change=move |ev| {
                    if let Ok(v) = event_target_value(&ev).parse::<u32>() {
                        edit(Change::Fade(v));
                    }
                }
            >
                {move || {
                    let (.., current) = saved();
                    fades
                        .get()
                        .into_iter()
                        .map(|secs| {
                            view! {
                                <option value=secs.to_string() selected=secs == current>
                                    {fade_label(lang(), secs)}
                                </option>
                            }
                        })
                        .collect_view()
                }}
            </select>
        </label>
    }
}

#[cfg(test)]
mod tests {
    use nidoo_core::model::MediaChoice;

    use super::*;

    fn choice(id: &str, name: &str) -> MediaChoice {
        MediaChoice {
            id: id.into(),
            name: name.into(),
        }
    }

    #[test]
    fn playlist_options_put_saved_unknown_last() {
        let state = MediaPlayerState {
            playlists: vec![choice("favorites", "Favorites"), choice("calm", "Calm")],
            ..Default::default()
        };
        let list = playlist_options(Lang::En, &state, "x7f");
        let ids: Vec<&str> = list.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["favorites", "calm", "x7f"]);
        assert!(list[..2].iter().all(|o| o.known));
        assert!(!list[2].known);
        assert_eq!(list[2].label, "unknown playlist (x7f)");
        // A saved playlist the player lists is not repeated.
        let list = playlist_options(Lang::En, &state, "calm");
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|o| o.known));
    }

    #[test]
    fn unknown_saved_playlist_is_kept() {
        // Pinson offline, or still on a version that sends no lists.
        let state = MediaPlayerState::default();
        let list = playlist_options(Lang::Fr, &state, "x7f");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "x7f");
        assert!(!list[0].known);
        assert!(list[0].label.contains("playlist inconnue"));
        // The default id still reads as Favorites without a list.
        let list = playlist_options(Lang::Fr, &state, "favorites");
        assert_eq!(list.len(), 1);
        assert!(list[0].known);
        assert_eq!(list[0].label, "Favoris");
    }

    #[test]
    fn output_options_start_with_current_output() {
        let state = MediaPlayerState {
            source_list: vec![choice("nidoo", "Nidoo speakers"), choice("cast", "Cast")],
            ..Default::default()
        };
        let list = output_options(Lang::Fr, &state, None);
        assert_eq!(list[0].id, "");
        assert_eq!(list[0].label, "Sortie actuelle");
        assert_eq!(list.len(), 3);
        let list = output_options(Lang::En, &state, Some("gone"));
        assert_eq!(list[0].label, "Current output");
        assert_eq!(list.last().unwrap().id, "gone");
        assert!(!list.last().unwrap().known);
        assert_eq!(list.last().unwrap().label, "unknown output (gone)");
        // A listed output is not repeated.
        assert_eq!(output_options(Lang::En, &state, Some("cast")).len(), 3);
    }

    #[test]
    fn fade_labels_in_both_languages() {
        assert_eq!(fade_label(Lang::En, 0), "Off");
        assert_eq!(fade_label(Lang::Fr, 0), "Sans");
        for lang in [Lang::En, Lang::Fr] {
            assert_eq!(fade_label(lang, 30), "30 s");
            assert_eq!(fade_label(lang, 60), "1 min");
            assert_eq!(fade_label(lang, 120), "2 min");
            assert_eq!(fade_label(lang, 300), "5 min");
        }
    }
}
