//! The week calendar: seven day columns with the hours going down, the base setpoint as the
//! background and each block as a bar coloured by its temperature. When editable, a block's ends
//! drag in 15-minute steps and a click opens its editor under the calendar (times, setpoint,
//! delete) so editing also works without precise dragging; when viewing, a click shows the
//! block's times under the calendar.
//!
//! The editing functions are pure: they take a block index into `blocks` and return new rules
//! that still satisfy `ModeRules` validation (from < to, within 00:00 to 24:00, no overlap on a
//! shared day).

use leptos::prelude::*;
use nidoo_climate::doc::{Block, Day, HhMm, ModeRules, Setpoint};
use wasm_bindgen::JsCast;

use super::{TempBand, day_short, setpoint_text, temp_band};
use crate::i18n::climate::{ClimateText, use_climate_text};
use crate::i18n::format::{input_number, parse_decimal};
use crate::i18n::{Key, Lang, use_lang, use_text};

/// The grid blocks snap to, in minutes.
const STEP: u16 = 15;
const DAY_END: u16 = 1440;
/// The temperatures a block can hold (`ModeRules` validation accepts 0 to 40).
const MIN_TEMP: f64 = 0.0;
const MAX_TEMP: f64 = 40.0;
/// The base of a new schedule, and of one whose base is switched back on from Off.
pub const NEW_BASE: f64 = 16.0;

/// Which end of a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    From,
    To,
}

/// The block a viewer tapped, by the day and times it was tapped on rather than by its index, so
/// a refetch that reorders or removes blocks never moves the peek to another block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeekKey {
    pub day: Day,
    pub from: HhMm,
    pub to: HhMm,
}

impl PeekKey {
    /// The key of `block` as tapped on `day`.
    pub fn of(block: &Block, day: Day) -> Self {
        Self {
            day,
            from: block.from,
            to: block.to,
        }
    }
}

/// The index of the block `key` names, while one still covers that day at those times.
pub fn peek_index(rules: &ModeRules, key: PeekKey) -> Option<usize> {
    rules
        .blocks
        .iter()
        .position(|b| b.days.contains(&key.day) && b.from == key.from && b.to == key.to)
}

/// The first day of the week `block` covers: its one tab stop.
pub fn first_day(block: &Block) -> Option<Day> {
    Day::ALL.into_iter().find(|d| block.days.contains(d))
}

/// `minutes` rounded to the nearest 15 minutes, at most 24:00.
fn snap(minutes: u16) -> u16 {
    ((minutes.saturating_add(STEP / 2)) / STEP * STEP).min(DAY_END)
}

/// "HH:MM" for minutes since midnight.
fn hhmm_text(minutes: u16) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// One block as a line: "07:15 – 14:00 · 18.5° · 3/5" (the evidence only on a learned block).
pub fn peek_text(lang: Lang, b: &nidoo_climate::doc::Block) -> String {
    let mut out = format!(
        "{} – {} · {}",
        hhmm_text(b.from.0),
        hhmm_text(b.to.0),
        setpoint_text(lang, Some(b.setpoint))
    );
    if let Some(e) = &b.evidence {
        out.push_str(&format!(" · {}/{}", e.agree, e.of));
    }
    out
}

/// Moves one end of `rules.blocks[block]` to `to`, snapped to 15 minutes. The end keeps `from`
/// before `to` and stops at the nearest block it shares a day with. An unknown block, or no
/// room to move, returns the rules unchanged.
pub fn move_edge(rules: &ModeRules, block: usize, edge: Edge, to: HhMm) -> ModeRules {
    let mut out = rules.clone();
    let Some(b) = rules.blocks.get(block) else {
        return out;
    };
    let neighbours = rules
        .blocks
        .iter()
        .enumerate()
        .filter(|(i, o)| *i != block && o.days.iter().any(|d| b.days.contains(d)))
        .map(|(_, o)| o);
    let want = snap(to.0);
    let (lo, hi) = match edge {
        Edge::From => (
            neighbours
                .filter(|o| o.to <= b.from)
                .map(|o| o.to.0)
                .max()
                .unwrap_or(0),
            (b.to.0.saturating_sub(1)) / STEP * STEP,
        ),
        Edge::To => (
            (b.from.0 / STEP + 1) * STEP,
            neighbours
                .filter(|o| o.from >= b.to)
                .map(|o| o.from.0)
                .min()
                .unwrap_or(DAY_END),
        ),
    };
    let at = if lo <= hi {
        want.clamp(lo, hi)
    } else {
        // No grid step fits between the block's own edge and an off-grid neighbour edge: the
        // neighbour's exact edge is the one place the end can still go.
        match edge {
            Edge::From if lo < b.to.0 => lo,
            Edge::To if hi > b.from.0 => hi,
            _ => return out,
        }
    };
    let at = HhMm(at);
    let same = match edge {
        Edge::From => at == b.from,
        Edge::To => at == b.to,
    };
    if same {
        return out;
    }
    out.blocks[block].evidence = None;
    match edge {
        Edge::From => out.blocks[block].from = at,
        Edge::To => out.blocks[block].to = at,
    }
    out
}

/// Sets the setpoint of `rules.blocks[block]`; a temperature is kept within 0 to 40.
pub fn set_setpoint(rules: &ModeRules, block: usize, sp: Setpoint) -> ModeRules {
    let mut out = rules.clone();
    if let Some(b) = out.blocks.get_mut(block) {
        let next = match sp {
            Setpoint::Temp(t) if t.is_finite() => Setpoint::Temp(t.clamp(MIN_TEMP, MAX_TEMP)),
            Setpoint::Temp(_) => return rules.clone(),
            Setpoint::Off => Setpoint::Off,
        };
        if b.setpoint != next {
            b.setpoint = next;
            // The learner's evidence no longer describes this block.
            b.evidence = None;
        }
    }
    out
}

