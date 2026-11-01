//! The `trigger.sun` settings sheet and the helpers the canvas shares for sun cards: the summary
//! line, the next fire times and the "Paris fallback" badge.

use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use chrono_tz::Tz;
use leptos::prelude::*;
use nidoo_api::FlowsConfigView;
use nidoo_core::flow::sun::sun_events;
use nidoo_core::flow::{Node, NodeKind, SunWhich, TriggerSunParams};

use crate::flow_text::sun_summary;
use crate::i18n::flows::{FlowText, sun_name, use_flow_text};
use crate::i18n::format::{date_short, parse_decimal, time, weekday_short};
use crate::i18n::{Lang, use_lang};

/// The house point used when the hub reports no coordinates (matches the hub's fallback).
const PARIS: (f64, f64) = (48.8566, 2.3522);
/// Largest offset the chips offer, in minutes.
const MAX_CHIP_MINUTES: i32 = 720;
/// Minutes a freshly chosen "Before" or "After" chip starts at.
const DEFAULT_OFFSET: i32 = 15;
/// How many days after the current one are searched for the next event.
const SEARCH_DAYS: i64 = 7;

/// The params of a sun trigger for `which` shifted by `offset_minutes`.
pub fn sun_params_from_preset(which: SunWhich, offset_minutes: Option<i32>) -> TriggerSunParams {
    TriggerSunParams {
        which,
        offset_minutes,
        extra: Default::default(),
    }
}

/// The first fire of `which` (shifted by `offset_minutes`) strictly after `now`, from the UTC
/// day before `now` through the next seven; the same search the hub's engine runs.
fn next_fire(
    lat: f64,
    lon: f64,
    zone: Tz,
    which: SunWhich,
    offset_minutes: i32,
    now: DateTime<Utc>,
) -> Option<DateTime<Tz>> {
    let first = now.date_naive();
    let limit = now + Duration::days(SEARCH_DAYS);
    (-1..=SEARCH_DAYS)
        .filter_map(|days| {
            let events = sun_events(lat, lon, first + Duration::days(days));
            let event = match which {
                SunWhich::Sunrise => events.sunrise,
                SunWhich::Sunset => events.sunset,
                SunWhich::Dusk => events.dusk,
                SunWhich::Dawn => events.dawn,
            }?;
            event.checked_add_signed(Duration::minutes(i64::from(offset_minutes)))
        })
        .filter(|at| *at > now && *at <= limit)
        .min()
        .map(|at| at.with_timezone(&zone))
}

/// The next three times the trigger fires, soonest first, in `zone`.
pub fn next_three_fires(
    lat: f64,
    lon: f64,
    zone: Tz,
    which: SunWhich,
    offset_minutes: i32,
    mut now: DateTime<Utc>,
) -> Vec<DateTime<Tz>> {
    let mut out = Vec::new();
    for _ in 0..3 {
        let Some(at) = next_fire(lat, lon, zone, which, offset_minutes, now) else {
            break;
        };
        out.push(at);
        now = at.with_timezone(&Utc) + Duration::seconds(1);
    }
    out
}

/// Whether sun cards carry the "Paris fallback" badge.
pub fn fallback_badge_visible(cfg: &FlowsConfigView) -> bool {
    cfg.fallback_active
}

/// The house point and zone a preview is computed for: the configured ones, Paris otherwise.
pub(crate) fn house_place(cfg: Option<&FlowsConfigView>) -> ((f64, f64), Tz) {
    let zone = cfg
        .and_then(|c| c.timezone.parse::<Tz>().ok())
        .unwrap_or(chrono_tz::Europe::Paris);
    let point = match cfg {
        Some(FlowsConfigView {
            latitude: Some(lat),
            longitude: Some(lon),
            fallback_active: false,
            ..
        }) => (*lat, *lon),
        _ => PARIS,
    };
    (point, zone)
}

fn now_utc() -> DateTime<Utc> {
    Utc.timestamp_millis_opt(js_sys::Date::now() as i64)
        .single()
        .unwrap_or_default()
}

