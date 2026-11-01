//! The Security screen: alarm panels, actions, the summary line and the sensor sections.

use std::collections::BTreeMap;

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use nidoo_core::model::Kind;
use time::{OffsetDateTime, UtcOffset};

use crate::api::history::fetch_last_change;
use crate::cards::Sender;
use crate::components::alarm_hero::AlarmHero;
use crate::components::alert_row::RecentAlerts;
use crate::components::history_chart::paris_offset;
use crate::components::now::Now;
use crate::house::House;
use crate::i18n::security::{
    SecurityText, action_label, section_title, security_text, since_days, since_yesterday,
};
use crate::i18n::{Key, Lang, format as fmt, text, use_lang, use_text};
use crate::icons::{PLAY, PLUG, sensor_icon};
use crate::security::{ActionRow, SecurityUi, SensorRow};

/// Shared per-screen map `sensor id → last-change unix seconds (None if the hub has no row)`.
#[derive(Clone, Copy)]
struct SinceMap(RwSignal<BTreeMap<String, Option<i64>>>);

fn local_dt(unix: i64) -> OffsetDateTime {
    let off = paris_offset(unix);
    let offset = UtcOffset::from_whole_seconds(off).unwrap_or(UtcOffset::UTC);
    OffsetDateTime::from_unix_timestamp(unix)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .to_offset(offset)
}

/// Natural-language time since `changed_at`. `""` when the sensor has no row.
pub fn since_label(lang: Lang, now_secs: i64, changed_at: Option<i64>) -> String {
    let Some(t) = changed_at else {
        return String::new();
    };
    let delta = now_secs - t;
    if delta < 0 {
        return String::new();
    }
    if delta < 3600 {
        return fmt::ago(lang, delta as u64);
    }
    let now = local_dt(now_secs);
    let then = local_dt(t);
    let hhmm = format!("{:02}:{:02}", then.hour(), then.minute());
    let days = (now.date().to_julian_day() - then.date().to_julian_day()) as i64;
    if days == 0 {
        hhmm
    } else if days == 1 {
        since_yesterday(lang, &hhmm)
    } else {
        since_days(lang, days)
    }
}

fn entity_href(id: &str) -> String {
    format!("/entity/{}", crate::api::encode_id(id))
}

#[component]
fn SensorTile(row: SensorRow) -> impl IntoView {
    let class = format!(
        "tile{}{}{}",
        if row.danger_active() {
            " tile--danger"
        } else {
            ""
        },
        if row.intrusion_active() {
            " tile--intrusion"
        } else {
            ""
        },
        if row.available { "" } else { " tile--off" },
    );
    let lang = use_lang();
    let label = {
        let row = row.clone();
        move || row.aria_label(lang())
    };
    let icon = sensor_icon(row.device_class.as_deref());
    let since_map = use_context::<SinceMap>();
    let now = use_context::<Now>();
    let id_for_since = row.id.clone();
    let since = move || match (since_map, now) {
        (Some(SinceMap(m)), Some(now)) => {
            let changed_at = m.with(|m| m.get(&id_for_since).copied().flatten());
            let now_secs = now.0.get() / 1000;
            since_label(lang(), now_secs as i64, changed_at)
        }
        _ => String::new(),
    };
    view! {
        <A href=entity_href(&row.id) attr:class=class attr:aria-label=label>
            <span class="tile__line">
                <span class="tile__icon" inner_html=icon></span>
                <span class="tile__name">{row.name.clone()}</span>
            </span>
            <span class="tile__line">
                <span class="tile__area">
                    {
                        let area = row.area_name.clone();
                        move || {
                            area.clone()
                                .unwrap_or_else(|| {
                                    text(lang(), Key::Unassigned).to_owned()
                                })
                        }
                    }
                </span>
                <span class="tile__state">{let row = row.clone(); move || row.state_word(lang())}</span>
            </span>
            <span class="tile__line security-since">{since}</span>
        </A>
    }
}

