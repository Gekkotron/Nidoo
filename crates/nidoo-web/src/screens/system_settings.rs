//! The Settings → System page: how long the hub has been up, the machine's load and storage,
//! house counters and the state of the services it depends on.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use nidoo_api::{ServiceState, ServiceView, SystemService, SystemView};

use crate::api;
use crate::i18n::format::{date_time, decimal, decimal_grouped, local_date_time};
use crate::i18n::{Lang, SystemText, system_text, use_lang, use_system_text};
use crate::screens::alarm_settings::SwitchRow;

/// How often the view is fetched again while the page is open.
const REFRESH_MS: u32 = 60_000;

/// This page's address; the chart and restart views are `?show=` on it.
const HREF: &str = "/settings/system";

fn now_secs() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

/// A span of `secs` seconds as "12 d 4 h", "3 h 5 min" or "4 min".
pub fn uptime_text(lang: Lang, secs: u64) -> String {
    let t = |k| system_text(lang, k);
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    if d > 0 {
        format!(
            "{d} {} {h} {}",
            t(SystemText::UnitDay),
            t(SystemText::UnitHour)
        )
    } else if h > 0 {
        format!(
            "{h} {} {m} {}",
            t(SystemText::UnitHour),
            t(SystemText::UnitMinute)
        )
    } else {
        format!("{m} {}", t(SystemText::UnitMinute))
    }
}

/// The Settings list's status for this page: "Up 12 d 4 h"; `now` is Unix seconds.
pub fn status_label(view: &SystemView, now: u64, lang: Lang) -> String {
    format!(
        "{} {}",
        system_text(lang, SystemText::Up),
        uptime_text(lang, now.saturating_sub(view.started_at))
    )
}

/// A percentage as "23 %".
fn percent(lang: Lang, v: f64) -> String {
    format!("{} %", decimal(lang, v.round(), 0))
}

/// A byte count as "180 MB", or "3.2 GB" from one gigabyte up.
fn size(lang: Lang, bytes: u64) -> String {
    const GB: f64 = 1_073_741_824.0;
    const MB: f64 = 1_048_576.0;
    let b = bytes as f64;
    if b >= GB {
        format!(
            "{} {}",
            decimal(lang, b / GB, 1),
            system_text(lang, SystemText::UnitGb)
        )
    } else {
        format!(
            "{} {}",
            decimal(lang, (b / MB).round(), 0),
            system_text(lang, SystemText::UnitMb)
        )
    }
}

/// `used` of `total` as a whole percentage; `None` when the total is 0.
fn share(used: u64, total: u64) -> Option<f64> {
    (total > 0).then(|| used as f64 * 100.0 / total as f64)
}

/// One tile: a caption, a big value and a small line, a link when `href` is given.
#[component]
fn Tile(
    #[prop(into)] label: Signal<String>,
    #[prop(into)] value: Signal<String>,
    #[prop(into, optional)] sub: Signal<String>,
    #[prop(optional)] href: Option<String>,
) -> impl IntoView {
    let body = view! {
        <span class="system__label">{move || label.get()}</span>
        <span class="system__value">{move || value.get()}</span>
        <span class="system__sub">{move || sub.get()}</span>
    };
    match href {
        Some(href) => {
            view! { <A href=href attr:class="system__tile system__tile--link">{body}</A> }
                .into_any()
        }
        None => view! { <div class="system__tile">{body}</div> }.into_any(),
    }
}

/// A service's tile: a status dot, its state in words and since when.
#[component]
fn ServiceTile(service: ServiceView) -> impl IntoView {
    let lang = use_lang();
    let name = match service.service {
        SystemService::Broker => SystemText::Broker,
        SystemService::Zigbee2mqtt => SystemText::Zigbee,
        SystemService::Database => SystemText::Database,
    };
    let (dot, word) = match service.state {
        ServiceState::Up => ("dot dot--online", SystemText::Up),
        ServiceState::Down => ("dot dot--offline", SystemText::Down),
        ServiceState::Unknown => ("dot dot--unknown", SystemText::Unknown),
    };
    let since = move || {
        service
            .since
            .map(|s| {
                format!(
                    "{} {}",
                    system_text(lang(), SystemText::Since),
                    date_time(lang(), local_date_time(s))
                )
            })
            .unwrap_or_default()
    };
    view! {
        <div class="system__tile">
            <span class="system__label">{move || system_text(lang(), name)}</span>
            <span class="system__value system__value--service">
                <span class=dot></span>
                {move || system_text(lang(), word)}
            </span>
            <span class="system__sub">{since}</span>
        </div>
    }
}

