//! The Network panel of a device's sheet: watch the device by ping from its own page.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_api::{EntityView, NetworkWatchSpec, NetworkWatchView, WatchState};

use crate::api;
use crate::house::House;
use crate::i18n::{Lang, NetworkText, network_text, use_lang, use_network_text};
use crate::screens::alarm_settings::SwitchRow;
use crate::screens::network_settings::{interval_label, interval_options, state_label};

/// How often the watch is fetched again while the sheet is open.
const REFRESH_MS: u32 = 10_000;

/// The longest id the hub accepts for a watch.
const MAX_ID_LEN: usize = 48;

/// Entities the hub makes itself; they have no device to ping.
const HUB_PREFIXES: [&str; 3] = ["group_", "virtual_", "network_"];

/// The id of the watch for a device, the same rule as the hub: lowercase, every character outside
/// `[a-z0-9_]` becomes `_`. `None` when the result is empty or too long.
fn id_for_device(device_id: &str) -> Option<String> {
    let id: String = device_id
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    (!id.is_empty() && id.len() <= MAX_ID_LEN).then_some(id)
}

fn now_secs() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

/// "12 s ago · 4 ms", or a dash before the first reply.
fn last_reply_label(view: &NetworkWatchView, now: u64, lang: Lang) -> String {
    let Some(at) = view.status.last_reply else {
        return "—".to_owned();
    };
    let ago = now.saturating_sub(at);
    let (n, unit) = match ago {
        0..=59 => (ago, NetworkText::UnitSecond),
        60..=3599 => (ago / 60, NetworkText::UnitMinute),
        3600..=86_399 => (ago / 3600, NetworkText::UnitHour),
        _ => (ago / 86_400, NetworkText::UnitDay),
    };
    let span = format!("{n} {}", network_text(lang, unit));
    let ago = network_text(lang, NetworkText::LastReplyAgo).replace("{}", &span);
    match view.status.rtt_ms {
        Some(rtt) => format!(
            "{ago} · {} {}",
            rtt.round() as u64,
            network_text(lang, NetworkText::UnitMillisecond)
        ),
        None => ago,
    }
}

/// The time three missed pings take, in whole seconds up to two minutes ("90 s"), else minutes.
fn miss_window(secs: u32, lang: Lang) -> String {
    if secs > 120 && secs.is_multiple_of(60) {
        interval_label(secs, lang)
    } else {
        format!("{secs} {}", network_text(lang, NetworkText::UnitSecond))
    }
}

/// The panel for `entity`, shown only for a device the hub does not already manage: not a
/// Zigbee2MQTT device and not an entity the hub makes itself.
#[component]
pub fn NetworkPanel(entity: EntityView) -> impl IntoView {
    let house = expect_context::<House>();
    let Some(device_id) = entity.device_id.clone() else {
        return ().into_any();
    };
    if HUB_PREFIXES.iter().any(|p| entity.id.starts_with(p)) {
        return ().into_any();
    }
    let is_zigbee = {
        let device_id = device_id.clone();
        Memo::new(move |_| house.devices.with(|d| d.iter().any(|d| d.id == device_id)))
    };
    let device_name = entity
        .device_name
        .clone()
        .unwrap_or_else(|| entity.name.clone());
    view! {
        <Show when=move || !is_zigbee.get()>
            <PanelBody device_id=device_id.clone() device_name=device_name.clone() />
        </Show>
    }
    .into_any()
}