/// Sets the mode's base, the setpoint outside the blocks; a temperature is kept within 0 to 40.
pub fn set_base(rules: &ModeRules, sp: Setpoint) -> ModeRules {
    let mut out = rules.clone();
    out.base = match sp {
        Setpoint::Temp(t) if t.is_finite() => Setpoint::Temp(t.clamp(MIN_TEMP, MAX_TEMP)),
        Setpoint::Temp(_) => return out,
        Setpoint::Off => Setpoint::Off,
    };
    out
}

/// The temperature an Off block goes back to: the mode's base when it is a temperature, else
/// the temperature of the nearest other block in time.
pub fn restore_temp(rules: &ModeRules, block: usize) -> Option<f64> {
    if let Setpoint::Temp(t) = rules.base {
        return Some(t);
    }
    let b = rules.blocks.get(block)?;
    let gap = |o: &nidoo_climate::doc::Block| {
        if o.from >= b.to {
            o.from.0 - b.to.0
        } else {
            b.from.0.saturating_sub(o.to.0)
        }
    };
    rules
        .blocks
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != block)
        .filter_map(|(_, o)| match o.setpoint {
            Setpoint::Temp(t) => Some((gap(o), t)),
            Setpoint::Off => None,
        })
        .min_by_key(|(g, _)| *g)
        .map(|(_, t)| t)
}

/// Removes `rules.blocks[block]`; the other blocks keep their order.
pub fn delete_block(rules: &ModeRules, block: usize) -> ModeRules {
    let mut out = rules.clone();
    if block < out.blocks.len() {
        out.blocks.remove(block);
    }
    out
}

/// Whether `rules.blocks[block]` can also cover `day`: no other block on that day overlaps its
/// times. A day it already covers fits.
pub fn day_fits(rules: &ModeRules, block: usize, day: Day) -> bool {
    let Some(b) = rules.blocks.get(block) else {
        return false;
    };
    b.days.contains(&day)
        || !rules
            .blocks
            .iter()
            .enumerate()
            .any(|(i, o)| i != block && o.days.contains(&day) && b.from < o.to && o.from < b.to)
}

/// Adds `day` to `rules.blocks[block]` when it fits, or removes it when the block has other
/// days. Anything else returns the rules unchanged.
pub fn toggle_day(rules: &ModeRules, block: usize, day: Day) -> ModeRules {
    let mut out = rules.clone();
    let Some(b) = out.blocks.get_mut(block) else {
        return out;
    };
    if b.days.contains(&day) {
        if b.days.len() == 1 {
            return out;
        }
        b.days.retain(|d| *d != day);
    } else if day_fits(rules, block, day) {
        b.days.push(day);
        b.days.sort();
    } else {
        return out;
    }
    b.evidence = None;
    out
}

/// A new block on `day` from the 15-minute step at `minute`, two hours long or up to the next
/// block that day, at the base + 2 °C (19 °C when the base is off). Returns the rules and the
/// new block's index, or `None` when a block on that day already holds `minute`.
pub fn add_block(rules: &ModeRules, day: Day, minute: u16) -> Option<(ModeRules, usize)> {
    const LENGTH: u16 = 120;
    const OFF_BASE_TEMP: f64 = 19.0;
    let from = (minute.min(DAY_END - 1) / STEP) * STEP;
    let same_day = || rules.blocks.iter().filter(|o| o.days.contains(&day));
    if same_day().any(|o| o.from.0 <= from && from < o.to.0) {
        return None;
    }
    let to = same_day()
        .map(|o| o.from.0)
        .filter(|&f| f > from)
        .fold((from + LENGTH).min(DAY_END), u16::min);
    let setpoint = match rules.base {
        Setpoint::Temp(t) => Setpoint::Temp((t + 2.0).clamp(MIN_TEMP, MAX_TEMP)),
        Setpoint::Off => Setpoint::Temp(OFF_BASE_TEMP),
    };
    let mut out = rules.clone();
    out.blocks.push(nidoo_climate::doc::Block {
        days: vec![day],
        from: HhMm(from),
        to: HhMm(to),
        setpoint,
        evidence: None,
    });
    let index = out.blocks.len() - 1;
    Some((out, index))
}

/// The minute at `client_y` down a day column.
fn minute_on(col: &web_sys::Element, client_y: i32) -> Option<u16> {
    let rect = col.get_bounding_client_rect();
    if rect.height() <= 0.0 {
        return None;
    }
    let fraction = ((f64::from(client_y) - rect.top()) / rect.height()).clamp(0.0, 1.0);
    Some((fraction * f64::from(DAY_END)).round() as u16)
}

/// The minute under the pointer down the day column the handle sits in.
fn minute_at(ev: &web_sys::PointerEvent) -> Option<u16> {
    let handle = ev.target()?.dyn_into::<web_sys::Element>().ok()?;
    let col = handle.closest(".climate-timeline__col").ok()??;
    minute_on(&col, ev.client_y())
}

/// A percentage of the day for a style: `30.208`.
fn day_percent(minutes: u16) -> f64 {
    f64::from(minutes) / f64::from(DAY_END) * 100.0
}

fn capture(ev: &web_sys::PointerEvent) {
    if let Some(target) = ev
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
    {
        let _ = target.set_pointer_capture(ev.pointer_id());
    }
}

