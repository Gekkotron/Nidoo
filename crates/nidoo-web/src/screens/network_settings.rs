//! The Settings → Network presence page: the hosts the hub pings, one row each, and a form to
//! watch another.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_api::{NetworkWatchSpec, NetworkWatchView, WatchState, WatchStatus};

use crate::api;
use crate::format::area_name;
use crate::house::House;
use crate::i18n::{Lang, NetworkText, network_text, use_lang, use_network_text};
use crate::screens::alarm_settings::SwitchRow;

/// How often the list is fetched again while the page is open.
const REFRESH_MS: u32 = 10_000;

/// The longest id the hub accepts for a watch.
const MAX_ID_LEN: usize = 48;

/// The ping intervals the picker offers, in seconds.
pub fn interval_options() -> &'static [u32] {
    &[10, 15, 30, 60, 120, 300, 600]
}

/// A span of `secs` seconds in its largest whole unit: "45 s", "14 min", "3 h", "2 d".
fn duration_label(secs: u64, lang: Lang) -> String {
    let (n, unit) = match secs {
        0..=59 => (secs, NetworkText::UnitSecond),
        60..=3599 => (secs / 60, NetworkText::UnitMinute),
        3600..=86_399 => (secs / 3600, NetworkText::UnitHour),
        _ => (secs / 86_400, NetworkText::UnitDay),
    };
    format!("{n} {}", network_text(lang, unit))
}

/// An interval as "30 s" or "2 min".
pub fn interval_label(s: u32, lang: Lang) -> String {
    duration_label(u64::from(s), lang)
}

/// Where a watch stands, in words; `now` is Unix seconds.
pub fn state_label(status: &WatchStatus, now: u64, lang: Lang) -> String {
    let t = |k| network_text(lang, k);
    match status.state {
        WatchState::Connected => t(NetworkText::Connected).to_owned(),
        WatchState::Disconnected => match status.since {
            Some(since) => format!(
                "{} {}",
                t(NetworkText::DisconnectedFor),
                duration_label(now.saturating_sub(since), lang)
            ),
            None => t(NetworkText::DisconnectedFor).to_owned(),
        },
        WatchState::Paused => t(NetworkText::Paused).to_owned(),
        WatchState::Waiting => t(NetworkText::Waiting).to_owned(),
        WatchState::CannotPing => t(NetworkText::CannotPing).to_owned(),
    }
}

/// The Settings list's status for this page: how many hosts are watched.
pub fn watched_label(watches: &[NetworkWatchView], lang: Lang) -> String {
    let n = watches.iter().filter(|w| w.spec.enabled).count();
    format!("{n} {}", network_text(lang, NetworkText::Watched))
}

/// A new watch id from its name: lowercase, every character outside `[a-z0-9_]` becomes `_`,
/// then `_2`, `_3`… until it is not in `taken`.
fn new_id(name: &str, taken: &[String]) -> String {
    let mut base: String = name
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
    base.truncate(MAX_ID_LEN - 4);
    if base.is_empty() {
        base.push_str("host");
    }
    let mut id = base.clone();
    let mut n = 2;
    while taken.contains(&id) {
        id = format!("{base}_{n}");
        n += 1;
    }
    id
}

fn now_secs() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

fn blank_spec() -> NetworkWatchSpec {
    NetworkWatchSpec {
        name: String::new(),
        host: String::new(),
        interval_s: 30,
        enabled: true,
        area: None,
        device: None,
    }
}

type Draft = RwSignal<NetworkWatchSpec>;