#[component]
fn PanelBody(device_id: String, device_name: String) -> impl IntoView {
    let lang = use_lang();
    let t = use_network_text();
    let watch = RwSignal::new(None::<NetworkWatchView>);
    let draft = RwSignal::new(NetworkWatchSpec {
        name: format!("{device_name} {}", t(NetworkText::NameSuffix)),
        host: String::new(),
        interval_s: 30,
        enabled: false,
        area: None,
        device: Some(device_id.clone()),
    });
    let error = RwSignal::new(None::<String>);
    // The draft is filled from the stored watch once, so a refresh never overwrites typing.
    let adopted = RwSignal::new(false);
    let load = {
        let device_id = device_id.clone();
        move || {
            let device_id = device_id.clone();
            spawn_local(async move {
                let Ok(list) = api::list_network_watches().await else {
                    return;
                };
                let found = list
                    .into_iter()
                    .find(|w| w.spec.device.as_deref() == Some(device_id.as_str()));
                if let Some(w) = &found
                    && !adopted.try_get_untracked().unwrap_or(true)
                {
                    draft.try_set(w.spec.clone());
                    adopted.try_set(true);
                }
                watch.try_set(found);
            });
        }
    };
    {
        let load = load.clone();
        Effect::new(move |_| load());
    }
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        spawn_local(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(REFRESH_MS).await;
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                load();
            }
        });
    }
    on_cleanup(move || stop.store(true, Ordering::Relaxed));

    // Sends the draft; with no host yet there is nothing to ping, so the fields just wait.
    let save = {
        let device_id = device_id.clone();
        move || {
            let spec = {
                let d = draft.get_untracked();
                NetworkWatchSpec {
                    host: d.host.trim().to_owned(),
                    ..d
                }
            };
            if spec.host.is_empty() {
                return;
            }
            let Some(id) = id_for_device(&device_id) else {
                return;
            };
            error.set(None);
            spawn_local(async move {
                match api::set_network_watch(&id, &spec).await {
                    Ok(view) => {
                        adopted.try_set(true);
                        watch.try_set(Some(view));
                    }
                    Err(e) => {
                        error.try_set(Some(e.error));
                    }
                }
            });
        }
    };
    let on_toggle = {
        let save = save.clone();
        Callback::new(move |v: bool| {
            draft.update(|d| d.enabled = v);
            save();
        })
    };
    let on_host = {
        let save = save.clone();
        move |ev| {
            draft.update(|d| d.host = event_target_value(&ev));
            save();
        }
    };
    let on_interval = move |ev| {
        if let Ok(v) = event_target_value(&ev).parse::<u32>() {
            draft.update(|d| d.interval_s = v);
            save();
        }
    };
    let shown = move || draft.with(|d| d.enabled) || watch.with(Option::is_some);
    let intervals = move || {
        let current = draft.with(|d| d.interval_s);
        let mut all = interval_options().to_vec();
        if !all.contains(&current) {
            all.push(current);
            all.sort_unstable();
        }
        all
    };
    let hint = move || {
        let (every, name) = draft.with(|d| (d.interval_s, d.name.clone()));
        format!(
            "{} ({}). {}",
            t(NetworkText::PanelHintHead),
            miss_window(every * 3, lang()),
            t(NetworkText::PanelHintTail).replace("{}", &name)
        )
    };
    let dot = move || match watch.with(|w| w.as_ref().map(|w| w.status.state)) {
        Some(WatchState::Connected) => "dot dot--online",
        Some(WatchState::Disconnected) => "dot dot--offline",
        _ => "dot dot--unknown",
    };
    view! {
        <section class="sheet__panel">
            <h2 class="sheet__heading">{move || t(NetworkText::PanelTitle)}</h2>
            <SwitchRow
                label=Signal::derive(move || t(NetworkText::WatchOnNetwork).to_owned())
                on=Signal::derive(move || draft.with(|d| d.enabled))
                on_toggle=on_toggle
            />
            <Show when=shown>
                <div class="network-form">
                    <label class="climate-field">
                        <span class="climate-field__label">{move || t(NetworkText::Host)}</span>
                        <input
                            class="field"
                            type="text"
                            autocomplete="off"
                            prop:value=move || draft.with(|d| d.host.clone())
                            on:change=on_host.clone()
                        />
                    </label>
                    <label class="climate-field">
                        <span class="climate-field__label">{move || t(NetworkText::Every)}</span>
                        <select class="field" on:change=on_interval.clone()>
                            {move || {
                                intervals()
                                    .into_iter()
                                    .map(|s| {
                                        view! {
                                            <option
                                                value=s.to_string()
                                                prop:selected=move || draft.with(|d| d.interval_s == s)
                                            >
                                                {interval_label(s, lang())}
                                            </option>
                                        }
                                    })
                                    .collect_view()
                            }}
                        </select>
                    </label>
                </div>
                <p class="settings__hint">{hint}</p>
                {move || error.get().map(|e| view! { <p class="home-settings__error">{e}</p> })}
                <Show when=move || {
                    watch.with(|w| w.as_ref().is_some_and(|w| w.status.state == WatchState::CannotPing))
                }>
                    <p class="settings__hint network-hint">{move || t(NetworkText::CannotPingHint)}</p>
                </Show>
                {move || {
                    watch
                        .get()
                        .map(|w| {
                            let state = state_label(&w.status, now_secs(), lang());
                            let reply = last_reply_label(&w, now_secs(), lang());
                            view! {
                                <dl class="sheet__rows">
                                    <div class="sheet__row">
                                        <dt>{t(NetworkText::State)}</dt>
                                        <dd>
                                            <span class="settings__state">
                                                <span class=dot></span>
                                                " "
                                                {state}
                                            </span>
                                        </dd>
                                    </div>
                                    <div class="sheet__row">
                                        <dt>{t(NetworkText::LastReply)}</dt>
                                        <dd>{reply}</dd>
                                    </div>
                                </dl>
                            }
                        })
                }}
            </Show>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_follow_the_hub_rule() {
        assert_eq!(
            id_for_device("roborock_40313ca8b964").as_deref(),
            Some("roborock_40313ca8b964")
        );
        assert_eq!(id_for_device("Living-TV.1").as_deref(), Some("living_tv_1"));
        assert_eq!(id_for_device(&"a".repeat(49)), None);
        assert_eq!(id_for_device(""), None);
    }
}