/// A draggable end of a block. The element stays in the DOM while the rules change, so the
/// pointer capture survives the drag.
#[component]
fn Handle(
    block: usize,
    edge: Edge,
    rules: Signal<ModeRules>,
    on_change: Callback<ModeRules>,
    selected: RwSignal<Option<usize>>,
) -> impl IntoView {
    let dragging = RwSignal::new(false);
    let moved = RwSignal::new(false);
    let t = use_climate_text();
    let label = move || {
        t(match edge {
            Edge::From => ClimateText::DragStart,
            Edge::To => ClimateText::DragEnd,
        })
    };
    view! {
        <span
            class="climate-timeline__handle"
            class:climate-timeline__handle--from=edge == Edge::From
            class:climate-timeline__handle--to=edge == Edge::To
            aria-label=label
            on:pointerdown=move |ev: web_sys::PointerEvent| {
                ev.stop_propagation();
                ev.prevent_default();
                capture(&ev);
                moved.set(false);
                dragging.set(true);
            }
            on:pointermove=move |ev: web_sys::PointerEvent| {
                if !dragging.get_untracked() {
                    return;
                }
                if let Some(minute) = minute_at(&ev) {
                    let current = rules.get_untracked();
                    let next = move_edge(&current, block, edge, HhMm(minute));
                    if next != current {
                        moved.set(true);
                        on_change.run(next);
                    }
                }
            }
            on:pointerup=move |_| {
                // A press without a drag opens the block, so a narrow block is never covered.
                if dragging.get_untracked() && !moved.get_untracked() {
                    selected.set(Some(block));
                }
                dragging.set(false);
            }
            on:pointercancel=move |_| dragging.set(false)
            on:click=|ev| ev.stop_propagation()
        ></span>
    }
}

/// One block in one day column. Built when the block covers the day; its text and position
/// follow the rules in place.
#[component]
fn Bar(
    day: Day,
    block: usize,
    rules: Signal<ModeRules>,
    editable: bool,
    on_change: Callback<ModeRules>,
    selected: RwSignal<Option<usize>>,
    /// The block whose times show under the calendar while viewing.
    peek: RwSignal<Option<PeekKey>>,
    /// The index `peek` names in the current rules.
    peeked: Memo<Option<usize>>,
) -> impl IntoView {
    let lang = use_lang();
    let covers = Memo::new(move |_| {
        rules.with(|r| r.blocks.get(block).is_some_and(|b| b.days.contains(&day)))
    });
    let style = move || {
        rules.with(|r| {
            r.blocks.get(block).map_or_else(String::new, |b| {
                let top = day_percent(b.from.0);
                let height = day_percent(b.to.0.saturating_sub(b.from.0));
                format!("top:{top:.3}%;height:{height:.3}%")
            })
        })
    };
    let text = move || {
        rules.with(|r| {
            r.blocks.get(block).map(|b| {
                let setpoint = setpoint_text(lang(), Some(b.setpoint));
                match &b.evidence {
                    Some(e) => format!("{setpoint} · {}/{}", e.agree, e.of),
                    None => setpoint,
                }
            })
        })
    };
    let times = move || {
        rules.with(|r| {
            r.blocks
                .get(block)
                .map(|b| format!("{} – {}", hhmm_text(b.from.0), hhmm_text(b.to.0)))
        })
    };
    let band = move || rules.with(|r| r.blocks.get(block).map(|b| temp_band(b.setpoint)));
    let title = move || rules.with(|r| r.blocks.get(block).map(|b| peek_text(lang(), b)));
    // One tab stop per block, on the first day it covers; the bars on its other days are the
    // same block.
    let tab_stop = move || {
        let first = rules.with(|r| r.blocks.get(block).and_then(first_day)) == Some(day);
        if first { "0" } else { "-1" }
    };
    let key = move || rules.with(|r| r.blocks.get(block).map(|b| PeekKey::of(b, day)));
    view! {
        <Show when=move || covers.get()>
            <div
                class="climate-timeline__bar"
                class:climate-temp--low=move || band() == Some(TempBand::Low)
                class:climate-temp--mid=move || band() == Some(TempBand::Mid)
                class:climate-temp--high=move || band() == Some(TempBand::High)
                class:climate-temp--off=move || band() == Some(TempBand::Off)
                class:climate-timeline__bar--editable=editable
                class:climate-timeline__bar--peeked=move || peeked.get() == Some(block)
                class:climate-timeline__bar--selected=move || selected.get() == Some(block)
                style=style
                title=title
                role="button"
                tabindex=tab_stop
                on:click=move |_| {
                    if editable {
                        selected.update(|s| *s = if *s == Some(block) { None } else { Some(block) });
                    } else if peeked.get_untracked() == Some(block) {
                        peek.set(None);
                    } else {
                        peek.set(key());
                    }
                }
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    if ev.key().eq_ignore_ascii_case("enter") || ev.key() == " " {
                        ev.prevent_default();
                        if editable {
                            selected.set(Some(block));
                        } else {
                            peek.set(key());
                        }
                    }
                }
            >
                <span class="climate-timeline__temp">{text}</span>
                <span class="climate-timeline__times">{times}</span>
                {editable
                    .then(|| {
                        view! {
                            <Handle block=block edge=Edge::From rules=rules on_change=on_change selected=selected />
                            <Handle block=block edge=Edge::To rules=rules on_change=on_change selected=selected />
                        }
                    })}
            </div>
        </Show>
    }
}

