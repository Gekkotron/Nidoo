//! The detail of one alert: media, text, actions, a link to the device and the source line.

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;
use nidoo_api::{AlertAction, AlertView};

use crate::alerts::match_entity;
use crate::api::send_alert_action;
use crate::components::toast::Toasts;
use crate::house::House;
use crate::i18n::format::{date_time, local_date_time};
use crate::i18n::security::{self, SecurityText, channel_name, security_text, send_failed};
use crate::i18n::{Lang, use_lang};
use crate::icons::channel_icon;
use crate::nav::BackLink;

/// The most action buttons shown.
const MAX_ACTIONS: usize = 3;

/// The full date and time of `sent_at` on the browser's clock, worded in `lang`.
pub fn full_date(lang: Lang, sent_at: u64) -> String {
    date_time(lang, local_date_time(sent_at))
}

/// The source line under the alert.
fn source_line(lang: Lang, source: &str, sent_at: u64) -> String {
    security::source_line(lang, source, &full_date(lang, sent_at))
}

/// The actions that get a button.
fn shown_actions(actions: &[AlertAction]) -> Vec<AlertAction> {
    actions.iter().take(MAX_ACTIONS).cloned().collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Media {
    Image,
    Playing,
    Failed,
}

#[component]
fn MediaBlock(image_url: Option<String>, video_url: Option<String>) -> impl IntoView {
    let lang = use_lang();
    let media = RwSignal::new(Media::Image);
    let has_video = video_url.is_some();
    view! {
        <div class="alert-media">
            {move || match media.get() {
                Media::Failed => {
                    view! { <p class="alert-media__message">
                            {move || security_text(lang(), SecurityText::MediaGone)}
                        </p> }
                        .into_any()
                }
                Media::Playing => {
                    view! {
                        <video
                            controls
                            autoplay
                            playsinline
                            src=video_url.clone()
                            on:error=move |_| media.set(Media::Failed)
                        ></video>
                    }
                        .into_any()
                }
                Media::Image => {
                    view! {
                        {image_url
                            .clone()
                            .map(|src| {
                                view! {
                                    <img src=src alt="" on:error=move |_| media.set(Media::Failed) />
                                }
                            })}
                        {has_video
                            .then(|| {
                                view! {
                                    <button
                                        type="button"
                                        class="btn btn--round alert-media__play"
                                        aria-label=move || security_text(lang(), SecurityText::PlayClip)
                                        title=move || security_text(lang(), SecurityText::Play)
                                        on:click=move |_| media.set(Media::Playing)
                                    >
                                        "▶"
                                    </button>
                                }
                            })}
                    }
                        .into_any()
                }
            }}
        </div>
    }
}

#[component]
fn ActionButton(alert_id: String, action: AlertAction) -> impl IntoView {
    let toasts = expect_context::<Toasts>();
    let lang = use_lang();
    let label = action.label.clone();
    let on_click = move |_| {
        let id = alert_id.clone();
        let action_id = action.id.clone();
        let label = action.label.clone();
        spawn_local(async move {
            match send_alert_action(&id, &action_id).await {
                Ok(()) => toasts.push(security_text(untrack(lang), SecurityText::Sent)),
                Err(_) => toasts.push(send_failed(untrack(lang), &label)),
            }
        });
    };
    view! {
        <button type="button" class="btn" on:click=on_click>
            {label}
        </button>
    }
}

fn back_header(title: Option<String>) -> impl IntoView {
    view! {
        <header class="sheet__header">
            <BackLink />
            {title.map(|t| view! { <div class="sheet__titles"><h1 class="sheet__title">{t}</h1></div> })}
        </header>
    }
}

#[component]
fn DetailBody(alert: AlertView) -> impl IntoView {
    let house = expect_context::<House>();
    let device = house
        .entities
        .with_untracked(|entities| match_entity(alert.entity_id.as_deref(), entities));
    let lang = use_lang();
    let has_media = alert.image_url.is_some() || alert.video_url.is_some();
    let sent_at = alert.sent_at;
    let source = alert.source.clone();
    let actions = shown_actions(&alert.actions);
    let id = alert.id.clone();
    view! {
        {back_header(None)}
        {has_media
            .then(|| {
                view! { <MediaBlock image_url=alert.image_url.clone() video_url=alert.video_url.clone() /> }
            })}
        <h1 class="sheet__title alert-detail__title">{alert.title.clone()}</h1>
        <p class="alert-detail__body">{alert.body.clone()}</p>
        <div class="alert-detail__badges">
            <span class="badge alert-detail__channel">
                <span class="alert-detail__glyph" inner_html=channel_icon(alert.channel)></span>
                {move || channel_name(lang(), alert.channel)}
            </span>
            {alert
                .resolved
                .then(|| {
                    view! {
                        <span class="badge">
                            {move || security_text(lang(), SecurityText::Resolved)}
                        </span>
                    }
                })}
        </div>
        <p class="sheet__updated">{move || full_date(lang(), sent_at)}</p>
        <div class="alert-detail__actions">
            {actions
                .into_iter()
                .map(|action| view! { <ActionButton alert_id=id.clone() action=action /> })
                .collect_view()}
            {device
                .map(|unique_id| {
                    view! {
                        <A href=format!("/entity/{}", crate::api::encode_id(&unique_id)) attr:class="btn alert-detail__device">
                            {move || security_text(lang(), SecurityText::OpenDevice)}
                        </A>
                    }
                })}
        </div>
        {source
            .map(|s| {
                view! {
                    <p class="alert-detail__source">{move || source_line(lang(), &s, sent_at)}</p>
                }
            })}
    }
}

#[component]
pub fn AlertDetailScreen() -> impl IntoView {
    let lang = use_lang();
    let house = expect_context::<House>();
    let params = use_params_map();
    let id = Memo::new(move |_| params.read().get("id").unwrap_or_default());
    let alert = Memo::new(move |_| {
        let wanted = id.get();
        house
            .alerts
            .with(|a| a.iter().find(|a| a.id == wanted).cloned())
    });
    // Before the snapshot arrives the list is empty for a moment; a deep link or a reload must
    // not flash "no longer available" then (same guard as the entity sheet).
    let loaded = Memo::new(move |_| {
        house.alerts.with(|a| !a.is_empty())
            || matches!(house.connection.get(), nidoo_api::ConnectionStatus::Online)
    });
    view! {
        <div class="sheet alert-detail">
            {move || match alert.get() {
                Some(alert) => view! { <DetailBody alert=alert /> }.into_any(),
                None if loaded.get() => {
                    view! {
                        {back_header(None)}
                        <p class="alerts__empty">
                            {move || security_text(lang(), SecurityText::AlertGone)}
                        </p>
                    }
                        .into_any()
                }
                None => {
                    view! {
                        {back_header(None)}
                        <div class="alert-media alert-media--skeleton" aria-busy="true"></div>
                    }
                        .into_any()
                }
            }}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use nidoo_api::AlertAction;

    use super::{full_date, shown_actions, source_line};
    use crate::i18n::Lang;

    #[test]
    fn full_date_follows_the_language() {
        assert_eq!(full_date(Lang::En, 1_792_402_860), "Oct 19 2026, 09:41");
        assert_eq!(full_date(Lang::Fr, 1_792_402_860), "19 oct. 2026, 09:41");
    }

    #[test]
    fn source_line_joins_source_and_date() {
        assert_eq!(
            source_line(Lang::En, "frigate", 1_792_402_860),
            "From frigate · Oct 19 2026, 09:41"
        );
        assert_eq!(
            source_line(Lang::Fr, "frigate", 1_792_402_860),
            "Via frigate · 19 oct. 2026, 09:41"
        );
    }

    #[test]
    fn at_most_three_actions_are_shown() {
        let actions: Vec<AlertAction> = (0..5)
            .map(|i| AlertAction {
                id: format!("a{i}"),
                label: format!("A{i}"),
            })
            .collect();
        let shown = shown_actions(&actions);
        assert_eq!(shown.len(), 3);
        assert_eq!(shown[2].id, "a2");
    }
}
