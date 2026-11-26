//! The Climate screen's parts: the zone card, the settings sheet and the pure text helpers.

pub mod correction_card;
pub mod eco_pill;
pub mod entity_field;
pub mod heaters_card;
pub mod home;
pub mod presence_card;
pub mod proposal;
pub mod room_card;
pub mod room_list;
pub mod room_panel;
pub mod room_settings;
pub mod settings_sheet;
pub mod target_stepper;
pub mod timeline;

use chrono::{DateTime, Datelike, Timelike};
use chrono_tz::Tz;
use leptos::prelude::{Callable, Callback};
use leptos::task::spawn_local;
use nidoo_api::ApiError;
use nidoo_api::EntityView;
use nidoo_climate::doc::{
    Day, Decision, FeedbackSettings, Mode, Rules, Setpoint, Source, ZoneState,
};
use nidoo_climate::view::{DecisionRow, LearnStatus, ZoneView};
use nidoo_core::model::Kind;

use crate::climate::heaters_card::name_ids;
use crate::components::toast::Toasts;
use crate::i18n::Lang;
use crate::i18n::climate::{
    ClimateText, climate_text, correction_summary_line, learning_progress, reason_text,
};
use crate::i18n::format::{ago, decimal, month_short, weekday_short};
use crate::i18n::states::{StateCtx, state_word};

/// What the learner's state says, in one line.
pub fn learn_status_text(lang: Lang, status: &LearnStatus) -> String {
    let word = |k| climate_text(lang, k).to_owned();
    match status {
        LearnStatus::Off => word(ClimateText::LearnOff),
        LearnStatus::HistoryOff => word(ClimateText::LearnHistoryOff),
        LearnStatus::NotEnough { days, needed } => learning_progress(lang, *days, *needed),
        LearnStatus::UpToDate => word(ClimateText::LearnUpToDate),
        LearnStatus::Proposed => word(ClimateText::LearnProposed),
        LearnStatus::Rejected => word(ClimateText::LearnRejected),
        LearnStatus::Eco => word(ClimateText::LearnEco),
    }
}

/// How old a reading is: "just now", "10 min ago", "2 h ago", "il y a 3 j".
pub fn age_text(lang: Lang, secs: u64) -> String {
    ago(lang, secs)
}

/// A temperature without a trailing ".0": `21°`, `20.5°` (`20,5°` in French).
pub fn degrees_text(lang: Lang, t: f64) -> String {
    format!("{}°", short_decimal(lang, t))
}

/// A number with one decimal and no trailing ".0": `21`, `20.5` (`20,5` in French).
fn short_decimal(lang: Lang, v: f64) -> String {
    let s = decimal(lang, v, 1);
    s.strip_suffix(".0")
        .or_else(|| s.strip_suffix(",0"))
        .map(str::to_owned)
        .unwrap_or(s)
}

pub fn setpoint_text(lang: Lang, sp: Option<Setpoint>) -> String {
    match sp {
        Some(Setpoint::Temp(t)) => degrees_text(lang, t),
        Some(Setpoint::Off) => state_word(lang, StateCtx::Hvac, "off"),
        None => "—".into(),
    }
}

/// The short name of `day`: "Mon" / "Lun.".
pub fn day_short(lang: Lang, day: Day) -> &'static str {
    use chrono::Weekday as W;
    weekday_short(
        lang,
        match day {
            Day::Mon => W::Mon,
            Day::Tue => W::Tue,
            Day::Wed => W::Wed,
            Day::Thu => W::Thu,
            Day::Fri => W::Fri,
            Day::Sat => W::Sat,
            Day::Sun => W::Sun,
        },
    )
}

pub fn mode_word(lang: Lang, mode: Option<Mode>) -> String {
    match mode {
        Some(Mode::Heat) => state_word(lang, StateCtx::Hvac, "heat"),
        Some(Mode::Cool) => state_word(lang, StateCtx::Hvac, "cool"),
        None => climate_text(lang, ClimateText::Idle).to_owned(),
    }
}