#[component]
fn ActionTile(row: ActionRow, confirming: RwSignal<Option<String>>) -> impl IntoView {
    let sender = Sender::from_context();
    let lang = use_lang();
    let id = row.id.clone();
    let tap = row.tap();
    let is_button = row.kind == Kind::Button;
    let verb = {
        let row = row.clone();
        move || row.verb(lang())
    };
    let question = {
        let row = row.clone();
        move || row.question(lang())
    };
    let on = row.available && row.on == Some(true);
    // Only an available button has no state word, in either language.
    let has_state = row.state_word(Lang::En).is_some();
    let tap_label = {
        let row = row.clone();
        move || action_label(&row.name, row.state_word(lang()).as_deref())
    };
    let class = format!(
        "tile tile--action{}{}",
        if on { " tile--active" } else { "" },
        if row.available { "" } else { " tile--off" },
    );
    let command = tap.command.clone();
    let ask = tap.confirm;
    let send_id = id.clone();
    let on_tap = move |_| {
        if ask {
            confirming.set(Some(send_id.clone()));
        } else {
            sender.send(&send_id, command.clone(), None);
        }
    };
    let confirm_command = tap.command;
    let confirm_id = id.clone();
    let is_confirming = {
        let id = id.clone();
        move || confirming.with(|c| c.as_deref() == Some(id.as_str()))
    };
    let normal_class = class.clone();
    let normal = move || {
        let icon = if is_button { PLAY } else { PLUG };
        let name = row.name.clone();
        let href = entity_href(&row.id);
        let area = row.area_name.clone();
        let state_row = row.clone();
        view! {
            <div class=normal_class.clone()>
                <button
                    type="button"
                    class="tile__tap"
                    aria-label=tap_label.clone()
                    disabled=!row.available
                    on:click=on_tap.clone()
                ></button>
                <span class="tile__line">
                    <span class="tile__icon" inner_html=icon></span>
                    <A href=href attr:class="tile__name tile__link">
                        {name}
                    </A>
                </span>
                <span class="tile__line">
                    <span class="tile__area">
                        {move || {
                            area.clone()
                                .unwrap_or_else(|| {
                                    text(lang(), Key::Unassigned).to_owned()
                                })
                        }}
                    </span>
                    {has_state
                        .then(|| {
                            let row = state_row.clone();
                            view! {
                                <span class="tile__state">
                                    {move || row.state_word(lang()).unwrap_or_default()}
                                </span>
                            }
                        })}
                </span>
            </div>
        }
    };
    let confirm = move || {
        let command = confirm_command.clone();
        let id = confirm_id.clone();
        view! {
            <div class="tile tile--action tile--confirm" role="group" aria-label=question.clone()>
                <span class="tile__question">{question.clone()}</span>
                <span class="tile__buttons">
                    <button
                        type="button"
                        class="btn btn--compact"
                        on:click=move |_| {
                            confirming.set(None);
                            sender.send(&id, command.clone(), None);
                        }
                    >
                        {verb.clone()}
                    </button>
                    <button
                        type="button"
                        class="btn btn--compact"
                        on:click=move |_| confirming.set(None)
                    >
                        {move || security_text(lang(), SecurityText::Cancel)}
                    </button>
                </span>
            </div>
        }
    };
    view! { <Show when=is_confirming fallback=normal.clone()>{confirm.clone()}</Show> }
}