/// The name, host, interval and room fields of a watch.
#[component]
fn SpecFields(draft: Draft) -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let t = use_network_text();
    let intervals = move || {
        let current = draft.with(|d| d.interval_s);
        let mut all = interval_options().to_vec();
        if !all.contains(&current) {
            all.push(current);
            all.sort_unstable();
        }
        all
    };
    let areas = move || {
        let current = draft.with(|d| d.area.clone());
        let mut all: Vec<(String, String)> = house.areas.with(|a| {
            a.iter()
                .map(|a| (a.id.clone(), area_name(lang(), &a.id, &a.name)))
                .collect()
        });
        if let Some(c) = current
            && !all.iter().any(|(id, _)| *id == c)
        {
            all.push((c.clone(), c));
        }
        all
    };
    view! {
        <div class="network-form">
            <label class="climate-field">
                <span class="climate-field__label">{move || t(NetworkText::Name)}</span>
                <input
                    class="field"
                    type="text"
                    prop:value=move || draft.with(|d| d.name.clone())
                    on:input=move |ev| draft.update(|d| d.name = event_target_value(&ev))
                />
            </label>
            <label class="climate-field">
                <span class="climate-field__label">{move || t(NetworkText::Host)}</span>
                <input
                    class="field"
                    type="text"
                    autocomplete="off"
                    prop:value=move || draft.with(|d| d.host.clone())
                    on:input=move |ev| draft.update(|d| d.host = event_target_value(&ev))
                />
            </label>
            <label class="climate-field">
                <span class="climate-field__label">{move || t(NetworkText::Every)}</span>
                <select
                    class="field"
                    on:change=move |ev| {
                        if let Ok(v) = event_target_value(&ev).parse::<u32>() {
                            draft.update(|d| d.interval_s = v);
                        }
                    }
                >
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
            <label class="climate-field">
                <span class="climate-field__label">{move || t(NetworkText::Room)}</span>
                <select
                    class="field"
                    on:change=move |ev| {
                        let v = event_target_value(&ev);
                        draft.update(|d| d.area = (!v.is_empty()).then_some(v));
                    }
                >
                    <option value="" prop:selected=move || draft.with(|d| d.area.is_none())>
                        {move || t(NetworkText::NoRoom)}
                    </option>
                    {move || {
                        areas()
                            .into_iter()
                            .map(|(id, name)| {
                                let value = id.clone();
                                view! {
                                    <option
                                        value=id
                                        prop:selected=move || draft.with(|d| d.area.as_deref() == Some(value.as_str()))
                                    >
                                        {name}
                                    </option>
                                }
                            })
                            .collect_view()
                    }}
                </select>
            </label>
        </div>
    }
}

/// The draft with its text trimmed, ready to send.
fn trimmed(spec: &NetworkWatchSpec) -> NetworkWatchSpec {
    NetworkWatchSpec {
        name: spec.name.trim().to_owned(),
        host: spec.host.trim().to_owned(),
        ..spec.clone()
    }
}