/// The small pill telling that sun flows run against Paris.
#[component]
pub fn FallbackBadge(cfg: Signal<Option<FlowsConfigView>>) -> impl IntoView {
    let t = use_flow_text();
    view! {
        <Show when=move || cfg.with(|c| c.as_ref().is_some_and(fallback_badge_visible))>
            <span class="badge-fallback" title=move || t(FlowText::ParisWarning)>
                {move || t(FlowText::ParisFallback)}
            </span>
        </Show>
    }
}

/// A next fire as "Mon Oct 24, 07:30" / "lun. 24 oct., 07:30", in the house zone.
fn fire_text(lang: Lang, at: DateTime<Tz>) -> String {
    format!(
        "{} {}, {}",
        weekday_short(lang, at.weekday()),
        date_short(lang, at.date_naive()),
        time(at.time())
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Timing {
    At,
    Before,
    After,
}

fn timing_of(offset: i32) -> Timing {
    match offset {
        0 => Timing::At,
        n if n < 0 => Timing::Before,
        _ => Timing::After,
    }
}

/// The stored offset for a timing chip and a minutes count; zero minutes is no offset.
fn offset_for(timing: Timing, minutes: i32) -> Option<i32> {
    match timing {
        Timing::At => None,
        Timing::Before => (minutes != 0).then_some(-minutes),
        Timing::After => (minutes != 0).then_some(minutes),
    }
}

/// The editor of a `trigger.sun` node: event, timing chips, summary and next fire times.
pub fn sun_sheet(node: RwSignal<Node>, cfg: Signal<Option<FlowsConfigView>>) -> impl IntoView {
    let lang = use_lang();
    let t = use_flow_text();
    let params = move || {
        node.with(|n| match &n.kind {
            NodeKind::TriggerSun(p) => p.clone(),
            _ => TriggerSunParams::default(),
        })
    };
    let write = move |which: Option<SunWhich>, offset: Option<Option<i32>>| {
        node.update(|n| {
            if let NodeKind::TriggerSun(ref mut p) = n.kind {
                if let Some(w) = which {
                    p.which = w;
                }
                if let Some(o) = offset {
                    p.offset_minutes = o;
                }
            }
        });
    };
    let which = Memo::new(move |_| params().which);
    let offset = Memo::new(move |_| params().offset());
    let timing = RwSignal::new(timing_of(offset.get_untracked()));
    let minutes = Memo::new(move |_| offset.get().unsigned_abs());

    let set_timing = move |t: Timing| {
        timing.set(t);
        let m = match minutes.get_untracked() {
            0 => DEFAULT_OFFSET,
            m => (m as i32).min(MAX_CHIP_MINUTES),
        };
        write(None, Some(offset_for(t, m)));
    };
    let on_minutes = move |ev: web_sys::Event| {
        let Some(m) = parse_decimal(&event_target_value(&ev)) else {
            return;
        };
        let m = (m as i32).clamp(0, MAX_CHIP_MINUTES);
        let current = timing.get_untracked();
        if current == Timing::At {
            return;
        }
        // Zero minutes is "at the event": the chip follows the stored offset.
        timing.set(timing_of(offset_for(current, m).unwrap_or(0)));
        write(None, Some(offset_for(current, m)));
    };

    let events = [
        SunWhich::Sunrise,
        SunWhich::Sunset,
        SunWhich::Dawn,
        SunWhich::Dusk,
    ];
    let timings = [
        (Timing::At, FlowText::TimingAt),
        (Timing::Before, FlowText::TimingBefore),
        (Timing::After, FlowText::TimingAfter),
    ];
    let fires = Memo::new(move |_| {
        let ((lat, lon), zone) = cfg.with(|c| house_place(c.as_ref()));
        next_three_fires(lat, lon, zone, which.get(), offset.get(), now_utc())
    });

    view! {
        <div class="node-editor__field">
            <span class="node-editor__label">{move || t(FlowText::SolarEvent)}</span>
            <div
                class="segmented"
                role="group"
                aria-label=move || t(FlowText::SolarEvent)
            >
                {events
                    .into_iter()
                    .map(|w| {
                        view! {
                            <button
                                type="button"
                                class="chip segmented__option"
                                class:chip--selected=move || which.get() == w
                                on:click=move |_| write(Some(w), None)
                            >
                                {move || sun_name(lang(), w)}
                            </button>
                        }
                    })
                    .collect_view()}
            </div>
        </div>
        <div class="node-editor__field">
            <span class="node-editor__label">{move || t(FlowText::Timing)}</span>
            <div class="segmented" role="group" aria-label=move || t(FlowText::Timing)>
                {timings
                    .into_iter()
                    .map(|(kind, label)| {
                        view! {
                            <button
                                type="button"
                                class="chip segmented__option"
                                class:chip--selected=move || timing.get() == kind
                                on:click=move |_| set_timing(kind)
                            >
                                {move || t(label)}
                            </button>
                        }
                    })
                    .collect_view()}
            </div>
        </div>
        <Show when=move || timing.get() != Timing::At>
            <label class="node-editor__field">
                <span class="node-editor__label">{move || t(FlowText::Minutes)}</span>
                <input
                    type="number"
                    class="field"
                    min="0"
                    max=MAX_CHIP_MINUTES.to_string()
                    prop:value=move || minutes.get().to_string()
                    on:input=on_minutes
                />
            </label>
        </Show>
        <p class="node-editor__hint">{move || sun_summary(lang(), which.get(), offset.get())}</p>
        <FallbackBadge cfg=cfg />
        <div class="node-editor__field">
            <span class="node-editor__label">{move || t(FlowText::NextFires)}</span>
            <ul class="node-editor__fires">
                {move || {
                    let list = fires.get();
                    if list.is_empty() {
                        return view! { <li>{move || t(FlowText::NoEventWeek)}</li> }.into_any();
                    }
                    let lang = lang();
                    list.into_iter()
                        .map(|at| view! { <li>{fire_text(lang, at)}</li> })
                        .collect_view()
                        .into_any()
                }}
            </ul>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use nidoo_api::FlowsConfigView;
    use nidoo_core::flow::SunWhich;

    use super::*;

    #[test]
    fn preset_sunset_with_minus_30_builds_the_expected_params() {
        let p = sun_params_from_preset(SunWhich::Sunset, Some(-30));
        assert_eq!(p.which, SunWhich::Sunset);
        assert_eq!(p.offset(), -30);
    }

    #[test]
    fn zero_minutes_snaps_the_chip_back_to_the_event() {
        assert_eq!(offset_for(Timing::Before, 0), None);
        assert_eq!(
            timing_of(offset_for(Timing::Before, 0).unwrap_or(0)),
            Timing::At
        );
        assert_eq!(offset_for(Timing::Before, 20), Some(-20));
        assert_eq!(offset_for(Timing::After, 20), Some(20));
        assert_eq!(timing_of(-20), Timing::Before);
    }

    #[test]
    fn next_three_fire_times_derive_from_shared_sun_events() {
        let now = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 6, 0, 0).unwrap();
        let times = next_three_fires(
            45.75,
            4.85,
            chrono_tz::Europe::Paris,
            SunWhich::Sunset,
            0,
            now,
        );
        assert_eq!(times.len(), 3);
        assert!(times[0] < times[1] && times[1] < times[2]);
    }

    #[test]
    fn fires_read_in_both_languages() {
        let at = chrono_tz::Europe::Paris
            .with_ymd_and_hms(2026, 10, 24, 7, 30, 0)
            .unwrap();
        assert_eq!(fire_text(Lang::En, at), "Sat Oct 24, 07:30");
        assert_eq!(fire_text(Lang::Fr, at), "Sam. 24 oct., 07:30");
    }

    #[test]
    fn fallback_badge_tracks_flows_config() {
        let cfg = FlowsConfigView {
            latitude: None,
            longitude: None,
            timezone: "Europe/Paris".into(),
            fallback_active: true,
        };
        assert!(fallback_badge_visible(&cfg));
        let cfg = FlowsConfigView {
            latitude: Some(45.75),
            longitude: Some(4.85),
            timezone: "Europe/Paris".into(),
            fallback_active: false,
        };
        assert!(!fallback_badge_visible(&cfg));
    }
}