/// The 15-minute choices from `from` to 24:00, plus `current` when it is off the grid (a
/// neighbour's edge), in order.
fn time_choices(current: u16, from: u16) -> Vec<u16> {
    let mut all: Vec<u16> = (from..=DAY_END).step_by(usize::from(STEP)).collect();
    if !all.contains(&current) {
        all.push(current);
        all.sort_unstable();
    }
    all
}

fn time_options(current: u16, from: u16) -> impl IntoView {
    time_choices(current, from)
        .into_iter()
        .map(
            |m| view! { <option value=m.to_string() selected=m == current>{hhmm_text(m)}</option> },
        )
        .collect_view()
}

/// The popover of the selected block: its times, setpoint and delete.
#[component]
fn BlockEditor(
    block: usize,
    rules: Signal<ModeRules>,
    on_change: Callback<ModeRules>,
    selected: RwSignal<Option<usize>>,
) -> impl IntoView {
    let lang = use_lang();
    let t = use_climate_text();
    let text = use_text();
    let setpoint = move || rules.with(|r| r.blocks.get(block).map(|b| b.setpoint));
    // Bumped after each pick so the controls show the real value even when the rules did not
    // change (the move was refused).
    let refresh = RwSignal::new(0u32);
    let edge_at = move |edge: Edge| {
        refresh.track();
        rules.with(|r| {
            r.blocks.get(block).map_or(0, |b| match edge {
                Edge::From => b.from.0,
                Edge::To => b.to.0,
            })
        })
    };
    let pick = move |edge: Edge| {
        move |ev: web_sys::Event| {
            if let Ok(minute) = event_target_value(&ev).parse::<u16>() {
                let current = rules.get_untracked();
                let next = move_edge(&current, block, edge, HhMm(minute));
                if next != current {
                    on_change.run(next);
                }
                refresh.update(|n| *n += 1);
            }
        }
    };
    let is_off = move || {
        refresh.track();
        setpoint() == Some(Setpoint::Off)
    };
    let temp = move || {
        refresh.track();
        match setpoint() {
            Some(Setpoint::Temp(v)) => input_number(lang(), v),
            _ => String::new(),
        }
    };
    // The temperature a block held before Off was ticked, so unticking gives it back.
    let before_off = RwSignal::new(None::<f64>);
    let day_chips = Day::ALL
        .into_iter()
        .map(|day| {
            let on =
                move || rules.with(|r| r.blocks.get(block).is_some_and(|b| b.days.contains(&day)));
            // The last day stays (Delete block removes it); a day another block holds cannot join.
            let locked = move || {
                rules.with(|r| match r.blocks.get(block) {
                    Some(b) if b.days.contains(&day) => b.days.len() == 1,
                    Some(_) => !day_fits(r, block, day),
                    None => true,
                })
            };
            view! {
                <button
                    type="button"
                    class="chip"
                    class:chip--selected=on
                    aria-pressed=move || on().to_string()
                    disabled=locked
                    on:click=move |_| {
                        let current = rules.get_untracked();
                        let next = toggle_day(&current, block, day);
                        if next != current {
                            on_change.run(next);
                        }
                    }
                >
                    {move || day_short(lang(), day)}
                </button>
            }
        })
        .collect_view();
    view! {
        <div class="climate-timeline__editor card">
            <div class="climate-timeline__days" role="group" aria-label=move || t(ClimateText::Days)>
                {day_chips}
            </div>
            <label class="climate-field">
                <span class="climate-field__label">{move || t(ClimateText::From)}</span>
                // A block can start up to one step before its end.
                <select class="field" on:change=pick(Edge::From)>
                    {move || time_options(edge_at(Edge::From), 0)}
                </select>
            </label>
            <label class="climate-field">
                <span class="climate-field__label">{move || t(ClimateText::To)}</span>
                <select class="field" on:change=pick(Edge::To)>
                    {move || time_options(edge_at(Edge::To), STEP)}
                </select>
            </label>
            <label class="climate-field">
                <span class="climate-field__label">{move || t(ClimateText::TemperatureLabel)}</span>
                <input
                    class="field"
                    type="text"
                    inputmode="decimal"
                    prop:value=temp
                    on:change=move |ev| {
                        if let Some(t) = parse_decimal(&event_target_value(&ev)) {
                            let current = rules.get_untracked();
                            let next = set_setpoint(&current, block, Setpoint::Temp(t));
                            if next != current {
                                on_change.run(next);
                            }
                        }
                        // A refused or clamped value shows the block's real one again.
                        refresh.update(|n| *n += 1);
                    }
                />
            </label>
            <label class="climate-field climate-field--inline">
                <input
                    type="checkbox"
                    prop:checked=is_off
                    on:change=move |ev| {
                        let current = rules.get_untracked();
                        let sp = if event_target_checked(&ev) {
                            if let Some(Setpoint::Temp(v)) = current.blocks.get(block).map(|b| b.setpoint) {
                                before_off.set(Some(v));
                            }
                            Some(Setpoint::Off)
                        } else {
                            before_off
                                .get_untracked()
                                .or_else(|| restore_temp(&current, block))
                                .map(Setpoint::Temp)
                        };
                        // With no temperature to go back to, the block stays Off until one is typed.
                        if let Some(sp) = sp {
                            on_change.run(set_setpoint(&current, block, sp));
                        }
                        refresh.update(|n| *n += 1);
                    }
                />
                <span>{move || text(Key::ModeOff)}</span>
            </label>
            <div class="climate-timeline__editor-actions">
                <button
                    type="button"
                    class="btn btn--compact"
                    on:click=move |_| {
                        selected.set(None);
                        on_change.run(delete_block(&rules.get_untracked(), block));
                    }
                >
                    {move || t(ClimateText::DeleteBlock)}
                </button>
                <button type="button" class="btn btn--compact" on:click=move |_| selected.set(None)>
                    {move || t(ClimateText::Done)}
                </button>
            </div>
        </div>
    }
}