pub fn source_word(lang: Lang, source: Source) -> &'static str {
    climate_text(
        lang,
        match source {
            Source::Safety => ClimateText::SrcSafety,
            Source::Override => ClimateText::SrcOverride,
            Source::Boost => ClimateText::SrcBoost,
            Source::Holiday => ClimateText::SrcHoliday,
            Source::Away => ClimateText::SrcAway,
            Source::Eco => ClimateText::SrcEco,
            Source::Empty => ClimateText::SrcEmpty,
            Source::Schedule => ClimateText::SrcSchedule,
            Source::Idle => ClimateText::Idle,
        },
    )
}

/// The word for the source of an older history row, which the hub sends as a raw string.
pub fn history_source(lang: Lang, raw: &str) -> String {
    let known = match raw {
        "safety" => Some(Source::Safety),
        "override" => Some(Source::Override),
        "boost" => Some(Source::Boost),
        "holiday" => Some(Source::Holiday),
        "away" => Some(Source::Away),
        "eco" => Some(Source::Eco),
        "empty" => Some(Source::Empty),
        "schedule" => Some(Source::Schedule),
        "idle" => Some(Source::Idle),
        _ => None,
    };
    known.map_or_else(
        || crate::format::word(raw),
        |s| source_word(lang, s).to_owned(),
    )
}

/// The house zone from its IANA name; UTC when the name is unknown.
pub fn parse_zone(name: &str) -> Tz {
    name.parse().unwrap_or(Tz::UTC)
}

fn local(unix: u64, tz: Tz) -> Option<DateTime<Tz>> {
    DateTime::from_timestamp(i64::try_from(unix).ok()?, 0).map(|t| t.with_timezone(&tz))
}

/// Unix seconds as "HH:MM" in `tz`.
pub fn clock_text(unix: u64, tz: Tz) -> String {
    local(unix, tz).map_or_else(
        || "—".into(),
        |t| format!("{:02}:{:02}", t.hour(), t.minute()),
    )
}

/// When a hold ends, in `tz`: "07:00" today, "tomorrow 07:00", else "Mon 07:00".
pub fn until_text(lang: Lang, until: u64, now: u64, tz: Tz) -> String {
    let (Some(end), Some(start)) = (local(until, tz), local(now, tz)) else {
        return "—".into();
    };
    let clock = format!("{:02}:{:02}", end.hour(), end.minute());
    match (end.date_naive() - start.date_naive()).num_days() {
        0 => clock,
        1 => format!("{} {clock}", climate_text(lang, ClimateText::Tomorrow)),
        _ => {
            let day = day_short(lang, Day::from_chrono(end.weekday()));
            // Mid-sentence in French ("jusqu'à jeu. 07:00").
            let day = match lang {
                Lang::Fr => day.to_lowercase(),
                Lang::En => day.to_owned(),
            };
            format!("{day} {clock}")
        }
    }
}

/// When a decision was made, in `tz`: "14:05" today, else with its date ("Tue 3 Nov 14:05").
pub fn stamp_text(lang: Lang, at: u64, now: u64, tz: Tz) -> String {
    match (local(at, tz), local(now, tz)) {
        (Some(a), Some(n)) if a.date_naive() == n.date_naive() => clock_text(at, tz),
        _ => eco_end_text(lang, at, tz),
    }
}

/// When a room's eco ends, in `tz`: "Tue 3 Nov 19:00" / "mar. 3 nov. 19:00". The date is always
/// there, since an eco can end days or weeks away.
pub fn eco_end_text(lang: Lang, until: u64, tz: Tz) -> String {
    let Some(end) = local(until, tz) else {
        return "—".into();
    };
    let day = day_short(lang, Day::from_chrono(end.weekday()));
    // Mid-sentence in French ("jusqu'au mar. 3 nov. 19:00").
    let day = match lang {
        Lang::Fr => day.to_lowercase(),
        Lang::En => day.to_owned(),
    };
    format!(
        "{day} {} {} {:02}:{:02}",
        end.day(),
        month_short(lang, end.month()),
        end.hour(),
        end.minute()
    )
}

/// The fan modes the house's climate units list, in first-seen order with how many units list
/// each, and how many climate units there are (groups left out).
pub fn fan_mode_choices<'a>(
    entities: impl IntoIterator<Item = &'a EntityView>,
) -> (Vec<(String, usize)>, usize) {
    let mut choices: Vec<(String, usize)> = Vec::new();
    let mut units = 0;
    for entity in entities {
        if entity.kind != Kind::Climate || entity.id.starts_with("group_") {
            continue;
        }
        units += 1;
        for mode in &entity.capabilities.fan_modes {
            match choices.iter_mut().find(|(m, _)| m == mode) {
                Some((_, n)) => *n += 1,
                None => choices.push((mode.clone(), 1)),
            }
        }
    }
    (choices, units)
}

