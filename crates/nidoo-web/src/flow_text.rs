//! Sentences for flow cards: "Turn off LED", "Entrée > 16 lx?". Pure, tested natively.

use std::collections::BTreeMap;

use nidoo_api::{AreaView, EntityView};
use nidoo_core::command::command_name;
use nidoo_core::flow::{
    AlarmModeFilter, FlowLabelCatalog, Node, NodeKind, OffPeakEdge, Op, RuleOp, SunWhich,
    command_with_number, fields, parse_duration, template,
};
use nidoo_core::model::{Command, Hs, Kind};
use nidoo_core::vocabulary::Reports;
use serde_json::Value;

use crate::flows::{humanise, value_text};
use crate::format::{entity_title, shown_as_light};
use crate::i18n::Lang;
use crate::i18n::flows::{self as t, FlowText, flow_text};
use crate::i18n::format::{number, with_unit};
use crate::i18n::states::{StateCtx, state_word};
use crate::schedule::describe;

/// The card's sentence: "At sunset", "15 min after sunrise", "15 min before civil dusk".
pub fn sun_summary(lang: Lang, which: SunWhich, offset_minutes: i32) -> String {
    t::sun_sentence(lang, which, offset_minutes)
}

/// What a card does in a flow; drives its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Trigger,
    Check,
    Action,
    Wait,
}

/// The role of a node type.
pub fn role(kind: &NodeKind) -> Role {
    match kind {
        NodeKind::TriggerState(_)
        | NodeKind::TriggerSchedule(_)
        | NodeKind::TriggerSun(_)
        | NodeKind::TriggerWakeUp(_)
        | NodeKind::TriggerManual(_)
        | NodeKind::TriggerSignal(_)
        | NodeKind::TriggerOffPeak(_)
        | NodeKind::TriggerAlarm(_)
        | NodeKind::TriggerSilent(_) => Role::Trigger,
        NodeKind::If(_) | NodeKind::Switch(_) | NodeKind::TimeBetween(_) => Role::Check,
        NodeKind::Command(_)
        | NodeKind::FlowEnable(_)
        | NodeKind::FlowLabel(_)
        | NodeKind::BackupRun(_)
        | NodeKind::ClimateHold(_)
        | NodeKind::ClimateEco(_)
        | NodeKind::Notify(_)
        | NodeKind::Speak(_)
        | NodeKind::Ask(_)
        | NodeKind::Math(_) => Role::Action,
        NodeKind::Delay(_) | NodeKind::Limit(_) => Role::Wait,
    }
}

/// A catalog with no labels, for sentences that never name one.
pub static NO_LABELS: FlowLabelCatalog = FlowLabelCatalog {
    labels: Vec::new(),
    choices: Vec::new(),
    extra: BTreeMap::new(),
};

/// The house a sentence is read against.
pub struct Ctx<'a> {
    pub entities: &'a BTreeMap<String, EntityView>,
    pub areas: &'a [AreaView],
    /// `(id, name)` of every flow.
    pub flows: &'a [(String, String)],
    /// The flow labels and choices, for the label cards.
    pub labels: &'a FlowLabelCatalog,
    /// The UI language of the sentences.
    pub lang: Lang,
}

fn entity_text(id: &str, ctx: &Ctx) -> String {
    ctx.entities
        .get(id)
        .map_or_else(|| id.to_owned(), |e| entity_title(ctx.lang, e))
}

fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_lowercase().chain(chars).collect())
        .unwrap_or_default()
}

/// The word for an on/off value of an entity. English says on/off; French says "allumée" for a
/// light, the device-class word for a binary sensor ("ouvert") and "activé" for the rest.
fn bool_word(lang: Lang, entity: Option<&EntityView>, on: bool) -> String {
    if lang == Lang::En {
        return if on { "on" } else { "off" }.into();
    }
    match entity {
        Some(e) if e.kind == Kind::Light || shown_as_light(e) => {
            if on { "allumée" } else { "éteinte" }.into()
        }
        Some(e) if e.kind == Kind::BinarySensor => lower_first(&state_word(
            lang,
            StateCtx::BinarySensor(e.device_class.clone()),
            if on { "on" } else { "off" },
        )),
        _ => if on { "activé" } else { "désactivé" }.into(),
    }
}