/// Seven day columns (Mon to Sun) by 24 hours going down, for one mode's rules. With `now`,
/// today's column is marked and a line sits at the current minute.
#[component]
pub fn WeekTimeline(
    rules: Signal<ModeRules>,
    editable: bool,
    on_change: Callback<ModeRules>,
    #[prop(optional, into)] now: Option<Signal<(Day, u16)>>,
) -> impl IntoView {
    let lang = use_lang();
    let t = use_climate_text();
    let text = use_text();
    let selected = RwSignal::new(None::<usize>);
    let peek = RwSignal::new(None::<PeekKey>);
    let peeked = Memo::new(move |_| {
        peek.get()
            .filter(|_| !editable)
            .and_then(|key| rules.with(|r| peek_index(r, key)))
    });
    let count = move || rules.with(|r| r.blocks.len());
    let base = move || rules.with(|r| setpoint_text(lang(), Some(r.base)));
    let is_today = move |day: Day| now.is_some_and(|n| n.with(|(d, _)| *d == day));
    let headers = Day::ALL
        .into_iter()
        .map(|day| {
            // The first two letters stay on a narrow screen, the rest hides.
            let split = move || {
                let short = day_short(lang(), day);
                let at = short.char_indices().nth(2).map_or(short.len(), |(i, _)| i);
                (&short[..at], &short[at..])
            };
            view! {
                <span
                    class="climate-timeline__dh"
                    class:climate-timeline__dh--today=move || is_today(day)
                >
                    <span class="climate-timeline__dname">{move || split().0}</span>
                    <span class="climate-timeline__dname-rest">{move || split().1}</span>
                </span>
            }
        })
        .collect_view();
    let hours = (0u16..=24)
        .step_by(3)
        .map(|h| {
            view! {
                <span
                    class="climate-timeline__hour"
                    class:climate-timeline__hour--minor=h % 6 != 0
                    style=format!("top:{:.3}%", day_percent(h * 60))
                >
                    {format!("{h:02}")}
                </span>
            }
        })
        .collect_view();
    let cols = Day::ALL
        .into_iter()
        .map(|day| {
            let now_line = move || {
                now.and_then(|n| {
                    let (d, minute) = n.get();
                    (d == day).then(|| {
                        view! {
                            <div
                                class="climate-timeline__now"
                                style=format!("top:{:.3}%", day_percent(minute))
                            ></div>
                        }
                    })
                })
            };
            view! {
                <div
                    class="climate-timeline__col"
                    class:climate-timeline__col--editable=editable
                    class:climate-timeline__col--today=move || is_today(day)
                    on:click=move |ev: web_sys::MouseEvent| {
                        // Only the empty column: a click on a bar opens that bar.
                        if !editable || ev.target() != ev.current_target() {
                            return;
                        }
                        let Some(col) = ev
                            .current_target()
                            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                        else {
                            return;
                        };
                        let Some(minute) = minute_on(&col, ev.client_y()) else {
                            return;
                        };
                        if let Some((next, index)) = add_block(&rules.get_untracked(), day, minute) {
                            on_change.run(next);
                            selected.set(Some(index));
                        }
                    }
                >
                    <For each=move || 0..count() key=|block| *block let:block>
                        <Bar
                            day=day
                            block=block
                            rules=rules
                            editable=editable
                            on_change=on_change
                            selected=selected
                            peek=peek
                            peeked=peeked
                        />
                    </For>
                    {now_line}
                </div>
            }
        })
        .collect_view();
    let peek_line = move || {
        peeked
            .get()
            .and_then(|i| rules.with(|r| r.blocks.get(i).map(|b| peek_text(lang(), b))))
    };
    view! {
        <div class="climate-timeline">
            {if editable {
                let base_temp = move || {
                    rules.with(|r| match r.base {
                        Setpoint::Temp(v) => input_number(lang(), v),
                        Setpoint::Off => String::new(),
                    })
                };
                let base_off = move || rules.with(|r| r.base == Setpoint::Off);
                view! {
                    <div class="climate-timeline__base climate-timeline__base--edit">
                        <label class="climate-field climate-field--inline">
                            <span>{move || t(ClimateText::Base)}</span>
                            <input
                                class="field climate-timeline__base-input"
                                type="text"
                                inputmode="decimal"
                                aria-label=move || t(ClimateText::BaseTemperature)
                                prop:value=base_temp
                                disabled=base_off
                                on:change=move |ev| {
                                    if let Some(v) = parse_decimal(&event_target_value(&ev)) {
                                        on_change.run(set_base(&rules.get_untracked(), Setpoint::Temp(v)));
                                    }
                                }
                            />
                        </label>
                        <label class="climate-field climate-field--inline">
                            <input
                                type="checkbox"
                                prop:checked=base_off
                                on:change=move |ev| {
                                    let sp = if event_target_checked(&ev) {
                                        Setpoint::Off
                                    } else {
                                        Setpoint::Temp(NEW_BASE)
                                    };
                                    on_change.run(set_base(&rules.get_untracked(), sp));
                                }
                            />
                            <span>{move || text(Key::ModeOff)}</span>
                        </label>
                        <span>{move || t(ClimateText::OutsideBlocks)}</span>
                    </div>
                }
                    .into_any()
            } else {
                view! {
                    <p class="climate-timeline__base">
                        {move || t(ClimateText::Base)} " " <strong>{base}</strong> " "
                        {move || t(ClimateText::OutsideBlocks)}
                    </p>
                }
                    .into_any()
            }}
            <div class="climate-timeline__cal">
                <span class="climate-timeline__corner"></span>
                {headers}
                <div class="climate-timeline__hours">{hours}</div>
                {cols}
            </div>
            {move || peek_line().map(|text| view! { <p class="climate-timeline__peek">{text}</p> })}
            {editable
                .then(|| {
                    view! {
                        <p class="climate-timeline__hint">{move || t(ClimateText::TapToAdd)}</p>
                    }
                })}
            {move || {
                selected
                    .get()
                    .filter(|_| editable)
                    .map(|block| {
                        view! {
                            <BlockEditor block=block rules=rules on_change=on_change selected=selected />
                        }
                    })
            }}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_climate::doc::{Block, Rules};

    fn block(days: &[Day], from: &str, to: &str, t: f64) -> Block {
        Block {
            days: days.to_vec(),
            from: HhMm::parse(from).unwrap(),
            to: HhMm::parse(to).unwrap(),
            setpoint: Setpoint::Temp(t),
            evidence: None,
        }
    }

    fn mode(blocks: Vec<Block>) -> ModeRules {
        ModeRules {
            base: Setpoint::Temp(17.0),
            blocks,
        }
    }

    fn valid(m: &ModeRules) {
        Rules {
            heat: Some(m.clone()),
            cool: None,
        }
        .validate()
        .expect("still valid");
    }

    fn at(s: &str) -> HhMm {
        HhMm::parse(s).unwrap()
    }

    #[test]
    fn set_base_clamps_a_temperature_and_keeps_the_blocks() {
        let m = mode(vec![block(&[Day::Mon], "07:00", "09:00", 21.0)]);
        let next = set_base(&m, Setpoint::Temp(15.5));
        assert_eq!(next.base, Setpoint::Temp(15.5));
        assert_eq!(next.blocks, m.blocks);
        assert_eq!(
            set_base(&m, Setpoint::Temp(55.0)).base,
            Setpoint::Temp(40.0)
        );
        assert_eq!(set_base(&m, Setpoint::Off).base, Setpoint::Off);
        assert_eq!(set_base(&m, Setpoint::Temp(f64::NAN)), m);
        valid(&set_base(&m, Setpoint::Off));
    }

    #[test]
    fn peek_index_follows_the_block_not_its_place() {
        let morning = block(&[Day::Mon, Day::Tue], "07:00", "09:00", 21.0);
        let evening = block(&[Day::Mon], "18:00", "22:00", 20.0);
        let key = PeekKey::of(&evening, Day::Mon);
        assert_eq!(
            peek_index(&mode(vec![morning.clone(), evening.clone()]), key),
            Some(1)
        );
        // Reordered: still the evening block.
        assert_eq!(
            peek_index(&mode(vec![evening.clone(), morning.clone()]), key),
            Some(0)
        );
        // Removed, or moved to other times: nothing is peeked.
        assert_eq!(peek_index(&mode(vec![morning.clone()]), key), None);
        let later = block(&[Day::Mon], "19:00", "22:00", 20.0);
        assert_eq!(peek_index(&mode(vec![morning, later]), key), None);
    }

    #[test]
    fn first_day_is_the_earliest_in_the_week() {
        assert_eq!(
            first_day(&block(&[Day::Sun, Day::Wed], "07:00", "09:00", 21.0)),
            Some(Day::Wed)
        );
        assert_eq!(first_day(&block(&[], "07:00", "09:00", 21.0)), None);
    }

    #[test]
    fn peek_text_reads_times_temp_and_evidence() {
        let mut b = block(&[Day::Mon], "07:15", "14:00", 18.5);
        assert_eq!(peek_text(Lang::En, &b), "07:15 – 14:00 · 18.5°");
        b.evidence = Some(nidoo_climate::doc::Evidence { agree: 3, of: 5 });
        assert_eq!(peek_text(Lang::En, &b), "07:15 – 14:00 · 18.5° · 3/5");
        b.evidence = None;
        b.setpoint = Setpoint::Off;
        assert_eq!(peek_text(Lang::En, &b), "07:15 – 14:00 · Off");
        assert_eq!(peek_text(Lang::Fr, &b), "07:15 – 14:00 · Arrêt");
        b.setpoint = Setpoint::Temp(18.5);
        assert_eq!(peek_text(Lang::Fr, &b), "07:15 – 14:00 · 18,5°");
    }

    #[test]
    fn move_edge_snaps_to_15_min() {
        let m = mode(vec![block(&[Day::Mon], "07:00", "09:00", 21.0)]);
        let a = move_edge(&m, 0, Edge::From, at("06:53"));
        assert_eq!(a.blocks[0].from, at("07:00"));
        let b = move_edge(&m, 0, Edge::From, at("06:38"));
        assert_eq!(b.blocks[0].from, at("06:45"));
        let c = move_edge(&m, 0, Edge::To, at("10:07"));
        assert_eq!(c.blocks[0].to, at("10:00"));
        let d = move_edge(&m, 0, Edge::To, at("23:59"));
        assert_eq!(d.blocks[0].to, at("24:00"));
        for r in [a, b, c, d] {
            valid(&r);
        }
    }

    #[test]
    fn move_edge_clamps_to_neighbour() {
        // Block 0 shares Monday with both neighbours; block 3 is on another day.
        let m = mode(vec![
            block(&[Day::Mon, Day::Tue], "09:00", "11:00", 21.0),
            block(&[Day::Mon], "06:00", "08:10", 19.0),
            block(&[Day::Mon], "12:00", "14:00", 19.0),
            block(&[Day::Wed], "08:30", "08:45", 19.0),
        ]);
        let earlier = move_edge(&m, 0, Edge::From, at("05:00"));
        assert_eq!(earlier.blocks[0].from, at("08:10"));
        let later = move_edge(&m, 0, Edge::To, at("13:00"));
        assert_eq!(later.blocks[0].to, at("12:00"));
        // Wednesday has no neighbour that blocks the move.
        let free = move_edge(&m, 3, Edge::To, at("20:00"));
        assert_eq!(free.blocks[3].to, at("20:00"));
        // Block 1 shares Monday with block 0, so its end stops at 09:00.
        let open = move_edge(&m, 1, Edge::To, at("08:55"));
        assert_eq!(open.blocks[1].to, at("09:00"));
        for r in [earlier, later, free, open] {
            valid(&r);
        }
    }

    #[test]
    fn move_edge_keeps_from_before_to() {
        let m = mode(vec![block(&[Day::Mon], "07:00", "09:00", 21.0)]);
        let a = move_edge(&m, 0, Edge::From, at("12:00"));
        assert_eq!(a.blocks[0].from, at("08:45"));
        let b = move_edge(&m, 0, Edge::To, at("03:00"));
        assert_eq!(b.blocks[0].to, at("07:15"));
        let c = move_edge(&m, 0, Edge::From, at("00:00"));
        assert_eq!(c.blocks[0].from, at("00:00"));
        // Unknown block: unchanged.
        assert_eq!(move_edge(&m, 5, Edge::To, at("10:00")), m);
        for r in [a, b, c] {
            valid(&r);
        }
    }

    #[test]
    fn move_edge_reaches_an_off_grid_neighbour_edge() {
        // Block 0 ends at 08:12; the next one starts at 08:14, so no 15-minute step fits.
        let m = mode(vec![
            block(&[Day::Mon], "08:10", "08:12", 21.0),
            block(&[Day::Mon], "08:14", "09:00", 19.0),
        ]);
        let out = move_edge(&m, 0, Edge::To, at("09:00"));
        assert_eq!(out.blocks[0].to, at("08:14"));
        valid(&out);
        // And the other way: block 1 can still go back to 08:12.
        let m = mode(vec![
            block(&[Day::Mon], "07:00", "08:10", 21.0),
            block(&[Day::Mon], "08:12", "08:14", 19.0),
        ]);
        let out = move_edge(&m, 1, Edge::From, at("06:00"));
        assert_eq!(out.blocks[1].from, at("08:10"));
        valid(&out);
    }

    #[test]
    fn editing_a_block_drops_its_evidence_only() {
        let mut m = mode(vec![
            block(&[Day::Mon], "07:00", "09:00", 21.0),
            block(&[Day::Tue], "07:00", "09:00", 21.0),
        ]);
        for b in &mut m.blocks {
            b.evidence = Some(nidoo_climate::doc::Evidence { agree: 17, of: 20 });
        }
        let moved = move_edge(&m, 0, Edge::To, at("10:00"));
        assert_eq!(moved.blocks[0].evidence, None);
        assert!(moved.blocks[1].evidence.is_some());
        // A refused move or an unchanged value keeps it.
        assert_eq!(move_edge(&m, 0, Edge::To, at("09:00")), m);
        assert_eq!(set_setpoint(&m, 0, Setpoint::Temp(21.0)), m);
        let off = set_setpoint(&m, 1, Setpoint::Off);
        assert_eq!(off.blocks[1].evidence, None);
        assert!(off.blocks[0].evidence.is_some());
    }

    #[test]
    fn restore_temp_prefers_base_then_nearest_block() {
        let mut m = mode(vec![
            block(&[Day::Mon], "06:00", "07:00", 19.0),
            block(&[Day::Mon], "08:00", "09:00", 21.0),
            block(&[Day::Mon], "20:00", "22:00", 23.0),
        ]);
        m.blocks[1].setpoint = Setpoint::Off;
        assert_eq!(restore_temp(&m, 1), Some(17.0));
        m.base = Setpoint::Off;
        // 06:00-07:00 is an hour before 08:00; 20:00 is eleven hours after 09:00.
        assert_eq!(restore_temp(&m, 1), Some(19.0));
        m.blocks.truncate(2);
        m.blocks[0].setpoint = Setpoint::Off;
        assert_eq!(restore_temp(&m, 1), None);
    }

    #[test]
    fn time_choices_include_an_off_grid_edge() {
        let c = time_choices(490, 0);
        assert!(c.contains(&490));
        assert!(c.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(time_choices(495, 15).len(), 96);
    }

    #[test]
    fn set_setpoint_off() {
        let m = mode(vec![
            block(&[Day::Mon], "07:00", "09:00", 21.0),
            block(&[Day::Tue], "07:00", "09:00", 21.0),
        ]);
        let off = set_setpoint(&m, 1, Setpoint::Off);
        assert_eq!(off.blocks[1].setpoint, Setpoint::Off);
        assert_eq!(off.blocks[0].setpoint, Setpoint::Temp(21.0));
        let hot = set_setpoint(&m, 0, Setpoint::Temp(99.0));
        assert_eq!(hot.blocks[0].setpoint, Setpoint::Temp(40.0));
        assert_eq!(set_setpoint(&m, 0, Setpoint::Temp(f64::NAN)), m);
        valid(&off);
        valid(&hot);
    }

    #[test]
    fn delete_block_keeps_others() {
        let m = mode(vec![
            block(&[Day::Mon], "07:00", "09:00", 21.0),
            block(&[Day::Tue], "08:00", "10:00", 20.0),
            block(&[Day::Wed], "09:00", "11:00", 19.0),
        ]);
        let out = delete_block(&m, 1);
        assert_eq!(out.blocks, vec![m.blocks[0].clone(), m.blocks[2].clone()]);
        assert_eq!(out.base, m.base);
        assert_eq!(delete_block(&m, 9), m);
        valid(&out);
    }

    #[test]
    fn toggle_day_adds_a_free_day_and_removes_one() {
        let mut m = mode(vec![
            block(&[Day::Mon, Day::Tue, Day::Wed], "00:00", "07:30", 18.0),
            block(&[Day::Sat], "00:00", "07:30", 18.0),
        ]);
        m.blocks[1].evidence = Some(nidoo_climate::doc::Evidence { agree: 1, of: 1 });
        let sunday = toggle_day(&m, 1, Day::Sun);
        assert_eq!(sunday.blocks[1].days, vec![Day::Sat, Day::Sun]);
        assert_eq!(sunday.blocks[1].evidence, None);
        let monday = toggle_day(&m, 1, Day::Mon);
        assert_eq!(monday, m, "Monday already has an overlapping block");
        let tue_off = toggle_day(&m, 0, Day::Tue);
        assert_eq!(tue_off.blocks[0].days, vec![Day::Mon, Day::Wed]);
        // Days stay in week order.
        let fri = toggle_day(&m, 0, Day::Fri);
        assert_eq!(
            fri.blocks[0].days,
            vec![Day::Mon, Day::Tue, Day::Wed, Day::Fri]
        );
        for r in [sunday, tue_off, fri] {
            valid(&r);
        }
    }

    #[test]
    fn toggle_day_keeps_the_last_day() {
        let m = mode(vec![block(&[Day::Sat], "00:00", "07:30", 18.0)]);
        assert_eq!(toggle_day(&m, 0, Day::Sat), m);
        assert_eq!(toggle_day(&m, 4, Day::Sun), m);
    }

    #[test]
    fn day_fits_only_where_no_block_overlaps() {
        let m = mode(vec![
            block(&[Day::Mon], "06:00", "08:00", 19.0),
            block(&[Day::Tue], "07:00", "09:00", 21.0),
            block(&[Day::Wed], "08:00", "10:00", 21.0),
        ]);
        // Block 1 (07:00-09:00) overlaps Monday's 06:00-08:00 and Wednesday's 08:00-10:00.
        assert!(!day_fits(&m, 1, Day::Mon));
        assert!(!day_fits(&m, 1, Day::Wed));
        assert!(day_fits(&m, 1, Day::Thu));
        // Its own day always fits.
        assert!(day_fits(&m, 1, Day::Tue));
        let touching = mode(vec![
            block(&[Day::Mon], "06:00", "07:00", 19.0),
            block(&[Day::Tue], "07:00", "09:00", 21.0),
        ]);
        assert!(day_fits(&touching, 1, Day::Mon));
    }

    #[test]
    fn add_block_starts_on_the_tapped_quarter_for_two_hours() {
        let m = mode(vec![block(&[Day::Sat], "00:00", "07:30", 18.0)]);
        let (out, i) = add_block(&m, Day::Sun, 18 * 60 + 7).expect("free spot");
        assert_eq!(i, 1);
        assert_eq!(out.blocks[0], m.blocks[0]);
        let b = &out.blocks[1];
        assert_eq!(b.days, vec![Day::Sun]);
        assert_eq!((b.from, b.to), (at("18:00"), at("20:00")));
        // Base 17 °C + 2.
        assert_eq!(b.setpoint, Setpoint::Temp(19.0));
        assert_eq!(b.evidence, None);
        valid(&out);
    }

    #[test]
    fn add_block_stops_at_the_next_block_and_the_day_end() {
        let m = mode(vec![block(&[Day::Mon, Day::Tue], "09:00", "11:00", 21.0)]);
        let (short, _) = add_block(&m, Day::Mon, 8 * 60 + 20).unwrap();
        assert_eq!(
            (short.blocks[1].from, short.blocks[1].to),
            (at("08:15"), at("09:00"))
        );
        let (late, _) = add_block(&m, Day::Mon, 23 * 60).unwrap();
        assert_eq!(late.blocks[1].to, at("24:00"));
        let (end, _) = add_block(&m, Day::Mon, 1440).unwrap();
        assert_eq!(
            (end.blocks[1].from, end.blocks[1].to),
            (at("23:45"), at("24:00"))
        );
        // Inside a block on that day: nothing to add.
        assert_eq!(add_block(&m, Day::Tue, 10 * 60), None);
        // The same time on a day the block does not cover is free.
        assert!(add_block(&m, Day::Wed, 10 * 60).is_some());
        for r in [short, late, end] {
            valid(&r);
        }
    }

    #[test]
    fn add_block_temperature_follows_the_base() {
        let mut m = mode(Vec::new());
        m.base = Setpoint::Temp(39.0);
        let (hot, _) = add_block(&m, Day::Mon, 0).unwrap();
        assert_eq!(hot.blocks[0].setpoint, Setpoint::Temp(40.0));
        m.base = Setpoint::Off;
        let (off, _) = add_block(&m, Day::Mon, 0).unwrap();
        assert_eq!(off.blocks[0].setpoint, Setpoint::Temp(19.0));
    }
}
