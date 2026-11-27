//! The period controls of the history page: unit chips, the `‹ name ›` row with its picker and
//! "Back to now", and the year to compare with. It only edits a `PeriodState`; the screen
//! writes it to the address.

use chrono::{Datelike, NaiveDate};
use leptos::prelude::*;

use crate::components::period_picker::PeriodPicker;
use crate::history_period::{
    PeriodState, Span, Unit, can_step, compare_years, is_current, shown_span, step, switch_unit,
};
use crate::i18n::history::{HistoryText, history_text, period_name, unit_chip};
use crate::i18n::use_lang;

const UNITS: [Unit; 7] = [
    Unit::Hour1,
    Unit::Hour6,
    Unit::Day,
    Unit::Week,
    Unit::Month,
    Unit::Year,
    Unit::Custom,
];

/// The word for "the period you are in", shown beside its name.
fn now_label(unit: Unit) -> Option<HistoryText> {
    match unit {
        Unit::Day => Some(HistoryText::Today),
        Unit::Week => Some(HistoryText::ThisWeek),
        Unit::Month => Some(HistoryText::ThisMonth),
        Unit::Year => Some(HistoryText::ThisYear),
        _ => None,
    }
}

/// The state with the current period shown again (a custom span becomes today).
pub fn back_to_now(s: &PeriodState) -> PeriodState {
    PeriodState {
        unit: if s.unit == Unit::Custom {
            Unit::Day
        } else {
            s.unit
        },
        at: None,
        custom: None,
        vs: s.vs,
        show: s.show.clone(),
    }
}

/// The state set to compare `vs` years back (`None` clears it).
pub fn with_compare(s: &PeriodState, vs: Option<u8>) -> PeriodState {
    PeriodState { vs, ..s.clone() }
}

/// The offsets the compare list offers: the data's, plus the chosen one if it fell outside.
pub fn compare_options(shown: Span, first: Option<NaiveDate>, vs: Option<u8>) -> Vec<u8> {
    let mut years = compare_years(shown, first);
    if let Some(v) = vs
        && !years.contains(&v)
    {
        years.push(v);
    }
    years
}