/// The vocabulary an entity's string states belong to.
fn state_ctx(e: &EntityView) -> StateCtx {
    match e.kind {
        Kind::Cover => StateCtx::Cover,
        Kind::Climate | Kind::WaterHeater => StateCtx::Hvac,
        Kind::AlarmPanel => StateCtx::Alarm,
        Kind::Vacuum => StateCtx::Vacuum,
        Kind::MediaPlayer => StateCtx::Media,
        Kind::BinarySensor => StateCtx::BinarySensor(e.device_class.clone()),
        Kind::Generic if e.component == "lock" => StateCtx::Lock,
        _ => StateCtx::Choice,
    }
}

/// A value as a sentence reads it: booleans on/off, numbers followed by the unit, known state
/// words translated, anything else as it is.
pub(crate) fn value_words(
    lang: Lang,
    value: &Value,
    unit: Option<&str>,
    entity: Option<&EntityView>,
) -> String {
    match value {
        Value::Bool(b) => bool_word(lang, entity, *b),
        Value::Number(n) => {
            let text = n
                .as_f64()
                .map_or_else(|| value_text(value), |f| number(lang, f));
            with_unit(text, unit)
        }
        Value::String(s) if lang == Lang::Fr => {
            let word = state_word(lang, entity.map_or(StateCtx::Choice, state_ctx), s);
            if word == crate::format::word(s) {
                s.clone()
            } else {
                lower_first(&word)
            }
        }
        other => value_text(other),
    }
}

fn unit_of<'a>(id: &str, ctx: &'a Ctx) -> Option<&'a str> {
    ctx.entities.get(id).and_then(|e| e.unit.as_deref())
}

/// What a card reads, as its sentence names it: the entity, followed by the field's name when
/// it reads one ("Roborock · Battery"), with the unit of that value.
fn read_text(id: &str, field: Option<&str>, ctx: &Ctx) -> (String, Option<String>) {
    let e = entity_text(id, ctx);
    let Some(field) = field else {
        return (e, unit_of(id, ctx).map(str::to_owned));
    };
    let unit = ctx.entities.get(id).and_then(|entity| {
        fields(
            entity.kind,
            &entity.component,
            &entity.capabilities,
            entity.unit.as_deref(),
        )
        .into_iter()
        .find(|f| f.name == field)
        .and_then(|f| match f.reports {
            Reports::Number { unit, .. } => unit,
            _ => None,
        })
    });
    (format!("{e} · {}", t::field_label(ctx.lang, field)), unit)
}

/// A generic control as a sentence: "Lock Front door", "Set Bedroom fan percentage to 60",
/// "Activate Scene 1" (a press has no value).
fn control_sentence(lang: Lang, control: &str, value: Option<&Value>, e: &str) -> String {
    let name = control.replace('_', " ");
    // TODO: table-only dispatch. The lock/unlock wording is the one control name matched
    // here; a `sentence` hint on the table's `ControlDef` would let the table own it.
    match (control, value) {
        ("locked", Some(Value::Bool(true))) => {
            format!("{} {e}", flow_text(lang, FlowText::Lock))
        }
        ("locked", Some(Value::Bool(false))) => {
            format!("{} {e}", flow_text(lang, FlowText::Unlock))
        }
        (_, None) => format!("{} {e}", crate::format::word(control)),
        (_, Some(v)) => t::set_control(lang, e, &name, &value_words(lang, v, None, None)),
    }
}

