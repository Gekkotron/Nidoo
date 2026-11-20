//! Settings → Wake-up: the switch that shows the Wake-up page, the confirm that turning it off
//! switches the alarms off, and what the media player reports.

use std::collections::BTreeMap;

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use nidoo_api::{EntityView, FlowView};
use nidoo_core::flow::wake_up::is_managed;
use nidoo_core::model::{Kind, StateValue};

use crate::components::toast::Toasts;
use crate::house::House;
use crate::i18n::wake_up::{WakeText, disable_body, disable_title, player_status, wake_text};
use crate::i18n::{HomeText, Lang, home_text, use_lang};

/// The media player that can play a playlist, if one is connected.
pub fn playlist_player(entities: &BTreeMap<String, EntityView>) -> Option<EntityView> {
    entities
        .values()
        .find(|e| {
            e.kind == Kind::MediaPlayer
                && e.capabilities
                    .media_features
                    .iter()
                    .any(|f| f == "play_playlist")
        })
        .cloned()
}

/// How many playlists and outputs a player lists.
pub fn list_counts(player: &EntityView) -> (usize, usize) {
    match &player.state.value {
        StateValue::MediaPlayer(s) => (s.playlists.len(), s.source_list.len()),
        _ => (0, 0),
    }
}

/// The alarms that are on: managed flows whose switch is on.
pub fn active_alarms(flows: &[FlowView]) -> usize {
    flows
        .iter()
        .filter(|f| is_managed(&f.definition) && f.definition.enabled)
        .count()
}

/// Stores `wake_up`: the switch moves at once, and goes back with a toast when the hub refuses.
fn set_wake_up(house: House, on: bool) {
    let previous = house.home_settings.get_untracked();
    if previous.wake_up == on {
        return;
    }
    let settings = nidoo_api::HomeSettings {
        wake_up: on,
        ..previous.clone()
    };
    house.home_settings.set(settings.clone());
    let toasts = use_context::<Toasts>();
    let lang = use_context::<RwSignal<Lang>>();
    spawn_local(async move {
        match crate::api::home::put_home_settings(&settings).await {
            Ok(stored) => {
                house.home_settings.try_set(stored);
            }
            Err(e) => {
                house.home_settings.try_set(previous);
                if let Some(toasts) = toasts {
                    let l = lang.map_or(Lang::En, |l| l.get_untracked());
                    toasts.push(format!(
                        "{}: {}",
                        home_text(l, HomeText::CouldNotSave),
                        e.error
                    ));
                }
            }
        }
    });
}

#[component]
pub fn WakeUpSettingsPage() -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let asking = RwSignal::new(false);
    let on = Memo::new(move |_| house.home_settings.with(|s| s.wake_up));
    let active = Memo::new(move |_| house.flows.with(|f| active_alarms(f)));
    let player = Memo::new(move |_| house.entities.with(playlist_player));
    let toggle = move |_| {
        if !on.get_untracked() {
            set_wake_up(house, true);
        } else if active.get_untracked() > 0 {
            asking.update(|a| *a = !*a);
        } else {
            set_wake_up(house, false);
        }
    };
    // The question goes away once the switch is off, or when the last alarm is gone.
    Effect::new(move |_| {
        if !on.get() || active.get() == 0 {
            asking.set(false);
        }
    });
    view! {
        <div class="wake-settings">
            <p class="wake-settings__lede">{move || wake_text(lang(), WakeText::Lede)}</p>
            <div class="wake-settings__card">
                <div class="wake-settings__head">
                    <div class="wake-settings__titles">
                        <div class="wake-settings__title">
                            {move || wake_text(lang(), WakeText::Enable)}
                        </div>
                        <div class="wake-settings__hint">
                            {move || wake_text(lang(), WakeText::EnableHint)}
                        </div>
                    </div>
                    <button
                        type="button"
                        role="switch"
                        class="switch wake-settings__switch"
                        class:switch--on=move || on.get()
                        aria-checked=move || on.get().to_string()
                        aria-label=move || wake_text(lang(), WakeText::Enable)
                        on:click=toggle
                    >
                        <span class="switch__knob"></span>
                    </button>
                </div>
                <Show when=move || asking.get()>
                    <div class="wake-settings__banner" role="group">
                        <div class="wake-settings__banner-text">
                            <strong>{move || disable_title(lang(), active.get())}</strong>
                            <p>{move || disable_body(lang(), active.get())}</p>
                            <div class="wake-settings__actions">
                                <button
                                    type="button"
                                    class="btn wake-settings__cancel"
                                    on:click=move |_| asking.set(false)
                                >
                                    {move || wake_text(lang(), WakeText::Cancel)}
                                </button>
                                <button
                                    type="button"
                                    class="btn btn--primary wake-settings__confirm"
                                    on:click=move |_| {
                                        asking.set(false);
                                        set_wake_up(house, false);
                                    }
                                >
                                    {move || wake_text(lang(), WakeText::TurnOff)}
                                </button>
                            </div>
                        </div>
                    </div>
                </Show>
                <div class="wake-settings__row">
                    <span class="wake-settings__label">
                        {move || wake_text(lang(), WakeText::Player)}
                    </span>
                    <span class="wake-settings__status">
                        {move || {
                            player.with(|p| {
                                let shown = p.as_ref().map(|p| {
                                    let (playlists, outputs) = list_counts(p);
                                    (p.name.as_str(), playlists, outputs)
                                });
                                player_status(lang(), shown)
                            })
                        }}
                    </span>
                </div>
                <Show when=move || on.get()>
                    <A href="/wake-up" attr:class="wake-settings__link">
                        {move || wake_text(lang(), WakeText::OpenPage)}
                    </A>
                </Show>
            </div>
        </div>
    }
}

/// "Managed by Wake-up · open", on a flow the Wake-up page writes; `open` goes to its editor.
#[component]
pub fn ManagedNote(id: String) -> impl IntoView {
    let lang = use_lang();
    let href = format!("/wake-up/{}", crate::api::encode_id(&id));
    view! {
        <span class="managed-note">
            <span class="managed-note__glyph" inner_html=crate::icons::ALARM_CLOCK></span>
            {move || wake_text(lang(), WakeText::ManagedBy)}
            " \u{b7} "
            <A href=href attr:class="managed-note__link">
                {move || wake_text(lang(), WakeText::OpenManaged)}
            </A>
        </span>
    }
}

/// The warning over the canvas of a flow the Wake-up page writes: edits made here are lost the
/// next time the page saves.
#[component]
pub fn ManagedBanner(id: String) -> impl IntoView {
    let lang = use_lang();
    let href = format!("/wake-up/{}", crate::api::encode_id(&id));
    view! {
        <div class="managed-banner" role="note">
            <span class="managed-banner__glyph" inner_html=crate::icons::ALARM_CLOCK></span>
            <div class="managed-banner__text">
                <strong>{move || wake_text(lang(), WakeText::CanvasBanner)}</strong>
                <p>
                    {move || wake_text(lang(), WakeText::CanvasBannerBody)}
                    " "
                    <A href=href attr:class="managed-note__link">
                        {move || wake_text(lang(), WakeText::EditInWakeUp)}
                    </A>
                </p>
            </div>
        </div>
    }
}