/// One watch: its state line, and when `editing` names it, its edit form.
#[component]
fn WatchRow(
    id: String,
    watches: RwSignal<Option<Vec<NetworkWatchView>>>,
    editing: RwSignal<Option<String>>,
    reload: Callback<()>,
) -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let t = use_network_text();
    let current = {
        let id = id.clone();
        Memo::new(move |_| {
            watches.with(|w| {
                w.as_ref()
                    .and_then(|w| w.iter().find(|v| v.id == id).cloned())
            })
        })
    };
    let error = RwSignal::new(None::<String>);
    let summary = move || {
        current.get().map(|v| {
            let room = v.spec.area.as_ref().map(|a| {
                house.areas.with(|areas| {
                    areas
                        .iter()
                        .find(|x| x.id == *a)
                        .map_or_else(|| a.clone(), |x| area_name(lang(), &x.id, &x.name))
                })
            });
            let mut parts = vec![
                v.spec.host.clone(),
                format!(
                    "{} {}",
                    t(NetworkText::EveryPrefix),
                    interval_label(v.spec.interval_s, lang())
                ),
            ];
            parts.extend(room);
            parts.push(state_label(&v.status, now_secs(), lang()));
            parts.join(" · ")
        })
    };
    let dot = move || {
        let state = current.with(|v| v.as_ref().map(|v| v.status.state));
        match state {
            Some(WatchState::Connected) => "dot dot--online",
            Some(WatchState::Disconnected) => "dot dot--offline",
            _ => "dot dot--unknown",
        }
    };
    let is_editing = {
        let id = id.clone();
        move || editing.with(|e| e.as_deref() == Some(id.as_str()))
    };
    let form = {
        let id = id.clone();
        let is_editing = is_editing.clone();
        move || {
            if !is_editing() {
                return None;
            }
            let spec = current.with_untracked(|v| v.as_ref().map(|v| v.spec.clone()))?;
            let draft: Draft = RwSignal::new(spec);
            error.set(None);
            let save = {
                let id = id.clone();
                move |_| {
                    let id = id.clone();
                    let spec = trimmed(&draft.get_untracked());
                    spawn_local(async move {
                        match api::set_network_watch(&id, &spec).await {
                            Ok(_) => {
                                editing.try_set(None);
                                reload.run(());
                            }
                            Err(e) => {
                                error.try_set(Some(e.error));
                            }
                        }
                    });
                }
            };
            let remove = {
                let id = id.clone();
                move |_| {
                    let id = id.clone();
                    spawn_local(async move {
                        match api::delete_network_watch(&id).await {
                            Ok(()) => {
                                editing.try_set(None);
                                reload.run(());
                            }
                            Err(e) => {
                                error.try_set(Some(e.error));
                            }
                        }
                    });
                }
            };
            Some(view! {
                <div class="alarm-row__body alarm-row__body--form">
                    <SpecFields draft=draft/>
                    <SwitchRow
                        label=Signal::derive(move || t(NetworkText::WatchOnNetwork).to_owned())
                        on=Signal::derive(move || draft.with(|d| d.enabled))
                        on_toggle=Callback::new(move |v| draft.update(|d| d.enabled = v))
                    />
                    {move || error.get().map(|e| view! { <p class="home-settings__error">{e}</p> })}
                    <div class="network-form__actions">
                        <button type="button" class="chip home-settings__save" on:click=save>
                            {move || t(NetworkText::Save)}
                        </button>
                        <button type="button" class="btn btn--compact" on:click=move |_| editing.set(None)>
                            {move || t(NetworkText::Cancel)}
                        </button>
                        <button type="button" class="btn btn--compact" on:click=remove>
                            {move || t(NetworkText::Remove)}
                        </button>
                    </div>
                </div>
            })
        }
    };
    let edit_id = id.clone();
    view! {
        <li class="alarm-row network-row">
            <div class="alarm-row__head">
                <span class="settings__state">
                    <span class=dot></span>
                </span>
                <span class="alarm-row__name">
                    {move || current.with(|v| v.as_ref().map(|v| v.spec.name.clone()))}
                </span>
                {move || {
                    current
                        .with(|v| v.as_ref().is_some_and(|v| v.spec.device.is_some()))
                        .then(|| {
                            view! { <span class="network-tag">{t(NetworkText::DeviceTag)}</span> }
                        })
                }}
                <Show when={
                    let is_editing = is_editing.clone();
                    move || !is_editing()
                }>
                    <button
                        type="button"
                        class="btn btn--compact"
                        on:click={
                            let edit_id = edit_id.clone();
                            move |_| editing.set(Some(edit_id.clone()))
                        }
                    >
                        {move || t(NetworkText::Edit)}
                    </button>
                </Show>
            </div>
            {move || {
                if is_editing() {
                    None
                } else {
                    Some(view! { <div class="alarm-row__body"><p class="settings__hint">{summary}</p></div> })
                }
            }}
            {form}
        </li>
    }
}

/// The form that starts watching a host that is not a device the hub knows.
#[component]
fn AddHost(
    watches: RwSignal<Option<Vec<NetworkWatchView>>>,
    reload: Callback<()>,
) -> impl IntoView {
    let t = use_network_text();
    let draft: Draft = RwSignal::new(blank_spec());
    let error = RwSignal::new(None::<String>);
    let add = move |_| {
        let spec = trimmed(&draft.get_untracked());
        let taken: Vec<String> = watches.with_untracked(|w| {
            w.as_ref()
                .map(|w| w.iter().map(|v| v.id.clone()).collect())
                .unwrap_or_default()
        });
        let id = new_id(&spec.name, &taken);
        error.set(None);
        spawn_local(async move {
            match api::set_network_watch(&id, &spec).await {
                Ok(_) => {
                    draft.try_set(blank_spec());
                    reload.run(());
                }
                Err(e) => {
                    error.try_set(Some(e.error));
                }
            }
        });
    };
    view! {
        <section class="sheet__panel alarm-section">
            <h2 class="sheet__heading">{move || t(NetworkText::AddTitle)}</h2>
            <SpecFields draft=draft/>
            <p class="settings__hint">{move || t(NetworkText::AddHint)}</p>
            {move || error.get().map(|e| view! { <p class="home-settings__error">{e}</p> })}
            <div class="network-form__actions">
                <button type="button" class="chip home-settings__save" on:click=add>
                    {move || t(NetworkText::Add)}
                </button>
            </div>
        </section>
    }
}