/// The colour word of a hue (0..360) and saturation (0..100): white when barely saturated.
pub(crate) fn colour_name(hs: Hs) -> FlowText {
    if hs.s < 15.0 {
        return FlowText::ColourWhite;
    }
    match hs.h.rem_euclid(360.0) {
        h if h < 15.0 => FlowText::ColourRed,
        h if h < 45.0 => FlowText::ColourOrange,
        h if h < 70.0 => FlowText::ColourYellow,
        h if h < 165.0 => FlowText::ColourGreen,
        h if h < 195.0 => FlowText::ColourCyan,
        h if h < 255.0 => FlowText::ColourBlue,
        h if h < 285.0 => FlowText::ColourPurple,
        h if h < 330.0 => FlowText::ColourMagenta,
        h if h < 345.0 => FlowText::ColourPink,
        _ => FlowText::ColourRed,
    }
}

/// "Turn on LED", "Turn on LED at 39 % in magenta with breathe".
fn turn_on_sentence(
    lang: Lang,
    e: &str,
    brightness: Option<u8>,
    color: Option<Hs>,
    effect: Option<&str>,
) -> String {
    let mut s = format!("{} {e}", flow_text(lang, FlowText::TurnOn));
    if let Some(b) = brightness {
        s += &t::at_brightness(lang, (f64::from(b) * 100.0 / 255.0).round());
    }
    if let Some(c) = color {
        s += &t::in_colour(lang, colour_name(c));
    }
    if let Some(fx) = effect {
        s += &t::with_effect(lang, &fx.replace('_', " "));
    }
    s
}

fn command_sentence(command: &Command, entity: &str, ctx: &Ctx) -> String {
    let lang = ctx.lang;
    let e = entity_text(entity, ctx);
    let verb = |k: FlowText| format!("{} {e}", flow_text(lang, k));
    match command {
        Command::TurnOn {
            brightness,
            color,
            effect,
        } => turn_on_sentence(lang, &e, *brightness, *color, effect.as_deref()),
        Command::TurnOff => verb(FlowText::TurnOff),
        Command::Toggle => verb(FlowText::Toggle),
        Command::Open => verb(FlowText::Open),
        Command::Close => verb(FlowText::Close),
        Command::Stop => verb(FlowText::Stop),
        Command::Press => verb(FlowText::Press),
        Command::Arm { mode, .. } => t::arm(lang, &e, *mode),
        Command::Disarm { .. } => verb(FlowText::Disarm),
        Command::SetPosition { position } => {
            t::set_to(lang, &e, &with_unit(position.to_string(), Some("%")))
        }
        Command::SetTemperature { temperature } => {
            t::set_to(lang, &e, &with_unit(number(lang, *temperature), Some("°C")))
        }
        Command::SetNumber { value } => t::set_to(
            lang,
            &e,
            &with_unit(
                number(lang, *value),
                unit_of(entity, ctx).filter(|u| !u.is_empty()),
            ),
        ),
        Command::SelectOption { option } => t::set_to(lang, &e, option),
        Command::SetHvacMode { mode } => {
            let word = match lang {
                Lang::En => mode.to_string(),
                Lang::Fr => lower_first(&state_word(lang, StateCtx::Hvac, mode)),
            };
            t::set_mode(lang, &e, &word)
        }
        Command::Control { control, value } => control_sentence(lang, control, value.as_ref(), &e),
        other => format!("{} {e}", humanise(command_name(other))),
    }
}

fn op_words(op: Op) -> &'static str {
    match op {
        Op::Gt => ">",
        Op::Lt => "<",
        Op::Gte => "≥",
        Op::Lte => "≤",
        Op::Eq | Op::Ne | Op::IsDark | Op::IsOffPeak => "",
    }
}

/// "Off-peak starts" or "Off-peak ends".
pub fn off_peak_edge_text(edge: OffPeakEdge) -> FlowText {
    match edge {
        OffPeakEdge::Start => FlowText::OffPeakStarts,
        OffPeakEdge::End => FlowText::OffPeakEnds,
    }
}