#[component]
pub fn SecurityScreen() -> impl IntoView {
    let house = expect_context::<House>();
    let lang = use_lang();
    let t = use_text();
    let ui = Memo::new(move |_| {
        house
            .entities
            .with(|m| house.areas.with(|a| SecurityUi::build(m, a)))
    });

    // Provide the shared `since X` map, populated by one batched /last_change when the set of
    // visible sensor ids changes (not on every UI recompute — reviewer finding #4).
    let since_map = SinceMap(RwSignal::new(BTreeMap::new()));
    provide_context(since_map);
    let visible_ids: Memo<Vec<String>> =
        Memo::new(move |_| ui.with(|u| u.sensors.iter().map(|r| r.id.clone()).collect()));
    Effect::new(move |_| {
        let ids = visible_ids.get();
        if ids.is_empty() {
            return;
        }
        spawn_local(async move {
            let ordered_ids = ids.clone();
            if let Ok(reply) = fetch_last_change(&ordered_ids).await {
                let mut m = BTreeMap::new();
                for (id, item) in ordered_ids.iter().zip(reply.items.iter()) {
                    m.insert(id.clone(), item.changed_at);
                }
                since_map.0.set(m);
            }
            // On error: leave the map as-is; the tiles show no `since` line.
        });
    });
    let loaded = Memo::new(move |_| {
        house.areas.with(|a| !a.is_empty())
            || house.entities.with(|m| !m.is_empty())
            || matches!(house.connection.get(), nidoo_api::ConnectionStatus::Online)
    });

    // The id of the action asking "Turn on X?"; it closes when the entity disappears or
    // becomes unavailable.
    let confirming = RwSignal::new(None::<String>);
    Effect::new(move |_| {
        if let Some(id) = confirming.get() {
            let usable = ui.with(|u| u.actions.iter().any(|a| a.id == id && a.available));
            if !usable {
                confirming.set(None);
            }
        }
    });

    let panels = move || ui.with(|u| u.panels.clone());
    let actions = move || ui.with(|u| u.actions.clone());
    let sections = move || ui.with(|u| u.sections.clone());
    let empty = move || ui.with(|u| u.is_empty());
    let has_sensors = move || ui.with(|u| !u.sensors.is_empty());
    let has_panels = move || ui.with(|u| !u.panels.is_empty());
    let has_actions = move || ui.with(|u| !u.actions.is_empty());

    view! {
        <Show
            when=move || loaded.get()
            fallback=|| {
                view! {
                    <div class="grid" aria-busy="true">
                        <div class="card card--skeleton"></div>
                        <div class="card card--skeleton"></div>
                    </div>
                }
            }
        >
            <section class="security">
                <h1 class="room__title">{move || t(Key::Security)}</h1>
                <Show
                    when=move || !empty()
                    fallback=move || {
                        view! {
                            <p class="room__empty">
                                {move || security_text(lang(), SecurityText::NothingYet)}
                            </p>
                        }
                    }
                >
                    <Show
                        when=has_panels
                        fallback=move || {
                            view! {
                                <p class="card security__note">
                                    {move || security_text(lang(), SecurityText::NoPanelBefore)}
                                    <code>"alarm_control_panel"</code>
                                    {move || security_text(lang(), SecurityText::NoPanelAfter)}
                                </p>
                            }
                        }
                    >
                        <For
                            each=panels
                            key=|p| format!("{p:?}")
                            children=|panel| view! { <AlarmHero panel=panel /> }
                        />
                    </Show>
                    <Show when=has_actions>
                        <h2 class="section-title">
                            {move || security_text(lang(), SecurityText::Actions)}
                        </h2>
                        <div class="tiles">
                            <For
                                each=actions
                                key=|a| format!("{a:?}")
                                children=move |row| {
                                    view! { <ActionTile row=row confirming=confirming /> }
                                }
                            />
                        </div>
                    </Show>
                    <Show
                        when=has_sensors
                        fallback=move || {
                            view! {
                                <p class="security__note">
                                    {move || security_text(lang(), SecurityText::NoSensors)}
                                </p>
                            }
                        }
                    >
                        <p class="summary" aria-live="polite">
                            {move || ui.with(|u| u.summary_text(lang()))}
                        </p>
                        <For
                            each=sections
                            key=|s| format!("{s:?}")
                            children=move |section| {
                                view! {
                                    <h2 class="section-title">
                                        {move || section_title(lang(), section.section)}
                                    </h2>
                                    <div class="tiles">
                                        {section
                                            .rows
                                            .into_iter()
                                            .map(|row| view! { <SensorTile row=row /> })
                                            .collect_view()}
                                    </div>
                                }
                            }
                        />
                    </Show>
                </Show>
                <RecentAlerts />
            </section>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::since_label;
    use crate::i18n::Lang;

    #[test]
    fn since_label_covers_the_natural_language_scale() {
        let now = 1_792_497_600;
        let en = Lang::En;
        assert_eq!(since_label(en, now, None), "");
        assert_eq!(since_label(en, now, Some(now - 30)), "just now");
        assert_eq!(since_label(en, now, Some(now - 180)), "3\u{A0}min ago");
        // Same-day: just HH:MM in local time.
        let same_day = since_label(en, now, Some(now - 3_600));
        assert_eq!(same_day.len(), 5);
        assert!(same_day.chars().nth(2) == Some(':'));
        // Yesterday
        let yesterday = now - 86_400;
        assert!(since_label(en, now, Some(yesterday)).starts_with("yesterday"));
        // Beyond
        assert_eq!(since_label(en, now, Some(now - 3 * 86_400)), "3 days ago");
    }

    #[test]
    fn since_label_in_french() {
        let now = 1_792_497_600;
        let fr = Lang::Fr;
        assert_eq!(since_label(fr, now, Some(now - 30)), "à l'instant");
        assert_eq!(since_label(fr, now, Some(now - 180)), "il y a 3\u{A0}min");
        assert!(since_label(fr, now, Some(now - 86_400)).starts_with("hier"));
        assert_eq!(
            since_label(fr, now, Some(now - 3 * 86_400)),
            "il y a 3 jours"
        );
    }

    #[test]
    fn since_label_ignores_future_timestamps() {
        let now = 1_792_497_600;
        assert_eq!(since_label(Lang::En, now, Some(now + 60)), "");
    }
}
