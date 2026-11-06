//! The media player card (design B): the cover, the song, ⏮ ▶/❚❚ ⏭ and a progress line, the
//! whole card tinted with the cover's colour when the player sends one.

use leptos::prelude::*;
use nidoo_api::EntityView;
use nidoo_core::model::{Command, EntityState, MediaPlayerState, StateValue};

use super::Sender;
use crate::components::card_frame::{CardFrame, open_entity};
use crate::components::card_place::{CardPlace, use_card_place};
use crate::i18n::devices::{DeviceText, use_device_text};
use crate::icons::{MEDIA_NEXT, MEDIA_NOTE, MEDIA_PREVIOUS, PAUSE, PLAY};

/// Seconds into the song at `now_ms`: the position, plus the time since `position_at` while
/// playing (never before it: the speakers may start a little later), within the song.
/// `None` without a duration. A playing player with no `position_at` is still loading.
pub(crate) fn elapsed_s(s: &MediaPlayerState, now_ms: i64) -> Option<f64> {
    let duration = s.duration_s.filter(|d| *d > 0.0)?;
    let position = s.position_s.unwrap_or(0.0);
    let moved = match (s.state.as_deref(), s.position_at_ms) {
        (Some("playing"), Some(at)) => (now_ms - at).max(0) as f64 / 1000.0,
        _ => 0.0,
    };
    Some((position + moved).clamp(0.0, duration))
}

/// The card's style for a `#rrggbb` cover colour; anything else leaves the card plain (the
/// value goes into a `style` attribute).
pub(crate) fn tint_style(color: Option<&str>) -> Option<String> {
    let digits = color?.strip_prefix('#')?;
    (digits.len() == 6 && digits.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| format!("--media-tint: #{}", digits.to_ascii_lowercase()))
}

fn now_ms() -> i64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as i64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        0
    }
}

#[component]
pub fn MediaPlayerCard(entity: EntityView) -> impl IntoView {
    let sender = Sender::from_context();
    let td = use_device_text();
    let in_sheet = use_card_place() == CardPlace::Sheet;
    let player = match &entity.state.value {
        StateValue::MediaPlayer(p) => p.clone(),
        _ => MediaPlayerState::default(),
    };
    let has = |feature: &str| {
        entity
            .capabilities
            .media_features
            .iter()
            .any(|f| f == feature)
    };
    let playing = player.state.as_deref() == Some("playing");
    let idle = !matches!(player.state.as_deref(), Some("playing" | "paused"));
    let tint = tint_style(player.color.as_deref());

    // A tap sends the command, the play/pause one with its guess so the button flips at once.
    let button = |action: &'static str,
                  label: DeviceText,
                  icon: &'static str,
                  class: &'static str| {
        let id = entity.id.clone();
        let state = entity.state.clone();
        let player = player.clone();
        view! {
            <button
                type="button"
                class=class
                aria-label=move || td(label)
                title=move || td(label)
                inner_html=icon
                on:click=move |_| {
                    let guess = match action {
                        "play" | "pause" => Some(EntityState {
                            value: StateValue::MediaPlayer(MediaPlayerState {
                                state: Some(if action == "play" { "playing" } else { "paused" }.into()),
                                ..player.clone()
                            }),
                            ..state.clone()
                        }),
                        _ => None,
                    };
                    sender.send(&id, Command::MediaAction { action: action.into() }, guess);
                }
            ></button>
        }
    };
    let main_button = if playing {
        has("pause").then(|| button("pause", DeviceText::MediaPause, PAUSE, "media__play"))
    } else {
        has("play").then(|| button("play", DeviceText::MediaPlay, PLAY, "media__play"))
    };
    let previous = (!idle && has("previous")).then(|| {
        button(
            "previous",
            DeviceText::MediaPrevious,
            MEDIA_PREVIOUS,
            "media__skip",
        )
    });
    let next = (!idle && has("next"))
        .then(|| button("next", DeviceText::MediaNext, MEDIA_NEXT, "media__skip"));

    let broken = RwSignal::new(false);
    let image = player.image_url.clone().filter(|_| !idle);
    let placeholder = || view! { <span class="media__placeholder" inner_html=MEDIA_NOTE></span> };
    let cover = move || match image.clone() {
        Some(src) if !broken.get() => view! {
            <img src=src alt="" referrerpolicy="no-referrer" on:error=move |_| broken.set(true) />
        }
        .into_any(),
        _ => placeholder().into_any(),
    };

    let title = player.title.clone();
    let title = move || match &title {
        Some(t) if !idle => t.clone(),
        _ => td(DeviceText::NothingPlaying).to_owned(),
    };
    let byline = [player.artist.clone(), player.album.clone()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let subtitle = move || {
        if idle {
            td(DeviceText::PlaySomeMusic).to_owned()
        } else {
            byline.clone()
        }
    };

    // The line moves by itself while playing; the clock ticks only then.
    let now = RwSignal::new(now_ms());
    #[cfg(target_arch = "wasm32")]
    if playing
        && player.position_at_ms.is_some()
        && let Ok(handle) = set_interval_with_handle(
            move || {
                now.try_set(now_ms());
            },
            std::time::Duration::from_secs(1),
        )
    {
        on_cleanup(move || handle.clear());
    }
    let progress = (!idle).then(|| {
        let p = player.clone();
        elapsed_s(&p, 0).map(|_| {
            let width = move || {
                let duration = p.duration_s.unwrap_or(1.0);
                let done = elapsed_s(&p, now.get()).unwrap_or(0.0);
                format!("width: {:.2}%", done / duration * 100.0)
            };
            view! {
                <div class="media__progress"><span class="media__progress-done" style=width></span></div>
            }
        })
    });

    let pending = sender.pending(&entity.id);
    let on_click = open_entity(&entity.id);
    view! {
        <CardFrame
            entity=entity
            on_click=on_click
            pending=pending
            wide=!in_sheet
            style=tint.unwrap_or_default()
        >
            <div class="media" class:media--idle=idle>
                <div class="media__cover">{cover}</div>
                <div class="media__info">
                    <p class="media__title">{title}</p>
                    <p class="media__byline">{subtitle}</p>
                    <div class="media__controls">{previous}{main_button}{next}</div>
                    {progress}
                </div>
            </div>
        </CardFrame>
    }
}