/// A local date (`YYYY-MM-DD`) and time (`HH:MM`, empty = midnight) in `tz` as Unix seconds. A
/// time the clocks skip (spring forward) is moved to the first valid minute after it.
pub fn eco_until(date: &str, time: &str, tz: Tz) -> Option<u64> {
    let date = chrono::NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d").ok()?;
    let time = match time.trim() {
        "" => chrono::NaiveTime::MIN,
        t => chrono::NaiveTime::parse_from_str(t, "%H:%M").ok()?,
    };
    let mut at = date.and_time(time);
    // A spring-forward gap is at most a few hours; step until the clock exists.
    for _ in 0..=240 {
        if let Some(t) = at.and_local_timezone(tz).earliest() {
            return u64::try_from(t.timestamp()).ok();
        }
        at += chrono::Duration::minutes(1);
    }
    None
}

/// Whether the zone is in eco now: its own eco until a date, or the house's while nobody is home.
pub fn in_eco(z: &ZoneView, now: u64) -> bool {
    z.eco.as_ref().is_some_and(|e| e.until > now)
        || z.decision
            .as_ref()
            .is_some_and(|d| matches!(d.source, Source::Eco | Source::Away))
}

/// The weekday and minute of the day at `now` in `tz`.
pub fn week_minute(now: u64, tz: Tz) -> (Day, u16) {
    local(now, tz).map_or((Day::Mon, 0), |t| {
        (
            Day::from_chrono(t.weekday()),
            (t.hour() * 60 + t.minute()) as u16,
        )
    })
}

/// One decision as a list line: "14:05 · Schedule · Heat 21° · reason", dated when not today.
/// The reason names the room's devices by their `labels` (id, label).
pub fn decision_line(
    lang: Lang,
    d: &Decision,
    now: u64,
    tz: Tz,
    labels: &[(String, String)],
) -> String {
    format!(
        "{} · {} · {} {} · {}",
        stamp_text(lang, d.at, now, tz),
        source_word(lang, d.source),
        mode_word(lang, d.mode),
        setpoint_text(lang, d.setpoint),
        name_ids(&reason_text(lang, &d.reason), labels)
    )
}

/// An older history row as a list line, dated when not today; its reason names the room's
/// devices by their `labels`.
pub fn row_line(
    lang: Lang,
    r: &DecisionRow,
    now: u64,
    tz: Tz,
    labels: &[(String, String)],
) -> String {
    let setpoint = r
        .setpoint
        .map_or_else(|| "—".to_owned(), |t| degrees_text(lang, t));
    format!(
        "{} · {} · {} {} · {}",
        stamp_text(lang, r.at, now, tz),
        history_source(lang, &r.source),
        state_word(lang, StateCtx::Hvac, &r.mode),
        setpoint,
        name_ids(&reason_text(lang, &r.reason), labels)
    )
}

/// Whether a zone in `state` offers the hold controls: only an active zone follows overrides,
/// and the hub refuses one anywhere else.
pub fn can_hold(state: ZoneState) -> bool {
    state == ZoneState::Active
}

/// How warm a setpoint is, for the block and room-dot colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TempBand {
    Low,
    Mid,
    High,
    Off,
}

impl TempBand {
    /// The CSS class that paints the band.
    pub fn class(self) -> &'static str {
        match self {
            TempBand::Low => "climate-temp--low",
            TempBand::Mid => "climate-temp--mid",
            TempBand::High => "climate-temp--high",
            TempBand::Off => "climate-temp--off",
        }
    }
}

/// Below 17 °C is low, below 20 °C mid, else high.
pub fn temp_band(sp: Setpoint) -> TempBand {
    match sp {
        Setpoint::Temp(t) if t < 17.0 => TempBand::Low,
        Setpoint::Temp(t) if t < 20.0 => TempBand::Mid,
        Setpoint::Temp(_) => TempBand::High,
        Setpoint::Off => TempBand::Off,
    }
}