/// A mode of the alarm as the user reads it: the alarm's state word, or "Armed (any mode)" for
/// the `Armed` filter.
pub fn alarm_mode_label(lang: Lang, mode: AlarmModeFilter) -> String {
    match mode {
        AlarmModeFilter::Armed => flow_text(lang, FlowText::AlarmArmedAny).to_owned(),
        other => state_word(lang, StateCtx::Alarm, other.as_str()),
    }
}

/// The card's sentence, generated from its parameters; ignores the node's name.
pub fn sentence(node: &Node, ctx: &Ctx) -> String {
    let base = base_sentence(node, ctx);
    match node.kind.catch_up_minutes() {
        0 => base,
        n => format!("{base}{}", t::catch_up_suffix(ctx.lang, n)),
    }
}

/// The names of climate zones as a step reads them: the room's name, or the raw id when no room
/// has it any more.
fn zone_names(zones: &[String], ctx: &Ctx) -> String {
    if zones.is_empty() {
        return flow_text(ctx.lang, FlowText::ZonesPlaceholder).to_owned();
    }
    zones
        .iter()
        .map(|id| {
            ctx.areas.iter().find(|a| a.id == *id).map_or_else(
                || id.clone(),
                |a| crate::format::area_name(ctx.lang, &a.id, &a.name),
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The names of the wake-up alarms a trigger follows: the flow's name, or the raw id when no
/// flow has it any more. `flows` is `(id, name)` of every flow.
pub fn alarm_names(alarms: &[String], flows: &[(String, String)]) -> Vec<String> {
    alarms
        .iter()
        .map(|id| {
            flows
                .iter()
                .find(|(fid, _)| fid == id)
                .map_or_else(|| id.clone(), |(_, name)| name.clone())
        })
        .collect()
}

/// How long a climate step lasts, worded.
fn climate_span(lang: Lang, minutes: Option<u32>, until: Option<&str>) -> String {
    let words = minutes.map(|m| duration_words(&format!("{m}m")));
    t::climate_span(lang, words.as_deref(), until)
}

fn base_sentence(node: &Node, ctx: &Ctx) -> String {
    let lang = ctx.lang;
    match &node.kind {
        NodeKind::TriggerState(p) if p.entity.is_empty() => {
            flow_text(lang, FlowText::DeviceSub).to_owned()
        }
        NodeKind::TriggerSignal(p) if p.entity.is_empty() => {
            flow_text(lang, FlowText::DeviceSub).to_owned()
        }
        NodeKind::TriggerState(p) => {
            let (e, unit) = read_text(&p.entity, p.field.as_deref(), ctx);
            let entity = ctx.entities.get(&p.entity);
            let base = if p.above.is_some() || p.below.is_some() {
                let range = p.above.is_some() && p.below.is_some();
                let with = |v: f64, last: bool| {
                    let text = number(lang, v);
                    if !range || last {
                        with_unit(text, unit.as_deref())
                    } else {
                        text
                    }
                };
                t::when_crosses(
                    lang,
                    &e,
                    p.above.map(|v| with(v, false)).as_deref(),
                    p.below.map(|v| with(v, true)).as_deref(),
                )
            } else {
                match &p.to {
                    None => t::when_changes(lang, &e),
                    Some(Value::Bool(b)) => t::when_turns(
                        lang,
                        &e,
                        *b,
                        entity.is_some_and(|x| x.kind == Kind::Light),
                        &bool_word(lang, entity, *b),
                    ),
                    Some(v) => {
                        t::when_becomes(lang, &e, &value_words(lang, v, unit.as_deref(), entity))
                    }
                }
            };
            match &p.for_ {
                Some(hold) => t::held_for(lang, &base, &duration_words(hold)),
                None => base,
            }
        }
        NodeKind::TriggerSignal(p) => {
            t::when_sends(lang, &entity_text(&p.entity, ctx), p.signal.as_deref())
        }
        NodeKind::TriggerSchedule(p) => describe(lang, &p.cron),
        NodeKind::TriggerSun(p) => sun_summary(lang, p.which, p.offset()),
        NodeKind::TriggerOffPeak(p) => flow_text(lang, off_peak_edge_text(p.edge)).to_owned(),
        NodeKind::TriggerAlarm(p) => match (p.from, p.to) {
            (None, None) => t::alarm_changes(lang),
            (None, Some(to)) => t::alarm_becomes(lang, &alarm_mode_label(lang, to)),
            (Some(from), None) => t::alarm_leaves(lang, &alarm_mode_label(lang, from)),
            (Some(from), Some(to)) => t::alarm_goes(
                lang,
                &alarm_mode_label(lang, from),
                &alarm_mode_label(lang, to),
            ),
        },
        NodeKind::TriggerSilent(p) => {
            let e = if p.entity.is_empty() {
                String::new()
            } else {
                entity_text(&p.entity, ctx)
            };
            t::silent_for(lang, &e, &duration_words(&p.for_))
        }
        NodeKind::Limit(p) => t::at_most(lang, p.max, &p.per),
        NodeKind::TriggerManual(p) => {
            let head = flow_text(lang, FlowText::NodeManual);
            match &p.value {
                Some(v) => format!("{head} · {}", value_words(lang, &v.to_json(), None, None)),
                None => head.to_owned(),
            }
        }
        NodeKind::If(p) if p.op == Op::IsDark => t::is_dark_sentence(lang, p.dark_offset()),
        NodeKind::If(p) if p.op == Op::IsOffPeak => flow_text(lang, FlowText::IsOffPeak).to_owned(),
        NodeKind::If(p) => {
            let ((e, unit), entity) = match (&p.result, &p.entity) {
                (Some(name), _) => ((name.clone(), None), None),
                (None, Some(id)) => (read_text(id, p.field.as_deref(), ctx), ctx.entities.get(id)),
                (None, None) => (
                    (flow_text(lang, FlowText::TriggerValue).to_owned(), None),
                    None,
                ),
            };
            let v = value_words(
                lang,
                p.value.as_ref().unwrap_or(&Value::Null),
                unit.as_deref(),
                entity,
            );
            let e = match p.average.as_deref() {
                Some(window) => t::average_of(lang, &e, &duration_words(window)),
                None => e,
            };
            match p.op {
                Op::Eq => t::is_value(lang, &e, &v),
                Op::Ne => t::is_not_value(lang, &e, &v),
                op => t::ask(lang, &format!("{e} {} {v}", op_words(op))),
            }
        }
        NodeKind::Delay(p) => t::wait(
            lang,
            &duration_words(&p.for_),
            p.up_to.as_deref().map(duration_words).as_deref(),
        ),
        NodeKind::Command(p) => match &p.value_from {
            Some(name) if command_with_number(&p.command, 0.0).is_some() => {
                t::set_to(lang, &entity_text(&p.entity, ctx), name)
            }
            _ => command_sentence(&p.command, &p.entity, ctx),
        },
        NodeKind::Switch(p) => {
            let e = match (&p.result, &p.entity) {
                (Some(name), _) => name.clone(),
                (None, Some(id)) => read_text(id, p.field.as_deref(), ctx).0,
                (None, None) => flow_text(lang, FlowText::TriggerValue).to_owned(),
            };
            let e = match p.average.as_deref() {
                Some(window) => t::average_of(lang, &e, &duration_words(window)),
                None => e,
            };
            t::switch_on(lang, &e)
        }
        NodeKind::TimeBetween(p) => t::time_between(lang, &p.from, &p.to),
        NodeKind::Math(p) => {
            let head = format!("{} = {}", p.name, p.formula);
            match &p.save_to {
                Some(id) => format!("{head} → {}", entity_text(id, ctx)),
                None => head,
            }
        }
        NodeKind::Notify(p) => {
            // Placeholders show the entity's name; a text that does not parse shows as typed.
            let name = |id: &str| ctx.entities.get(id).map(|e| entity_title(lang, e));
            let title =
                template::parse(&p.title).map_or_else(|_| p.title.clone(), |t| t.label(&name));
            t::notify(lang, &title)
        }
        NodeKind::Speak(nidoo_core::flow::SpeakParams { text, room }) => {
            let (text, room) = spoken_text_and_room(text, room.as_deref(), ctx);
            t::speak(lang, &text, room.as_deref())
        }
        NodeKind::Ask(p) => {
            let (text, room) = spoken_text_and_room(&p.text, p.room.as_deref(), ctx);
            let labels: Vec<&str> = p.answers.iter().map(|a| a.label.as_str()).collect();
            t::ask_card(lang, &text, room.as_deref(), &labels)
        }
        NodeKind::FlowEnable(p) => {
            let target = match &p.flow {
                None => flow_text(lang, FlowText::ThisFlow).to_owned(),
                Some(id) => ctx
                    .flows
                    .iter()
                    .find(|(fid, _)| fid == id)
                    .map_or_else(|| id.clone(), |(_, name)| name.clone()),
            };
            t::flow_switch(lang, p.enabled, &target)
        }
        NodeKind::FlowLabel(p) => {
            let label = ctx.labels.label(&p.label);
            let choice = label
                .and_then(|l| l.choice.as_deref())
                .and_then(|c| ctx.labels.choice(c))
                .map(|c| c.name.as_str());
            let name = label.map_or(p.label.as_str(), |l| l.name.as_str());
            t::flow_label_card(lang, p.action, name, choice)
        }
        NodeKind::BackupRun(_) => flow_text(lang, FlowText::BackupRunSentence).to_owned(),
        NodeKind::TriggerWakeUp(p) => {
            t::wake_up_sentence(lang, p.before_minutes, &alarm_names(&p.alarms, ctx.flows))
        }
        NodeKind::ClimateHold(p) => {
            let temperature = match p.setpoint {
                nidoo_core::flow::ClimateSetpoint::Temp(c) => {
                    Some(with_unit(number(lang, c), Some("°C")))
                }
                nidoo_core::flow::ClimateSetpoint::Off => None,
            };
            t::climate_hold(
                lang,
                &zone_names(&p.zones, ctx),
                temperature.as_deref(),
                &climate_span(lang, p.minutes, p.until.as_deref()),
            )
        }
        NodeKind::ClimateEco(p) => t::climate_eco(
            lang,
            &zone_names(&p.zones, ctx),
            &climate_span(lang, p.minutes, p.until.as_deref()),
        ),
    }
}

/// A spoken text with its placeholders worded, and the room's name when it has one.
fn spoken_text_and_room(text: &str, room: Option<&str>, ctx: &Ctx) -> (String, Option<String>) {
    let name = |id: &str| ctx.entities.get(id).map(|e| entity_title(ctx.lang, e));
    let text = template::parse(text).map_or_else(|_| text.to_owned(), |t| t.label(&name));
    let room = room.map(|id| {
        ctx.areas
            .iter()
            .find(|a| a.id == id)
            .map_or_else(|| id.to_owned(), |a| a.name.clone())
    });
    (text, room)
}

/// The short label beside the pin `port` of a card: a switch rule as "< 18 lx", "18–22 lx" or
/// "= on" (the unit and value words of the card's sentence), "otherwise" for the last exit, and
/// a time card's "07:00–22:30" / "outside". Empty for the other cards, whose pins carry none.
pub fn exit_label(lang: Lang, kind: &NodeKind, port: &str, ctx: &Ctx) -> String {
    match kind {
        NodeKind::Switch(p) => {
            if port == "otherwise" {
                return flow_text(lang, FlowText::Otherwise).to_owned();
            }
            let Some(rule) = port
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .and_then(|i| p.rules.get(i))
            else {
                return String::new();
            };
            let entity = p.entity.as_deref().and_then(|id| ctx.entities.get(id));
            let unit = p
                .entity
                .as_deref()
                .and_then(|id| read_text(id, p.field.as_deref(), ctx).1);
            let unit = unit.as_deref();
            let words = |v: &Value| value_words(lang, v, unit, entity);
            let sign = match rule.op {
                RuleOp::Eq => "=",
                RuleOp::Ne => "≠",
                RuleOp::Gt => ">",
                RuleOp::Lt => "<",
                RuleOp::Gte => "≥",
                RuleOp::Lte => "≤",
                RuleOp::Between => {
                    // The unit once, after the range: "18–22 °C".
                    let num = |v: &Value| match v {
                        Value::Number(n) => Some(
                            n.as_f64()
                                .map_or_else(|| n.to_string(), |f| number(lang, f)),
                        ),
                        _ => None,
                    };
                    return match (num(&rule.value), rule.to.as_ref().and_then(num)) {
                        (Some(a), Some(b)) => with_unit(format!("{a}–{b}"), unit),
                        _ => format!(
                            "{}–{}",
                            words(&rule.value),
                            rule.to.as_ref().map(words).unwrap_or_default()
                        ),
                    };
                }
            };
            format!("{sign} {}", words(&rule.value))
        }
        NodeKind::Ask(p) => {
            if port == nidoo_core::flow::ASK_NONE_PORT {
                return flow_text(lang, FlowText::AskNoAnswer).to_owned();
            }
            port.parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .and_then(|i| p.answers.get(i))
                .map(|a| a.label.clone())
                .unwrap_or_default()
        }
        NodeKind::TimeBetween(p) => match port {
            "in" => format!("{}–{}", p.from, p.to),
            "out" => flow_text(lang, FlowText::Outside).to_owned(),
            _ => String::new(),
        },
        NodeKind::BackupRun(_) => match port {
            nidoo_core::flow::BACKUP_OK_PORT => flow_text(lang, FlowText::BackupOk).to_owned(),
            nidoo_core::flow::BACKUP_PARTIAL_PORT => {
                flow_text(lang, FlowText::BackupPartial).to_owned()
            }
            nidoo_core::flow::BACKUP_FAILED_PORT => {
                flow_text(lang, FlowText::BackupFailed).to_owned()
            }
            _ => String::new(),
        },
        _ => String::new(),
    }
}

fn named(node: &Node) -> Option<&str> {
    node.name.as_deref().filter(|n| !n.trim().is_empty())
}

/// The node's name when it has one, else its [`sentence`].
pub fn title(node: &Node, ctx: &Ctx) -> String {
    named(node).map_or_else(|| sentence(node, ctx), str::to_owned)
}

fn entity_id(node: &Node) -> Option<&str> {
    match &node.kind {
        NodeKind::TriggerState(p) => Some(&p.entity),
        NodeKind::TriggerSignal(p) => Some(&p.entity),
        NodeKind::If(p) => p.entity.as_deref(),
        NodeKind::Switch(p) => p.entity.as_deref(),
        NodeKind::Command(p) => Some(&p.entity),
        _ => None,
    }
}

/// The small second line: `"{area} · {device}"` for a node with an entity, the sentence for
/// another node that carries a name.
pub fn subline(node: &Node, ctx: &Ctx) -> Option<String> {
    let Some(id) = entity_id(node) else {
        return named(node).map(|_| sentence(node, ctx));
    };
    let entity = ctx.entities.get(id)?;
    let area = ctx
        .areas
        .iter()
        .find(|a| a.id == entity.area)
        .map(|a| crate::format::area_name(ctx.lang, &a.id, &a.name));
    let device = entity.device_name.as_deref();
    let parts: Vec<String> = [area, device.map(str::to_owned)]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// `"5m"` as "5 min", `"1h30m"` as "1 h 30 min", `"45s"` as "45 s" (no-break space between a
/// number and its unit, in both languages); text that is not a duration comes back unchanged.
pub fn duration_words(spec: &str) -> String {
    let Ok(d) = parse_duration(spec) else {
        return spec.to_owned();
    };
    let secs = d.as_secs();
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    let parts: Vec<String> = [(h, "h"), (m, "min"), (s, "s")]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, unit)| with_unit(n.to_string(), Some(unit)))
        .collect();
    parts.join(" ")
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod generic_tests;