#[cfg(test)]
mod tests {
    use super::{elapsed_s, tint_style};
    use nidoo_core::model::MediaPlayerState;

    const NOW: i64 = 1_793_960_400_000;

    fn player(state: &str, at: Option<i64>) -> MediaPlayerState {
        MediaPlayerState {
            state: Some(state.into()),
            duration_s: Some(200.0),
            position_s: Some(40.0),
            position_at_ms: at,
            ..Default::default()
        }
    }

    #[test]
    fn elapsed_waits_for_a_future_position_at() {
        assert_eq!(
            elapsed_s(&player("playing", Some(NOW + 800)), NOW),
            Some(40.0)
        );
    }

    #[test]
    fn elapsed_moves_while_playing() {
        assert_eq!(
            elapsed_s(&player("playing", Some(NOW - 3000)), NOW),
            Some(43.0)
        );
    }

    #[test]
    fn elapsed_is_clamped_to_the_duration() {
        assert_eq!(
            elapsed_s(&player("playing", Some(NOW - 600_000)), NOW),
            Some(200.0)
        );
    }

    #[test]
    fn elapsed_is_none_without_duration() {
        let mut p = player("playing", Some(NOW));
        p.duration_s = None;
        assert_eq!(elapsed_s(&p, NOW), None);
        p.duration_s = Some(0.0);
        assert_eq!(elapsed_s(&p, NOW), None);
    }

    #[test]
    fn paused_elapsed_is_the_position() {
        assert_eq!(
            elapsed_s(&player("paused", Some(NOW - 3000)), NOW),
            Some(40.0)
        );
    }

    #[test]
    fn loading_elapsed_stays_at_the_position() {
        let mut p = player("playing", None);
        p.position_s = Some(0.0);
        assert_eq!(elapsed_s(&p, NOW), Some(0.0));
    }

    #[test]
    fn tint_accepts_only_hex() {
        assert_eq!(
            tint_style(Some("#3d2466")).as_deref(),
            Some("--media-tint: #3d2466")
        );
        assert_eq!(tint_style(Some("red")), None);
        assert_eq!(tint_style(Some("#3d2466; x:y")), None);
        assert_eq!(tint_style(None), None);
    }
}