/// The chart tiles and counters, in the order the page shows them.
fn tiles(view: &SystemView, lang: Lang) -> impl IntoView + use<> {
    let t = move |k| system_text(lang, k).to_owned();
    let unknown = t(SystemText::Unknown);
    let link = |show: &str| format!("{HREF}?show={show}");
    let cpu = match &view.cpu {
        Some(c) => view! {
            <Tile
                label=t(SystemText::Cpu)
                value=percent(lang, c.server_pct)
                sub=format!("{} {}", t(SystemText::Hub), percent(lang, c.hub_pct))
                href=link("cpu")
            />
        }
        .into_any(),
        None => view! { <Tile label=t(SystemText::Cpu) value=unknown.clone() /> }.into_any(),
    };
    let ram = match &view.ram {
        Some(r) => view! {
            <Tile
                label=t(SystemText::Ram)
                value=share(r.used_bytes, r.total_bytes)
                    .map_or_else(|| unknown.clone(), |p| percent(lang, p))
                sub=format!("{} {}", t(SystemText::Hub), size(lang, r.hub_bytes))
                href=link("ram")
            />
        }
        .into_any(),
        None => view! { <Tile label=t(SystemText::Ram) value=unknown.clone() /> }.into_any(),
    };
    let disk = match &view.disk {
        Some(d) => view! {
            <Tile
                label=t(SystemText::Disk)
                value=share(d.used_bytes, d.total_bytes)
                    .map_or_else(|| unknown.clone(), |p| percent(lang, p))
                sub=match d.db_bytes {
                    Some(b) => format!("{} {}", t(SystemText::DatabaseSize), size(lang, b)),
                    None => format!("{} / {}", size(lang, d.used_bytes), size(lang, d.total_bytes)),
                }
                href=link("disk")
            />
        }
        .into_any(),
        None => view! { <Tile label=t(SystemText::Disk) value=unknown.clone() /> }.into_any(),
    };
    let temperature = view.temperature_c.map(|c| {
        view! {
            <Tile
                label=t(SystemText::Temperature)
                value=format!("{} °C", decimal(lang, c.round(), 0))
                href=link("temperature")
            />
        }
    });
    let retained = view
        .retained
        .map_or_else(|| unknown.clone(), |n| decimal_grouped(lang, n as f64, 0));
    let services = view
        .services
        .iter()
        .cloned()
        .map(|service| view! { <ServiceTile service /> })
        .collect_view();
    view! {
        {cpu}
        {ram}
        {disk}
        {temperature}
        <Tile
            label=t(SystemText::Devices)
            value=decimal_grouped(lang, f64::from(view.devices), 0)
            sub=format!(
                "{} {}",
                decimal_grouped(lang, f64::from(view.entities), 0),
                t(SystemText::Entities)
            )
        />
        <Tile label=t(SystemText::Retained) value=retained />
        <Tile
            label=t(SystemText::CommandsToday)
            value=decimal_grouped(lang, view.commands_today as f64, 0)
        />
        {services}
    }
}