#[component]
pub fn PeriodBar(
    #[prop(into)] state: Signal<PeriodState>,
    #[prop(into)] today: Signal<NaiveDate>,
    #[prop(into)] first: Signal<Option<NaiveDate>>,
    on_change: Callback<PeriodState>,
    #[prop(into)] compare_allowed: Signal<bool>,
    /// Whether the hub's extent is known; until then the arrows and the comparison are off.
    #[prop(default = Signal::stored(true), into)]
    ready: Signal<bool>,
) -> impl IntoView {
    let lang = use_lang();
    let picker_open = RwSignal::new(false);
    let shown = move || shown_span(&state.get(), today.get());
    let can = move |by: i32| ready.get() && can_step(&state.get(), by, today.get(), first.get());

    let chips = UNITS.iter().map(|u| {
        let u = *u;
        view! {
            <button
                type="button"
                class="chip"
                class:chip--selected=move || state.get().unit == u
                aria-pressed=move || (state.get().unit == u).to_string()
                on:click={move |_| {
                    if u == Unit::Custom {
                        picker_open.update(|o| *o = !*o);
                    } else {
                        picker_open.set(false);
                        on_change.run(switch_unit(&state.get_untracked(), u, today.get_untracked()));
                    }
                }}
            >
                {move || unit_chip(lang(), u)}
            </button>
        }
    }).collect_view();

    let name = move || {
        shown()
            .map(|s| period_name(lang(), state.get().unit, s))
            .unwrap_or_default()
    };
    let stepper = move |by: i32, glyph: &'static str, label: HistoryText| {
        view! {
            <button
                type="button"
                class="period-bar__step"
                aria-label=move || history_text(lang(), label)
                disabled=move || !can(by)
                on:click={move |_| {
                    on_change.run(step(&state.get_untracked(), by, today.get_untracked()));
                }}
            >
                {glyph}
            </button>
        }
    };

    let status = move || {
        let s = state.get();
        if is_current(&s, today.get()) {
            now_label(s.unit).map(|l| {
                view! { <span class="period-bar__now">{move || history_text(lang(), l)}</span> }
                    .into_any()
            })
        } else {
            Some(
                view! {
                    <button
                        type="button"
                        class="period-bar__back"
                        on:click={move |_| on_change.run(back_to_now(&state.get_untracked()))}
                    >
                        {move || history_text(lang(), HistoryText::BackToNow)}
                    </button>
                }
                .into_any(),
            )
        }
    };

    let compare = move || {
        let span = shown()?;
        if !ready.get() || !compare_allowed.get() {
            return None;
        }
        let s = state.get();
        let years = compare_options(span, first.get(), s.vs);
        if years.is_empty() {
            return None;
        }
        let year = span.from.year();
        let options = years
            .into_iter()
            .map(|n| {
                view! {
                    <option value=n.to_string() selected=s.vs == Some(n)>
                        {(year - i32::from(n)).to_string()}
                    </option>
                }
            })
            .collect_view();
        Some(view! {
            <label class="period-bar__compare">
                <span>{move || history_text(lang(), HistoryText::CompareWith)}</span>
                <select on:change:target={move |ev| {
                    let vs = ev.target().value().parse::<u8>().ok();
                    on_change.run(with_compare(&state.get_untracked(), vs));
                }}>
                    <option value="" selected=s.vs.is_none()>
                        {move || history_text(lang(), HistoryText::CompareOff)}
                    </option>
                    {options}
                </select>
            </label>
        })
    };

    let initial = Signal::derive(move || {
        shown().unwrap_or(Span {
            from: today.get(),
            to: today.get(),
        })
    });
    let on_show = Callback::new(move |span: Span| {
        let s = state.get_untracked();
        on_change.run(PeriodState {
            unit: Unit::Custom,
            at: None,
            custom: Some(span),
            vs: s.vs,
            show: s.show,
        });
    });

    view! {
        <div class="period-bar">
            <div class="chips history-detail__chips">{chips}</div>
            <Show when=move || shown().is_some()>
                <div class="period-bar__row">
                    <div class="period-bar__stepper">
                        {stepper(-1, "‹", HistoryText::PreviousPeriod)}
                        <button
                            type="button"
                            class="period-bar__name"
                            aria-haspopup="dialog"
                            aria-expanded=move || picker_open.get().to_string()
                            on:click={move |_| picker_open.update(|o| *o = !*o)}
                        >
                            {name}
                        </button>
                        {stepper(1, "›", HistoryText::NextPeriod)}
                        {status}
                        <PeriodPicker
                            open=picker_open
                            initial=initial
                            today=today
                            first=first
                            on_show=on_show
                        />
                    </div>
                    {compare}
                </div>
            </Show>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn state(unit: Unit) -> PeriodState {
        PeriodState {
            unit,
            at: Some(d(2026, 10, 1)),
            custom: None,
            vs: Some(1),
            show: Some("battery".into()),
        }
    }

    #[test]
    fn back_to_now_keeps_the_unit_the_compare_and_the_values() {
        let s = back_to_now(&state(Unit::Month));
        assert_eq!((s.unit, s.at, s.vs), (Unit::Month, None, Some(1)));
        assert_eq!(s.show.as_deref(), Some("battery"));
    }

    #[test]
    fn back_to_now_from_a_custom_span_is_today() {
        let mut s = state(Unit::Custom);
        s.custom = Some(Span {
            from: d(2026, 1, 1),
            to: d(2026, 2, 1),
        });
        let s = back_to_now(&s);
        assert_eq!((s.unit, s.custom), (Unit::Day, None));
    }

    #[test]
    fn choosing_off_clears_the_compare() {
        assert_eq!(with_compare(&state(Unit::Month), None).vs, None);
        assert_eq!(with_compare(&state(Unit::Month), Some(3)).vs, Some(3));
    }

    #[test]
    fn the_compare_list_follows_the_data_and_keeps_the_chosen_year() {
        let month = Span {
            from: d(2026, 10, 1),
            to: d(2026, 10, 31),
        };
        assert_eq!(compare_options(month, Some(d(2024, 1, 1)), None), [1, 2]);
        assert_eq!(
            compare_options(month, Some(d(2024, 1, 1)), Some(5)),
            [1, 2, 5]
        );
        assert!(compare_options(month, None, None).is_empty());
    }

    #[test]
    fn only_calendar_units_have_a_now_word() {
        assert_eq!(now_label(Unit::Month), Some(HistoryText::ThisMonth));
        assert_eq!(now_label(Unit::Custom), None);
        assert_eq!(now_label(Unit::Hour1), None);
    }
}