/// The Settings → Network presence page.
#[component]
pub fn NetworkSettingsPage() -> impl IntoView {
    let t = use_network_text();
    let watches = RwSignal::new(None::<Vec<NetworkWatchView>>);
    let failed = RwSignal::new(None::<String>);
    let editing = RwSignal::new(None::<String>);
    let reload = Callback::new(move |()| {
        spawn_local(async move {
            match api::list_network_watches().await {
                Ok(list) => {
                    failed.try_set(None);
                    watches.try_set(Some(list));
                }
                Err(e) => {
                    failed.try_set(Some(e.error));
                }
            }
        });
    });
    Effect::new(move |_| reload.run(()));
    // The hub's verdicts change on their own, so the list is fetched again until the page closes.
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        spawn_local(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(REFRESH_MS).await;
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                reload.run(());
            }
        });
    }
    on_cleanup(move || stop.store(true, Ordering::Relaxed));
    let ids = move || {
        watches.with(|w| {
            w.as_ref()
                .map(|w| w.iter().map(|v| v.id.clone()).collect::<Vec<_>>())
                .unwrap_or_default()
        })
    };
    let cannot_ping = move || {
        watches.with(|w| {
            w.as_ref().is_some_and(|w| {
                w.iter()
                    .any(|v| v.spec.enabled && v.status.state == WatchState::CannotPing)
            })
        })
    };
    view! {
        <div class="alarm-settings">
            {move || {
                failed
                    .get()
                    .map(|e| {
                        view! {
                            <p class="home-settings__error">
                                {format!("{}: {e}", t(NetworkText::CouldNotLoad))}
                            </p>
                        }
                    })
            }}
            <section class="sheet__panel alarm-section">
                <h2 class="sheet__heading">{move || t(NetworkText::WatchedTitle)}</h2>
                <p class="settings__hint">{move || t(NetworkText::WatchedHint)}</p>
                <Show when=cannot_ping>
                    <p class="settings__hint network-hint">{move || t(NetworkText::CannotPingHint)}</p>
                </Show>
                <Show
                    when=move || watches.with(Option::is_some)
                    fallback=move || view! { <p class="settings__hint">{t(NetworkText::Loading)}</p> }
                >
                    <Show
                        when=move || !ids().is_empty()
                        fallback=move || view! { <p class="settings__hint">{t(NetworkText::Empty)}</p> }
                    >
                        <ul class="alarm-rows">
                            <For
                                each=ids
                                key=|id| id.clone()
                                children=move |id| {
                                    view! { <WatchRow id=id watches=watches editing=editing reload=reload /> }
                                }
                            />
                        </ul>
                    </Show>
                </Show>
            </section>
            <AddHost watches=watches reload=reload />
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: WatchState, since: Option<u64>) -> WatchStatus {
        WatchStatus {
            state,
            since,
            last_reply: None,
            rtt_ms: None,
        }
    }

    #[test]
    fn interval_labels() {
        assert_eq!(interval_label(30, Lang::En), "30 s");
        assert_eq!(interval_label(120, Lang::En), "2 min");
        assert_eq!(interval_label(600, Lang::Fr), "10 min");
    }

    #[test]
    fn state_labels() {
        let s = |state, since| state_label(&status(state, since), 1000, Lang::En);
        assert_eq!(s(WatchState::Connected, Some(10)), "connected");
        assert_eq!(
            s(WatchState::Disconnected, Some(1000 - 14 * 60)),
            "disconnected for 14 min"
        );
        assert_eq!(s(WatchState::Paused, None), "paused");
        assert_eq!(s(WatchState::Waiting, None), "waiting for the first reply");
        assert_eq!(s(WatchState::CannotPing, None), "the hub cannot send pings");
        assert_eq!(
            state_label(
                &status(WatchState::Disconnected, Some(20_000 - 3 * 3600)),
                20_000,
                Lang::Fr
            ),
            "déconnecté depuis 3 h"
        );
    }

    #[test]
    fn new_ids_slug_the_name_and_avoid_clashes() {
        assert_eq!(new_id("NAS", &[]), "nas");
        assert_eq!(new_id("Living TV", &[]), "living_tv");
        let taken = vec!["nas".to_owned(), "nas_2".to_owned()];
        assert_eq!(new_id("NAS", &taken), "nas_3");
        assert_eq!(new_id("", &[]), "host");
    }
}