/// The uptime card: "Up 12 d 4 h", when the hub started, its 30-day availability and restarts.
fn hero(view: &SystemView, lang: Lang) -> impl IntoView + use<> {
    let t = move |k| system_text(lang, k);
    let restarts = view.restarts.len();
    let word = if restarts == 1 {
        SystemText::Restart
    } else {
        SystemText::Restarts
    };
    let up = status_label(view, now_secs(), lang);
    let started = date_time(lang, local_date_time(view.started_at));
    let uptime_pct = format!("{} %", decimal(lang, view.uptime_pct, 1));
    view! {
        <A href=format!("{HREF}?show=restarts") attr:class="system__hero">
            <span class="system__label">{t(SystemText::Hub)}</span>
            <span class="system__up">{up}</span>
            <span class="system__sub">
                {format!("{} {started}", t(SystemText::Since))}
            </span>
            <span class="system__sub">
                {format!("{uptime_pct} {} · {restarts} {}", t(SystemText::UpOver30Days), t(word))}
            </span>
        </A>
    }
}

/// The Settings → System page. The view is fetched on mount and every minute while it is open.
#[component]
pub fn SystemSettingsPage() -> impl IntoView {
    let lang = use_lang();
    let t = use_system_text();
    let data = RwSignal::new(None::<SystemView>);
    let failed = RwSignal::new(None::<String>);
    let notify = RwSignal::new(false);
    let saving = RwSignal::new(false);
    let save_error = RwSignal::new(None::<String>);
    let reload = Callback::new(move |()| {
        spawn_local(async move {
            match api::get_system().await {
                Ok(v) => {
                    failed.try_set(None);
                    if !saving.get_untracked() {
                        notify.try_set(v.notify);
                    }
                    data.try_set(Some(v));
                }
                Err(e) => {
                    failed.try_set(Some(e.error));
                }
            }
        });
    });
    Effect::new(move |_| reload.run(()));
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
    let toggle = Callback::new(move |on: bool| {
        let before = notify.get_untracked();
        notify.set(on);
        saving.set(true);
        save_error.set(None);
        spawn_local(async move {
            match api::set_system_notify(on).await {
                Ok(saved) => {
                    notify.try_set(saved.notify);
                }
                Err(e) => {
                    notify.try_set(before);
                    save_error.try_set(Some(e.error));
                }
            }
            saving.try_set(false);
        });
    });
    view! {
        <div class="system">
            {move || {
                failed
                    .get()
                    .map(|e| {
                        view! {
                            <p class="home-settings__error">
                                {format!("{}: {e}", t(SystemText::CouldNotLoad))}
                            </p>
                        }
                    })
            }}
            {move || match data.get() {
                Some(v) => {
                    let l = lang();
                    view! {
                        {hero(&v, l)}
                        <div class="system__tiles">{tiles(&v, l)}</div>
                    }
                        .into_any()
                }
                None if failed.with(Option::is_none) => {
                    view! { <p class="system__loading">{move || t(SystemText::Loading)}</p> }
                        .into_any()
                }
                None => ().into_any(),
            }}
            <div class="system__notify">
                <SwitchRow
                    label=Signal::derive(move || t(SystemText::NotifyMe).to_owned())
                    on=notify
                    on_toggle=toggle
                />
                <p class="system__hint">{move || t(SystemText::NotifyHint)}</p>
                {move || {
                    save_error.get().map(|e| view! { <p class="home-settings__error">{e}</p> })
                }}
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_text_formats() {
        let secs = 86_400 * 12 + 4 * 3600;
        assert_eq!(uptime_text(Lang::En, secs), "12 d 4 h");
        assert_eq!(uptime_text(Lang::Fr, secs), "12 j 4 h");
        assert_eq!(uptime_text(Lang::En, 3 * 3600 + 5 * 60), "3 h 5 min");
        assert_eq!(uptime_text(Lang::En, 240), "4 min");
        assert_eq!(uptime_text(Lang::En, 30), "0 min");
    }

    #[test]
    fn sizes_switch_unit_at_one_gigabyte() {
        assert_eq!(size(Lang::En, 180 * 1_048_576), "180 MB");
        assert_eq!(size(Lang::En, 3_435_973_837), "3.2 GB");
        assert_eq!(size(Lang::Fr, 3_435_973_837), "3,2 Go");
    }

    #[test]
    fn share_is_none_without_a_total() {
        assert_eq!(share(5, 0), None);
        assert_eq!(share(1, 4), Some(25.0));
    }
}
