//! The last run of one ambiance: what each action did, and which devices failed or were skipped.

use chrono::NaiveTime;
use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use nidoo_api::{ActionReport, Ambiance, ApplyReport, Outcome, Problem};

use super::{HouseView, action_sentence};
use crate::house::House;
use crate::i18n::{self, Key, Lang, use_text};
use crate::nav::BackLink;

/// The time of day of `unix` seconds on the browser's clock; UTC off the browser.
fn local_time(unix: u64) -> NaiveTime {
    i18n::format::local_date_time(unix).time()
}

/// The word for an outcome.
fn outcome_key(outcome: Outcome) -> Key {
    match outcome {
        Outcome::Ok => Key::OutcomeDone,
        Outcome::Failed => Key::OutcomeFailed,
        Outcome::Skipped => Key::OutcomeSkipped,
    }
}

/// Each of `problems` as "name: reason", the name being the entity's when the house knows it.
fn problem_lines(house: House, problems: &[Problem]) -> Vec<String> {
    house.entities.with(|entities| {
        problems
            .iter()
            .map(|p| {
                let name = entities
                    .get(&p.id)
                    .map_or(p.id.as_str(), |e| e.name.as_str());
                format!("{name}: {}", p.reason)
            })
            .collect()
    })
}

fn action_rows(
    house: House,
    lang: Lang,
    ambiance: &Ambiance,
    report: &ApplyReport,
    zones: &[(String, String)],
) -> Vec<AnyView> {
    let t = move |k| i18n::text(lang, k);
    let flows: Vec<(String, String)> = house.flows.with(|list| {
        list.iter()
            .map(|f| (f.definition.id.clone(), f.definition.name.clone()))
            .collect()
    });
    let entities = house.entities.get();
    let areas = house.areas.get();
    let view = HouseView {
        entities: &entities,
        areas: &areas,
        flows: &flows,
        zones,
    };
    report
        .actions
        .iter()
        .map(|row: &ActionReport| {
            let sentence = ambiance.actions.get(row.index).map_or_else(
                || format!("#{}", row.index + 1),
                |a| action_sentence(a, &view, lang),
            );
            let failed = problem_lines(house, &row.failed);
            let skipped = problem_lines(house, &row.skipped);
            let class = match row.outcome {
                Outcome::Ok => "report__outcome",
                Outcome::Failed => "report__outcome report__outcome--failed",
                Outcome::Skipped => "report__outcome report__outcome--skipped",
            };
            let reason = row.reason.clone();
            view! {
                <li class="report__action">
                    <span class="report__sentence">{sentence}</span>
                    <span class=class>{t(outcome_key(row.outcome))}</span>
                    {reason.map(|r| view! { <p class="report__reason">{r}</p> })}
                    {(!failed.is_empty()).then(|| view! {
                        <p class="report__heading">{t(Key::OutcomeFailed)}</p>
                        <ul class="report__problems">
                            {failed.into_iter().map(|l| view! { <li>{l}</li> }).collect_view()}
                        </ul>
                    })}
                    {(!skipped.is_empty()).then(|| view! {
                        <p class="report__heading">{t(Key::OutcomeSkipped)}</p>
                        <ul class="report__problems">
                            {skipped.into_iter().map(|l| view! { <li>{l}</li> }).collect_view()}
                        </ul>
                    })}
                </li>
            }
            .into_any()
        })
        .collect()
}

#[component]
pub fn AmbianceReportScreen() -> impl IntoView {
    let t = use_text();
    let house = expect_context::<House>();
    let lang = use_context::<RwSignal<Lang>>().unwrap_or_else(|| RwSignal::new(Lang::En));
    let params = use_params_map();
    let id = Memo::new(move |_| params.read().get("id").unwrap_or_default());
    let ambiance = Memo::new(move |_| {
        let id = id.get();
        house.ambiances.with(|list| {
            list.iter()
                .find(|v| v.id == id)
                .and_then(|v| v.ambiance.clone())
        })
    });
    let report = Memo::new(move |_| house.ambiance_reports.with(|m| m.get(&id.get()).cloned()));

    // Zone names are needed only to word a heating action; fetch them for that case alone.
    let zones = RwSignal::new(Vec::<(String, String)>::new());
    Effect::new(move |done: Option<bool>| {
        let wants = ambiance.with(|a| {
            a.as_ref().is_some_and(|a| {
                a.actions
                    .iter()
                    .any(|x| matches!(x, nidoo_api::AmbianceAction::Climate(c) if c.zone.is_some()))
            })
        });
        if wants && done != Some(true) {
            leptos::task::spawn_local(async move {
                if let Ok(view) = crate::api::climate::get_climate().await {
                    zones.try_set(view.zones.into_iter().map(|z| (z.zone, z.name)).collect());
                }
            });
            return true;
        }
        done.unwrap_or(false)
    });

    view! {
        <div class="sheet ambiances">
            <header class="sheet__header">
                <BackLink />
                <h1 class="sheet__title">
                    {move || {
                        ambiance
                            .get()
                            .map_or_else(|| id.get(), |a| a.name)
                    }}
                </h1>
            </header>
            <section class="sheet__panel report">
                {move || match (report.get(), ambiance.get()) {
                    (Some(report), Some(a)) => {
                        let lang = lang.get();
                        let when = i18n::home_time(lang, local_time(report.started_at));
                        let rows = zones.with(|z| action_rows(house, lang, &a, &report, z));
                        view! {
                            <h2 class="sheet__heading">{move || t(Key::AmbianceLastRun)}" · "{when}</h2>
                            <ul class="report__actions">{rows}</ul>
                        }
                        .into_any()
                    }
                    _ => view! { <p class="report__none">{move || t(Key::AmbianceNoReport)}</p> }.into_any(),
                }}
            </section>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::local_time;
    use chrono::Timelike;

    #[test]
    fn local_time_off_the_browser_is_utc() {
        let t = local_time(1_792_402_860);
        assert_eq!((t.hour(), t.minute()), (9, 41));
    }
}