/// The band of the zone's decided temperature; none without one (a neutral dot).
pub fn room_band(z: &ZoneView) -> Option<TempBand> {
    match z.decision.as_ref()?.setpoint? {
        Setpoint::Temp(t) => Some(temp_band(Setpoint::Temp(t))),
        Setpoint::Off => None,
    }
}

/// The room list's line under a zone's name: "Heat 16° · Active", "Pending · no schedule".
pub fn room_subline(lang: Lang, z: &ZoneView) -> String {
    let word = |k| climate_text(lang, k);
    if z.orphan {
        return word(ClimateText::NoClimateDevice).into();
    }
    match z.settings.state {
        ZoneState::Off => word(ClimateText::ZoneOff).into(),
        ZoneState::Pending if z.settings.rules.is_empty() => {
            word(ClimateText::PendingNoSchedule).into()
        }
        ZoneState::Pending => word(ClimateText::ZonePending).into(),
        ZoneState::Active => {
            let line = z.decision.as_ref().map_or_else(
                || word(ClimateText::ActiveNoDecision).to_owned(),
                |d| {
                    format!(
                        "{} {} · {}",
                        mode_word(lang, d.mode),
                        setpoint_text(lang, d.setpoint),
                        word(ClimateText::ZoneActive)
                    )
                },
            );
            if z.overrides.is_empty() {
                line
            } else {
                format!("{line} · {}", word(ClimateText::Held))
            }
        }
    }
}

/// The zone to show: `wanted` while it exists, else the first one.
pub fn pick_zone(ids: &[String], wanted: Option<&str>) -> Option<String> {
    wanted
        .and_then(|w| ids.iter().find(|id| *id == w))
        .or_else(|| ids.first())
        .cloned()
}

/// Minutes from `(today, minute)` until the schedule of `mode` next changes level: the nearest
/// block start or end within the coming week. `None` without blocks for the mode.
pub fn minutes_to_next_change(rules: &Rules, mode: Mode, today: Day, minute: u16) -> Option<u32> {
    let blocks = &rules.get(mode)?.blocks;
    let today_ix = Day::ALL.iter().position(|d| *d == today)? as u32;
    let now = u32::from(minute);
    let mut best: Option<u32> = None;
    for offset in 0..=7u32 {
        let day = Day::ALL[((today_ix + offset) % 7) as usize];
        for block in blocks.iter().filter(|b| b.days.contains(&day)) {
            for edge in [block.from.0, block.to.0] {
                let at = offset * 1440 + u32::from(edge);
                if at > now {
                    let delta = at - now;
                    best = Some(best.map_or(delta, |b| b.min(delta)));
                }
            }
        }
    }
    best
}

/// Runs `work`, then refreshes the screen; a failure becomes a toast.
pub(crate) fn act<F>(toasts: Toasts, on_changed: Callback<()>, work: F)
where
    F: std::future::Future<Output = Result<(), ApiError>> + 'static,
{
    spawn_local(async move {
        match work.await {
            Ok(()) => on_changed.run(()),
            Err(e) => toasts.push(e.error),
        }
    });
}

/// How far one − / + press moves a room's target.
pub const TARGET_STEP: f64 = 0.5;

/// `current + delta` on the half-degree grid, kept within 0–40 °C.
pub fn step_target(current: f64, delta: f64) -> f64 {
    ((current + delta) * 2.0).round().clamp(0.0, 80.0) / 2.0
}

/// How long a − / + hold lasts: until the next schedule change, else two hours.
pub fn hold_minutes(next_change: Option<u32>) -> u32 {
    next_change.unwrap_or(120)
}

/// The target the − / + start from: the decided temperature, else the rounded reading.
pub fn start_target(z: &ZoneView) -> Option<f64> {
    match z.decision.as_ref().and_then(|d| d.setpoint) {
        Some(Setpoint::Temp(t)) => Some(t),
        _ => z.reading.map(f64::round),
    }
}

/// The correction's main numbers on one line, for under its switch.
pub fn correction_summary(lang: Lang, f: &FeedbackSettings) -> String {
    correction_summary_line(
        lang,
        &short_decimal(lang, f.step),
        &short_decimal(lang, f.band),
        f.interval_min,
    )
}

#[cfg(test)]
mod tests;
