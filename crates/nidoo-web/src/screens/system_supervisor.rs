//! The "Sensor supervisor" card on Settings → System: the switches, the sensitivity, the
//! ignored sensors and the open problems, each with a Mute button.

use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_api::{ProblemView, SupervisorSensitivity, SupervisorSettings, SupervisorView};
use nidoo_core::model::Kind;

use crate::api;
use crate::entity_picker::needs::keep;
use crate::entity_picker::{EntityPicker, Mode};
use crate::house::House;
use crate::i18n::{Lang, SystemText, learning_line, system_text, use_lang, use_system_text};
use crate::screens::alarm_settings::SwitchRow;

/// "2 h" from an hour on, "10 min" below.
fn duration(lang: Lang, secs: u64) -> String {
    if secs >= 3600 {
        format!(
            "{} {}",
            secs / 3600,
            system_text(lang, SystemText::UnitHour)
        )
    } else {
        format!(
            "{} {}",
            secs / 60,
            system_text(lang, SystemText::UnitMinute)
        )
    }
}

/// One open problem: "Tank temperature — stuck for 14 h". The time counts from when the
/// condition started, not from when the supervisor noticed it.
pub fn problem_line(lang: Lang, p: &ProblemView, now: u64) -> String {
    let t = |k| system_text(lang, k);
    let what = match p.check.as_str() {
        "stuck" => t(SystemText::CheckStuck),
        "silent" => t(SystemText::CheckSilent),
        _ => t(SystemText::CheckImpossible),
    };
    let before = p
        .evidence
        .get("for_secs")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let lasted = before + now.saturating_sub(p.opened_at);
    format!(
        "{} — {what} {} {}",
        p.name,
        t(SystemText::For),
        duration(lang, lasted)
    )
}

/// `s` with `id` ignored (once).
pub fn with_ignored(s: &SupervisorSettings, id: &str) -> SupervisorSettings {
    let mut next = s.clone();
    if !next.ignored.iter().any(|i| i == id) {
        next.ignored.push(id.to_owned());
    }
    next
}

/// `s` with `id` supervised again.
pub fn without_ignored(s: &SupervisorSettings, id: &str) -> SupervisorSettings {
    let mut next = s.clone();
    next.ignored.retain(|i| i != id);
    next
}

fn now_secs() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

const SENSITIVITIES: [(SupervisorSensitivity, SystemText); 3] = [
    (SupervisorSensitivity::Low, SystemText::SensitivityLow),
    (SupervisorSensitivity::Normal, SystemText::SensitivityNormal),
    (SupervisorSensitivity::High, SystemText::SensitivityHigh),
];

