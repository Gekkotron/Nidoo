//! The custom-period popover of the history page: a from and a to date, a few quick picks, and
//! Show / Cancel. A plain popover next to the period name, not a native dialog.

use chrono::NaiveDate;
use leptos::prelude::*;

use crate::history_period::{Span, all_data, last_12_months, this_winter};
use crate::i18n::history::{HistoryText, history_text};
use crate::i18n::use_lang;

fn parse(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// The span two date inputs describe, when both are set and in order.
pub fn picked_span(from: &str, to: &str) -> Option<Span> {
    let (from, to) = (parse(from)?, parse(to)?);
    (from <= to).then_some(Span { from, to })
}

#[component]
pub fn PeriodPicker(
    /// Whether the popover is showing; the picker closes itself on Show and Cancel.
    open: RwSignal<bool>,
    /// The span the inputs start from each time the popover opens.
    #[prop(into)]
    initial: Signal<Span>,
    #[prop(into)] today: Signal<NaiveDate>,
    #[prop(into)] first: Signal<Option<NaiveDate>>,
    on_show: Callback<Span>,
) -> impl IntoView {
    let lang = use_lang();
    let from = RwSignal::new(String::new());
    let to = RwSignal::new(String::new());
    Effect::new(move |_| {
        if open.get() {
            let s = initial.get_untracked();
            from.set(s.from.to_string());
            to.set(s.to.to_string());
        }
    });
    let set_span = move |s: Span| {
        from.set(s.from.to_string());
        to.set(s.to.to_string());
    };
    let quick = move |label: HistoryText, span: Signal<Option<Span>>| {
        view! {
            <Show when=move || span.get().is_some()>
                <button
                    type="button"
                    class="chip chip--sm"
                    on:click={move |_| {
                        if let Some(s) = span.get_untracked() {
                            set_span(s);
                        }
                    }}
                >
                    {move || history_text(lang(), label)}
                </button>
            </Show>
        }
    };
    let winter = Signal::derive(move || Some(this_winter(today.get())));
    let year = Signal::derive(move || Some(last_12_months(today.get())));
    let everything = Signal::derive(move || first.get().map(|f| all_data(f, today.get())));
    let picked = move || picked_span(&from.get(), &to.get());
    view! {
        <Show when=move || open.get()>
            <div class="period-pop" role="dialog" aria-label=move || history_text(lang(), HistoryText::PickTitle)>
                <h2 class="period-pop__title">{move || history_text(lang(), HistoryText::PickTitle)}</h2>
                <label class="period-pop__field">
                    <span>{move || history_text(lang(), HistoryText::PickFrom)}</span>
                    <input
                        type="date"
                        prop:value=move || from.get()
                        min=move || first.get().map(|d| d.to_string())
                        max=move || today.get().to_string()
                        on:input:target={move |ev| from.set(ev.target().value())}
                    />
                </label>
                <label class="period-pop__field">
                    <span>{move || history_text(lang(), HistoryText::PickTo)}</span>
                    <input
                        type="date"
                        prop:value=move || to.get()
                        min=move || first.get().map(|d| d.to_string())
                        max=move || today.get().to_string()
                        on:input:target={move |ev| to.set(ev.target().value())}
                    />
                </label>
                <div class="period-pop__quick">
                    {quick(HistoryText::ThisWinter, winter)}
                    {quick(HistoryText::Last12Months, year)}
                    {quick(HistoryText::AllData, everything)}
                </div>
                <div class="period-pop__actions">
                    <button type="button" class="chip chip--sm" on:click={move |_| open.set(false)}>
                        {move || history_text(lang(), HistoryText::PickCancel)}
                    </button>
                    <button
                        type="button"
                        class="chip chip--sm chip--selected period-pop__show"
                        disabled=move || picked().is_none()
                        on:click={move |_| {
                            if let Some(s) = picked() {
                                open.set(false);
                                on_show.run(s);
                            }
                        }}
                    >
                        {move || history_text(lang(), HistoryText::PickShow)}
                    </button>
                </div>
            </div>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn two_ordered_dates_make_a_span() {
        assert_eq!(
            picked_span("2026-11-01", "2026-11-26"),
            Some(Span {
                from: d(2026, 11, 1),
                to: d(2026, 11, 26)
            })
        );
        assert!(picked_span("2026-11-01", "2026-11-01").is_some());
    }

    #[test]
    fn unset_or_reversed_dates_make_none() {
        assert_eq!(picked_span("", "2026-11-26"), None);
        assert_eq!(picked_span("2026-11-27", "2026-11-26"), None);
        assert_eq!(picked_span("junk", "2026-11-26"), None);
    }
}
