//! The Climate screen's parts: the zone card, the settings sheet and the pure text helpers.

pub mod correction_card;
pub mod eco_pill;
pub mod entity_field;
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

use crate::components::toast::Toasts;
use crate::i18n::Lang;
use crate::i18n::climate::{ClimateText, climate_text, correction_summary_line, learning_progress};
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
pub fn decision_line(lang: Lang, d: &Decision, now: u64, tz: Tz) -> String {
    format!(
        "{} · {} · {} {} · {}",
        stamp_text(lang, d.at, now, tz),
        source_word(lang, d.source),
        mode_word(lang, d.mode),
        setpoint_text(lang, d.setpoint),
        d.reason
    )
}

/// An older history row as a list line, dated when not today.
pub fn row_line(lang: Lang, r: &DecisionRow, now: u64, tz: Tz) -> String {
    let setpoint = r
        .setpoint
        .map_or_else(|| "—".to_owned(), |t| degrees_text(lang, t));
    format!(
        "{} · {} · {} {} · {}",
        stamp_text(lang, r.at, now, tz),
        history_source(lang, &r.source),
        state_word(lang, StateCtx::Hvac, &r.mode),
        setpoint,
        r.reason
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
mod tests {
    use super::*;
    use nidoo_climate::doc::{Override, OverrideOrigin, ZoneSettings};

    fn zone_view(state: ZoneState) -> ZoneView {
        ZoneView {
            eco: None,
            zone: "z".into(),
            name: "Z".into(),
            orphan: false,
            settings: ZoneSettings {
                state,
                ..ZoneSettings::default()
            },
            settings_invalid: None,
            entities: vec![],
            reading: None,
            reading_age_s: None,
            room_sensor: None,
            thermometer: None,
            correction_step_at: None,
            presence: None,
            decision: None,
            overrides: vec![],
            proposal: None,
            learn: LearnStatus::Off,
        }
    }

    fn decided(mut z: ZoneView, setpoint: Setpoint) -> ZoneView {
        z.decision = Some(Decision {
            eco_restore: Default::default(),
            adjust: None,
            zone: z.zone.clone(),
            mode: Some(Mode::Heat),
            setpoint: Some(setpoint),
            source: Source::Schedule,
            cause: None,
            reason: String::new(),
            reading: None,
            reading_age_s: None,
            outdoor_mean: None,
            home: true,
            holiday: false,
            applied: true,
            at: 0,
            entities: Default::default(),
        });
        z
    }

    #[test]
    fn temp_band_cuts_at_17_and_20() {
        assert_eq!(temp_band(Setpoint::Temp(16.5)), TempBand::Low);
        assert_eq!(temp_band(Setpoint::Temp(17.0)), TempBand::Mid);
        assert_eq!(temp_band(Setpoint::Temp(19.5)), TempBand::Mid);
        assert_eq!(temp_band(Setpoint::Temp(20.0)), TempBand::High);
        assert_eq!(temp_band(Setpoint::Off), TempBand::Off);
        assert_eq!(TempBand::Mid.class(), "climate-temp--mid");
    }

    #[test]
    fn room_subline_reads_each_state() {
        let en = Lang::En;
        let mut orphan = zone_view(ZoneState::Active);
        orphan.orphan = true;
        assert_eq!(room_subline(en, &orphan), "No climate device");
        assert_eq!(room_subline(en, &zone_view(ZoneState::Off)), "Off");
        let pending = zone_view(ZoneState::Pending);
        assert_eq!(room_subline(en, &pending), "Pending · no schedule");
        let mut scheduled = pending.clone();
        scheduled.settings.rules = Rules {
            heat: Some(ModeRules {
                base: Setpoint::Temp(16.0),
                blocks: vec![],
            }),
            cool: None,
        };
        assert_eq!(room_subline(en, &scheduled), "Pending");
        assert_eq!(
            room_subline(en, &zone_view(ZoneState::Active)),
            "Active · no decision yet"
        );
        let active = decided(zone_view(ZoneState::Active), Setpoint::Temp(16.0));
        assert_eq!(room_subline(en, &active), "Heat 16° · Active");
        let mut held = active.clone();
        held.overrides.push(Override {
            entity: None,
            setpoint: Some(Setpoint::Temp(19.5)),
            mode: None,
            until: 0,
            origin: OverrideOrigin::Hand,
        });
        assert_eq!(room_subline(en, &held), "Heat 16° · Active · held");
    }

    #[test]
    fn room_band_follows_the_decision_setpoint() {
        let z = zone_view(ZoneState::Active);
        assert_eq!(room_band(&z), None);
        assert_eq!(
            room_band(&decided(z.clone(), Setpoint::Temp(18.5))),
            Some(TempBand::Mid)
        );
        assert_eq!(room_band(&decided(z, Setpoint::Off)), None);
    }

    #[test]
    fn pick_zone_keeps_a_known_choice_and_falls_back() {
        let ids = vec!["a".to_owned(), "b".to_owned()];
        assert_eq!(pick_zone(&ids, Some("b")).as_deref(), Some("b"));
        assert_eq!(pick_zone(&ids, Some("gone")).as_deref(), Some("a"));
        assert_eq!(pick_zone(&ids, None).as_deref(), Some("a"));
        assert_eq!(pick_zone(&[], Some("a")), None);
    }

    #[test]
    fn only_an_active_zone_can_hold() {
        assert!(can_hold(ZoneState::Active));
        assert!(!can_hold(ZoneState::Pending));
        assert!(!can_hold(ZoneState::Off));
    }
    use nidoo_climate::doc::{Block, HhMm, ModeRules};

    #[test]
    fn learn_status_text_reads_each_state() {
        assert_eq!(
            learn_status_text(
                Lang::En,
                &LearnStatus::NotEnough {
                    days: 6,
                    needed: 10
                }
            ),
            "Learning: 6 of 10 days so far"
        );
        assert_eq!(
            learn_status_text(Lang::En, &LearnStatus::HistoryOff),
            "History is off: rules can't be learned"
        );
        assert_eq!(
            learn_status_text(Lang::En, &LearnStatus::Proposed),
            "New rules to review"
        );
        assert_eq!(
            learn_status_text(Lang::En, &LearnStatus::UpToDate),
            "Rules up to date"
        );
        assert_eq!(
            learn_status_text(Lang::En, &LearnStatus::Eco),
            "Learning paused while in eco"
        );
    }

    #[test]
    fn age_text_picks_the_unit() {
        let en = Lang::En;
        assert_eq!(age_text(en, 45), "just now");
        assert_eq!(age_text(en, 600), "10\u{A0}min ago");
        assert_eq!(age_text(en, 7200), "2\u{A0}h ago");
        assert_eq!(age_text(en, 3 * 86_400), "3\u{A0}d ago");
    }

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> u64 {
        use chrono::TimeZone;
        chrono::Utc
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .unwrap()
            .timestamp() as u64
    }

    #[test]
    fn clock_and_week_follow_the_house_zone() {
        let paris = parse_zone("Europe/Paris");
        // Monday 2026-01-05 23:30 UTC is Tuesday 00:30 in Paris (UTC+1).
        let now = at(2026, 1, 5, 23, 30);
        assert_eq!(clock_text(now, paris), "00:30");
        assert_eq!(clock_text(now, Tz::UTC), "23:30");
        assert_eq!(week_minute(now, paris), (Day::Tue, 30));
        assert_eq!(week_minute(now, Tz::UTC), (Day::Mon, 23 * 60 + 30));
        assert_eq!(parse_zone("nowhere"), Tz::UTC);
    }

    #[test]
    fn until_text_names_the_day_when_not_today() {
        let paris = parse_zone("Europe/Paris");
        let now = at(2026, 1, 5, 10, 0); // Monday 11:00 in Paris
        assert_eq!(
            until_text(Lang::En, at(2026, 1, 5, 12, 0), now, paris),
            "13:00"
        );
        assert_eq!(
            until_text(Lang::En, at(2026, 1, 6, 6, 0), now, paris),
            "tomorrow 07:00"
        );
        assert_eq!(
            until_text(Lang::En, at(2026, 1, 8, 6, 0), now, paris),
            "Thu 07:00"
        );
        // 23:30 UTC is already tomorrow in Paris.
        assert_eq!(
            until_text(Lang::En, at(2026, 1, 5, 23, 30), now, paris),
            "tomorrow 00:30"
        );
    }

    #[test]
    fn eco_end_names_the_day_and_month() {
        let paris = parse_zone("Europe/Paris");
        // 18:00 UTC is 19:00 in Paris on Tuesday 3 November 2026.
        let until = at(2026, 11, 3, 18, 0);
        assert_eq!(eco_end_text(Lang::En, until, paris), "Tue 3 Nov 19:00");
        assert_eq!(eco_end_text(Lang::Fr, until, paris), "mar. 3 nov. 19:00");
    }

    #[test]
    fn degrees_drop_a_trailing_zero() {
        assert_eq!(degrees_text(Lang::En, 21.0), "21°");
        assert_eq!(degrees_text(Lang::En, 20.5), "20.5°");
        assert_eq!(setpoint_text(Lang::En, Some(Setpoint::Off)), "Off");
    }

    fn rules(blocks: Vec<Block>) -> Rules {
        Rules {
            heat: Some(ModeRules {
                base: Setpoint::Temp(17.0),
                blocks,
            }),
            cool: None,
        }
    }

    fn block(days: &[Day], from: &str, to: &str) -> Block {
        Block {
            days: days.to_vec(),
            from: HhMm::parse(from).unwrap(),
            to: HhMm::parse(to).unwrap(),
            setpoint: Setpoint::Temp(21.0),
            evidence: None,
        }
    }

    #[test]
    fn french_room_lines_and_ages() {
        let fr = Lang::Fr;
        let active = decided(zone_view(ZoneState::Active), Setpoint::Temp(16.5));
        assert_eq!(room_subline(fr, &active), "Chauffage 16,5° · Active");
        let mut held = active.clone();
        held.overrides.push(Override {
            entity: None,
            setpoint: Some(Setpoint::Temp(19.5)),
            mode: None,
            until: 0,
            origin: OverrideOrigin::Hand,
        });
        assert_eq!(
            room_subline(fr, &held),
            "Chauffage 16,5° · Active · dérogation en cours"
        );
        assert_eq!(
            room_subline(fr, &zone_view(ZoneState::Pending)),
            "En attente · aucune programmation"
        );
        assert_eq!(age_text(fr, 45), "à l'instant");
        assert_eq!(age_text(fr, 600), "il y a 10\u{A0}min");
        assert_eq!(degrees_text(fr, 20.5), "20,5°");
        assert_eq!(setpoint_text(fr, Some(Setpoint::Off)), "Arrêt");
        assert_eq!(mode_word(fr, None), "Au repos");
        assert_eq!(source_word(fr, Source::Schedule), "Programmation");
        assert_eq!(
            learn_status_text(fr, &LearnStatus::UpToDate),
            "Règles à jour"
        );
    }

    #[test]
    fn french_until_names_the_day() {
        let paris = parse_zone("Europe/Paris");
        let now = at(2026, 1, 5, 10, 0);
        assert_eq!(
            until_text(Lang::Fr, at(2026, 1, 6, 6, 0), now, paris),
            "demain 07:00"
        );
        assert_eq!(
            until_text(Lang::Fr, at(2026, 1, 8, 6, 0), now, paris),
            "jeu. 07:00"
        );
    }

    #[test]
    fn stamps_are_dated_unless_today() {
        let paris = parse_zone("Europe/Paris");
        let now = at(2026, 1, 5, 10, 0); // Monday 11:00 in Paris
        assert_eq!(
            stamp_text(Lang::En, at(2026, 1, 5, 7, 5), now, paris),
            "08:05"
        );
        // 23:30 UTC on Sunday is already Monday in Paris.
        assert_eq!(
            stamp_text(Lang::En, at(2026, 1, 4, 23, 30), now, paris),
            "00:30"
        );
        assert_eq!(
            stamp_text(Lang::En, at(2026, 1, 4, 22, 30), now, paris),
            "Sun 4 Jan 23:30"
        );
        assert_eq!(
            stamp_text(Lang::Fr, at(2026, 1, 4, 22, 30), now, paris),
            "dim. 4 janv. 23:30"
        );
    }

    #[test]
    fn history_rows_use_the_known_words() {
        assert_eq!(history_source(Lang::Fr, "schedule"), "Programmation");
        assert_eq!(history_source(Lang::Fr, "override"), "Dérogation");
        assert_eq!(history_source(Lang::En, "boost"), "Boost");
        assert_eq!(history_source(Lang::Fr, "boost"), "Boost");
        assert_eq!(source_word(Lang::En, Source::Boost), "Boost");
        assert_eq!(history_source(Lang::En, "away"), "Away");
        assert_eq!(history_source(Lang::En, "eco"), "Eco");
        assert_eq!(history_source(Lang::Fr, "eco"), "Éco");
        assert_eq!(history_source(Lang::En, "empty"), "Empty room");
        assert_eq!(history_source(Lang::Fr, "empty"), "Pièce vide");
        assert_eq!(history_source(Lang::Fr, "mystery_source"), "Mystery source");
    }

    #[test]
    fn next_change_is_the_nearest_block_edge() {
        let r = rules(vec![block(&[Day::Mon, Day::Tue], "07:00", "09:00")]);
        // Monday 06:00: the block starts in an hour.
        assert_eq!(
            minutes_to_next_change(&r, Mode::Heat, Day::Mon, 360),
            Some(60)
        );
        // Monday 08:00: it ends in an hour.
        assert_eq!(
            minutes_to_next_change(&r, Mode::Heat, Day::Mon, 480),
            Some(60)
        );
        // Monday 10:00: next is Tuesday 07:00.
        assert_eq!(
            minutes_to_next_change(&r, Mode::Heat, Day::Mon, 600),
            Some(1260)
        );
        // Wednesday: wraps to next Monday 07:00.
        assert_eq!(
            minutes_to_next_change(&r, Mode::Heat, Day::Wed, 0),
            Some(5 * 1440 + 420)
        );
        assert_eq!(minutes_to_next_change(&r, Mode::Cool, Day::Mon, 0), None);
        assert_eq!(
            minutes_to_next_change(&rules(vec![]), Mode::Heat, Day::Mon, 0),
            None
        );
    }

    fn climate_entity(id: &str, fans: &[&str]) -> nidoo_api::EntityView {
        nidoo_api::EntityView {
            id: id.into(),
            device_id: None,
            name: id.into(),
            device_name: None,
            label: None,
            kind: nidoo_core::model::Kind::Climate,
            component: "climate".into(),
            protocol_kind: nidoo_core::model::Kind::Climate,
            device_class: None,
            unit: None,
            icon: None,
            category: nidoo_core::model::EntityCategory::Primary,
            enabled_by_default: true,
            area: "a".into(),
            part_of: None,
            capabilities: nidoo_core::model::Capabilities {
                fan_modes: fans.iter().map(|f| f.to_string()).collect(),
                ..Default::default()
            },
            ui: Default::default(),
            state: Default::default(),
        }
    }

    #[test]
    fn fan_mode_choices_unions_and_counts() {
        let a = climate_entity("a", &["auto", "low", "high"]);
        let b = climate_entity("b", &["auto", "quiet"]);
        let (choices, units) = fan_mode_choices([&a, &b]);
        assert_eq!(units, 2);
        assert_eq!(
            choices,
            vec![
                ("auto".to_owned(), 2),
                ("low".to_owned(), 1),
                ("high".to_owned(), 1),
                ("quiet".to_owned(), 1)
            ]
        );
    }

    #[test]
    fn fan_mode_choices_skips_groups_and_non_climate() {
        let group = climate_entity("group_all", &["low"]);
        let mut sensor = climate_entity("s", &["low"]);
        sensor.kind = nidoo_core::model::Kind::Sensor;
        let none = climate_entity("c", &[]);
        let (choices, units) = fan_mode_choices([&group, &sensor, &none]);
        assert!(choices.is_empty());
        assert_eq!(
            units, 1,
            "a climate without fan control still counts as a unit"
        );
    }

    #[test]
    fn eco_until_local_and_dst_gap() {
        let paris = parse_zone("Europe/Paris");
        // 00:00 on 2026-11-01 in Paris (CET) is 23:00 UTC the day before.
        assert_eq!(eco_until("2026-11-01", "00:00", paris), Some(1_793_487_600));
        // 02:30 does not exist on 2026-03-29: the clocks jump to 03:00 (01:00 UTC).
        assert_eq!(eco_until("2026-03-29", "02:30", paris), Some(1_774_746_000));
        assert_eq!(eco_until("2026-11-01", "", paris), Some(1_793_487_600));
        assert_eq!(eco_until("nope", "00:00", paris), None);
    }

    #[test]
    fn in_eco_for_room_and_away() {
        let mut z = zone_view(ZoneState::Active);
        assert!(!in_eco(&z, 100));
        z.eco = Some(nidoo_climate::doc::RoomEco {
            until: 200,
            set_at: 0,
        });
        assert!(in_eco(&z, 100));
        assert!(!in_eco(&z, 200), "ended");
        z.eco = None;
        let mut z = decided(z, Setpoint::Temp(16.0));
        z.decision.as_mut().unwrap().source = Source::Away;
        assert!(in_eco(&z, 100));
    }

    #[test]
    fn step_target_rounds_and_clamps() {
        assert_eq!(step_target(19.0, TARGET_STEP), 19.5);
        assert_eq!(step_target(19.3, TARGET_STEP), 20.0);
        assert_eq!(step_target(0.2, -TARGET_STEP), 0.0);
        assert_eq!(step_target(39.8, TARGET_STEP), 40.0);
    }

    #[test]
    fn hold_minutes_defaults_to_two_hours() {
        assert_eq!(hold_minutes(Some(45)), 45);
        assert_eq!(hold_minutes(None), 120);
    }

    #[test]
    fn start_target_prefers_the_decision() {
        let mut read = zone_view(ZoneState::Active);
        read.reading = Some(19.6);
        assert_eq!(
            start_target(&decided(read.clone(), Setpoint::Temp(18.5))),
            Some(18.5)
        );
        assert_eq!(start_target(&read), Some(20.0));
        assert_eq!(start_target(&zone_view(ZoneState::Active)), None);
    }

    #[test]
    fn correction_summary_reads_in_both_languages() {
        let f = nidoo_climate::doc::FeedbackSettings {
            step: 0.5,
            band: 0.5,
            interval_min: 15,
            ..Default::default()
        };
        assert_eq!(
            correction_summary(Lang::En, &f),
            "Step 0.5 °C · tolerance 0.5 °C · every 15 min"
        );
        assert_eq!(
            correction_summary(Lang::Fr, &f),
            "Pas 0,5 °C · écart 0,5 °C · toutes les 15 min"
        );
    }
}