/// The card. Loads on mount; every change is saved at once, shown before the hub answers and
/// rolled back if it refuses.
#[component]
pub fn SupervisorCard() -> impl IntoView {
    let lang = use_lang();
    let t = use_system_text();
    let house = expect_context::<House>();
    let data = RwSignal::new(None::<SupervisorView>);
    let saving = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let picking = RwSignal::new(false);

    spawn_local(async move {
        match api::get_supervisor().await {
            Ok(v) => {
                data.try_set(Some(v));
            }
            Err(e) => {
                error.try_set(Some(e.error));
            }
        }
    });

    let save = Callback::new(move |next: SupervisorSettings| {
        if saving.get_untracked() {
            return;
        }
        let Some(before) = data.get_untracked() else {
            return;
        };
        data.update(|v| {
            if let Some(v) = v {
                v.settings = next.clone();
            }
        });
        saving.set(true);
        error.set(None);
        spawn_local(async move {
            match api::set_supervisor(&next).await {
                Ok(v) => {
                    data.try_set(Some(v));
                }
                Err(e) => {
                    data.try_set(Some(before));
                    error.try_set(Some(e.error));
                }
            }
            saving.try_set(false);
        });
    });
    let settings =
        move || data.with(|v| v.as_ref().map(|v| v.settings.clone()).unwrap_or_default());
    let switch = move |get: fn(&SupervisorSettings) -> bool,
                       set: fn(&mut SupervisorSettings, bool)| {
        Callback::new(move |on: bool| {
            let mut next = settings();
            if get(&next) != on {
                set(&mut next, on);
                save.run(next);
            }
        })
    };
    let name_of = move |id: &str| {
        house
            .entities
            .with_untracked(|m| m.get(id).map(|e| e.name.clone()))
            .unwrap_or_else(|| id.to_owned())
    };

    view! {
        <section class="system__supervisor">
            <h2 class="system__heading">{move || t(SystemText::SupervisorTitle)}</h2>
            <p class="system__hint">{move || t(SystemText::SupervisorHint)}</p>
            {move || {
                data.with(|v| v.as_ref().is_some_and(|v| !v.available))
                    .then(|| {
                        view! {
                            <p class="home-settings__error">{move || t(SystemText::NeedsDatabase)}</p>
                        }
                    })
            }}
            <SwitchRow
                label=Signal::derive(move || t(SystemText::Supervise).to_owned())
                on=Signal::derive(move || settings().enabled)
                on_toggle=switch(|s| s.enabled, |s, on| s.enabled = on)
                busy=saving
            />
            <SwitchRow
                label=Signal::derive(move || t(SystemText::NotifyMe).to_owned())
                on=Signal::derive(move || settings().notify)
                on_toggle=switch(|s| s.notify, |s, on| s.notify = on)
                busy=saving
            />
            <SwitchRow
                label=Signal::derive(move || t(SystemText::SupervisorRecovered).to_owned())
                on=Signal::derive(move || settings().notify_recovered)
                on_toggle=switch(|s| s.notify_recovered, |s, on| s.notify_recovered = on)
                busy=saving
            />
            <p class="system__label">{move || t(SystemText::Sensitivity)}</p>
            <div class="chips system__chips" role="group">
                {SENSITIVITIES
                    .iter()
                    .map(|&(level, word)| {
                        let selected = move || settings().sensitivity == level;
                        view! {
                            <button
                                type="button"
                                class="chip"
                                class:chip--selected=selected
                                aria-pressed=move || selected().to_string()
                                disabled=move || saving.get()
                                on:click=move |_| {
                                    let mut next = settings();
                                    if next.sensitivity != level {
                                        next.sensitivity = level;
                                        save.run(next);
                                    }
                                }
                            >
                                {move || t(word)}
                            </button>
                        }
                    })
                    .collect_view()}
            </div>

            <h3 class="system__subheading">{move || t(SystemText::OpenProblems)}</h3>
            {move || {
                let l = lang();
                let now = now_secs();
                let problems = data.with(|v| v.as_ref().map(|v| v.problems.clone()).unwrap_or_default());
                if problems.is_empty() {
                    view! { <p class="system__hint">{t(SystemText::NoProblems)}</p> }.into_any()
                } else {
                    view! {
                        <ul class="system__problems">
                            {problems
                                .into_iter()
                                .map(|p| {
                                    let id = p.entity_id.clone();
                                    view! {
                                        <li class="system__problem">
                                            <span>{problem_line(l, &p, now)}</span>
                                            <button
                                                type="button"
                                                class="chip"
                                                disabled=move || saving.get()
                                                on:click=move |_| save.run(with_ignored(&settings(), &id))
                                            >
                                                {t(SystemText::Mute)}
                                            </button>
                                        </li>
                                    }
                                })
                                .collect_view()}
                        </ul>
                    }
                        .into_any()
                }
            }}
            {move || {
                let n = data.with(|v| v.as_ref().map_or(0, |v| v.learning));
                (n > 0).then(|| view! { <p class="system__hint">{learning_line(lang(), n)}</p> })
            }}

            <h3 class="system__subheading">{move || t(SystemText::IgnoredSensors)}</h3>
            <div class="chips system__chips">
                {move || {
                    settings()
                        .ignored
                        .into_iter()
                        .map(|id| {
                            let name = name_of(&id);
                            let label = format!("{} {name}", t(SystemText::Unignore));
                            view! {
                                <span class="chip chip--selected">
                                    {name}
                                    <button
                                        type="button"
                                        class="label-chip__remove"
                                        aria-label=label
                                        disabled=move || saving.get()
                                        on:click=move |_| save.run(without_ignored(&settings(), &id))
                                    >
                                        "✕"
                                    </button>
                                </span>
                            }
                        })
                        .collect_view()
                }}
                <button
                    type="button"
                    class="chip chip--add"
                    aria-haspopup="dialog"
                    disabled=move || saving.get()
                    on:click=move |_| picking.set(true)
                >
                    {move || t(SystemText::AddSensor)}
                </button>
            </div>
            <EntityPicker
                open=picking
                title=Signal::derive(move || t(SystemText::IgnoredSensors).to_owned())
                mode=Mode::Single
                current=Signal::derive(move || settings().ignored)
                pool=Some(keep(|e| e.kind == Kind::Sensor))
                on_pick=Callback::new(move |id: Option<String>| {
                    if let Some(id) = id {
                        save.run(with_ignored(&settings(), &id));
                    }
                })
            />
            {move || error.get().map(|e| view! { <p class="home-settings__error">{e}</p> })}
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn problem(check: &str, name: &str, for_secs: u64, opened_at: u64) -> ProblemView {
        ProblemView {
            entity_id: "sensor.x".into(),
            name: name.into(),
            check: check.into(),
            opened_at,
            evidence: json!({"value": 15.0, "unit": "°C", "for_secs": for_secs, "usual_secs": 10800}),
        }
    }

    #[test]
    fn problem_lines_say_what_and_how_long() {
        const NOW: u64 = 1_800_000_000;
        let p = problem("stuck", "Tank temperature", 14 * 3600, NOW);
        assert_eq!(
            problem_line(Lang::En, &p, NOW),
            "Tank temperature — stuck for 14 h"
        );
        let p = problem("stuck", "Température ballon", 8 * 3600, NOW - 6 * 3600);
        assert_eq!(
            problem_line(Lang::Fr, &p, NOW),
            "Température ballon — bloquée depuis 14 h"
        );
        let p = problem("silent", "Garage", 7 * 3600, NOW);
        assert_eq!(problem_line(Lang::En, &p, NOW), "Garage — silent for 7 h");
        let p = problem("impossible", "Bathroom", 0, NOW - 2 * 3600);
        assert_eq!(
            problem_line(Lang::En, &p, NOW),
            "Bathroom — impossible value for 2 h"
        );
        // Under an hour reads as minutes.
        let p = problem("silent", "Garage", 0, NOW - 600);
        assert_eq!(
            problem_line(Lang::Fr, &p, NOW),
            "Garage — muette depuis 10 min"
        );
    }

    #[test]
    fn learning_line_counts_sensors() {
        assert_eq!(
            learning_line(Lang::En, 3),
            "Learning: 3 sensors have less than 7 days of history"
        );
        assert_eq!(
            learning_line(Lang::En, 1),
            "Learning: 1 sensor has less than 7 days of history"
        );
        assert_eq!(
            learning_line(Lang::Fr, 3),
            "Apprentissage : 3 capteurs ont moins de 7 jours d'historique"
        );
        assert_eq!(
            learning_line(Lang::Fr, 1),
            "Apprentissage : 1 capteur a moins de 7 jours d'historique"
        );
    }

    #[test]
    fn muting_appends_once() {
        let s = SupervisorSettings::default();
        let once = with_ignored(&s, "sensor.a");
        assert_eq!(once.ignored, vec!["sensor.a".to_string()]);
        assert_eq!(with_ignored(&once, "sensor.a").ignored, once.ignored);
        assert!(without_ignored(&once, "sensor.a").ignored.is_empty());
    }
}
