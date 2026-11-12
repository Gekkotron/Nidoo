//! Sentences for flow cards: "Turn off LED", "Entrée > 16 lx?". Pure, tested natively.

use std::collections::BTreeMap;

use nidoo_api::{AreaView, EntityView};
use nidoo_core::command::command_name;
use nidoo_core::flow::{
    FlowLabelCatalog, Node, NodeKind, OffPeakEdge, Op, RuleOp, SunWhich, command_with_number,
    fields, parse_duration, template,
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
        | NodeKind::TriggerManual(_)
        | NodeKind::TriggerSignal(_)
        | NodeKind::TriggerOffPeak(_)
        | NodeKind::TriggerSilent(_) => Role::Trigger,
        NodeKind::If(_) | NodeKind::Switch(_) | NodeKind::TimeBetween(_) => Role::Check,
        NodeKind::Command(_)
        | NodeKind::FlowEnable(_)
        | NodeKind::FlowLabel(_)
        | NodeKind::BackupRun(_)
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

/// The card's sentence, generated from its parameters; ignores the node's name.
pub fn sentence(node: &Node, ctx: &Ctx) -> String {
    let lang = ctx.lang;
    match &node.kind {
        NodeKind::TriggerState(p) => {
            let (e, unit) = read_text(&p.entity, p.field.as_deref(), ctx);
            let entity = ctx.entities.get(&p.entity);
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
        }
        NodeKind::TriggerSignal(p) => {
            t::when_sends(lang, &entity_text(&p.entity, ctx), p.signal.as_deref())
        }
        NodeKind::TriggerSchedule(p) => describe(lang, &p.cron),
        NodeKind::TriggerSun(p) => sun_summary(lang, p.which, p.offset()),
        NodeKind::TriggerOffPeak(p) => flow_text(lang, off_peak_edge_text(p.edge)).to_owned(),
        NodeKind::TriggerSilent(p) => {
            let e = if p.entity.is_empty() {
                String::new()
            } else {
                entity_text(&p.entity, ctx)
            };
            t::silent_for(lang, &e, &p.for_)
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
mod tests {
    use super::*;

    #[test]
    fn summary_line_renders_expected_string() {
        let en = |w, o| sun_summary(Lang::En, w, o);
        assert_eq!(en(SunWhich::Sunset, 0), "At sunset");
        assert_eq!(en(SunWhich::Sunrise, 15), "15\u{a0}min after sunrise");
        assert_eq!(en(SunWhich::Dusk, -15), "15\u{a0}min before civil dusk");
        assert_eq!(en(SunWhich::Dawn, 0), "At civil dawn");
        assert_eq!(
            sun_summary(Lang::Fr, SunWhich::Sunrise, 15),
            "15\u{a0}min après le lever du soleil"
        );
    }
    use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};

    fn ent(
        id: &str,
        name: &str,
        device: Option<&str>,
        kind: Kind,
        unit: Option<&str>,
    ) -> EntityView {
        EntityView {
            id: id.into(),
            device_id: None,
            name: name.into(),
            device_name: device.map(Into::into),
            label: None,
            kind,
            component: kind.component().to_string(),
            protocol_kind: kind,
            device_class: None,
            unit: unit.map(Into::into),
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: "salon".into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride::default(),
            state: EntityState::default(),
        }
    }

    #[test]
    fn a_switch_shown_as_a_light_reads_allumee() {
        let mut lamp = ent("s1", "Lampe", None, Kind::Switch, None);
        lamp.ui.show_as = Some("light".into());
        assert_eq!(bool_word(Lang::Fr, Some(&lamp), true), "allumée");
        assert_eq!(bool_word(Lang::Fr, Some(&lamp), false), "éteinte");
        let plain = ent("s2", "Prise", None, Kind::Switch, None);
        assert_eq!(bool_word(Lang::Fr, Some(&plain), true), "activé");
    }

    fn house() -> BTreeMap<String, EntityView> {
        [
            ent("l1", "Lumière TV", None, Kind::Light, None),
            ent("lux", "Entrée", None, Kind::Sensor, Some("lx")),
            ent("led", "LED", Some("LED strip"), Kind::Light, None),
            ent("p1", "Power 1", None, Kind::Number, Some("%")),
            ent("mot", "Motion", None, Kind::BinarySensor, None),
            ent("robo", "Roborock", None, Kind::Vacuum, None),
        ]
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect()
    }

    fn areas() -> Vec<AreaView> {
        serde_json::from_str(
            r#"[{"id":"salon","name":"Salon","icon":null,"order":null,"entity_count":0,"primary_count":0}]"#,
        )
        .unwrap()
    }

    fn node(json: &str) -> Node {
        serde_json::from_str(json).unwrap()
    }

    fn text(json: &str) -> String {
        text_in(Lang::En, json)
    }

    fn text_in(lang: Lang, json: &str) -> String {
        let entities = house();
        let areas = areas();
        let flows = vec![(
            "force-heating-loop".to_owned(),
            "Force heating loop".to_owned(),
        )];
        title(
            &node(json),
            &Ctx {
                entities: &entities,
                areas: &areas,
                flows: &flows,
                labels: &crate::flow_text::NO_LABELS,
                lang,
            },
        )
    }

    #[test]
    fn flow_label_sentence() {
        let catalog: nidoo_core::flow::FlowLabelCatalog =
            serde_json::from_value(serde_json::json!({
                "labels": [
                    {"id": "hiver", "name": "Hiver", "choice": "saison"},
                    {"id": "ete", "name": "Été", "choice": "saison"},
                    {"id": "vacances", "name": "Vacances"}
                ],
                "choices": [{"id": "saison", "name": "Saison"}]
            }))
            .unwrap();
        let entities = house();
        let areas = areas();
        let say = |lang: Lang, json: &str| {
            sentence(
                &node(json),
                &Ctx {
                    entities: &entities,
                    areas: &areas,
                    flows: &[],
                    labels: &catalog,
                    lang,
                },
            )
        };
        let pick = r#"{"id":"a","type":"flow.label","params":{"label":"ete","action":"pick"}}"#;
        assert_eq!(say(Lang::En, pick), "Pick a label: Saison → Été");
        assert_eq!(
            say(Lang::Fr, pick),
            "Choisir une étiquette\u{a0}: Saison → Été"
        );
        let on = r#"{"id":"a","type":"flow.label","params":{"label":"vacances","action":"on"}}"#;
        assert_eq!(say(Lang::En, on), "Turn on \"Vacances\"");
        assert_eq!(say(Lang::Fr, on), "Activer «\u{a0}Vacances\u{a0}»");
        let off = r#"{"id":"a","type":"flow.label","params":{"label":"vacances","action":"off"}}"#;
        assert_eq!(say(Lang::En, off), "Turn off \"Vacances\"");
        assert_eq!(say(Lang::Fr, off), "Désactiver «\u{a0}Vacances\u{a0}»");
        // A label the catalog lost reads as its id; an unset one as the card's name.
        let gone = r#"{"id":"a","type":"flow.label","params":{"label":"gone","action":"pick"}}"#;
        assert_eq!(say(Lang::En, gone), "Pick a label: gone");
        let unset = r#"{"id":"a","type":"flow.label","params":{"label":"","action":"pick"}}"#;
        assert_eq!(say(Lang::En, unset), "Pick a label");
    }

    #[test]
    fn backup_run_sentence_and_ports() {
        let json =
            r#"{"id":"b","type":"backup.run","params":{"destination":"ftp","timeout_minutes":30}}"#;
        assert_eq!(text_in(Lang::En, json), "Back up to the FTP server");
        assert_eq!(text_in(Lang::Fr, json), "Sauvegarder sur le serveur FTP");
        assert_eq!(label(Lang::En, json, "ok"), "Backed up");
        assert_eq!(label(Lang::En, json, "failed"), "Failed");
        assert_eq!(label(Lang::Fr, json, "ok"), "Sauvegardé");
        assert_eq!(label(Lang::Fr, json, "failed"), "Échec");
    }

    #[test]
    fn french_twins_of_the_english_sentences() {
        let fr = |json: &str| text_in(Lang::Fr, json);
        let off = r#"{"id":"a","type":"command","params":{"entity":"led","command":{"type":"turn_off"}}}"#;
        assert_eq!(fr(off), "Éteindre LED");
        let on =
            r#"{"id":"a","type":"command","params":{"entity":"l1","command":{"type":"turn_on"}}}"#;
        assert_eq!(fr(on), "Allumer Lumière TV");
        let changes = r#"{"id":"a","type":"trigger.state","params":{"entity":"l1"}}"#;
        assert_eq!(fr(changes), "Quand Lumière TV change");
        let is_on = r#"{"id":"a","type":"if","params":{"entity":"l1","op":"eq","value":true}}"#;
        assert_eq!(fr(is_on), "Lumière TV est allumée\u{a0}?");
        let turns = r#"{"id":"a","type":"trigger.state","params":{"entity":"l1","to":true}}"#;
        assert_eq!(fr(turns), "Quand Lumière TV s'allume");
        let gt = r#"{"id":"a","type":"if","params":{"entity":"lux","op":"gt","value":16}}"#;
        assert_eq!(fr(gt), "Entrée > 16\u{a0}lx\u{a0}?");
        let wait = r#"{"id":"a","type":"delay","params":{"for":"1h30m","up_to":"2h"}}"#;
        assert_eq!(fr(wait), "Attendre de 1\u{a0}h 30\u{a0}min à 2\u{a0}h");
        let notify = r#"{"id":"a","type":"notify","params":{"title":"Porte"}}"#;
        assert_eq!(fr(notify), "Notifier «\u{a0}Porte\u{a0}»");
        let dark = r#"{"id":"d","type":"if","params":{"op":"is_dark"}}"#;
        assert_eq!(fr(dark), "Fait-il nuit\u{a0}? (soleil sous \u{2212}6°)");
        let daily = r#"{"id":"s","type":"trigger.schedule","params":{"cron":"0 7 * * *"}}"#;
        assert_eq!(text(daily), "Every day at 07:00");
        assert_eq!(fr(daily), "Tous les jours à 07:00");
        let weekdays = r#"{"id":"s","type":"trigger.schedule","params":{"cron":"0 7 * * 1-5"}}"#;
        assert_eq!(fr(weekdays), "Du lundi au vendredi à 07:00");
        let enable = r#"{"id":"a","type":"flow.enable","params":{"enabled":false}}"#;
        assert_eq!(fr(enable), "Désactiver cette automatisation");
    }

    #[test]
    fn switch_and_time_sentences_en_fr() {
        let switch = r#"{"id":"s","type":"switch","params":{"entity":"lux","rules":[{"op":"lt","value":18}]}}"#;
        assert_eq!(text(switch), "Switch on Entrée");
        assert_eq!(text_in(Lang::Fr, switch), "Commuter selon Entrée");
        let bare = r#"{"id":"s","type":"switch","params":{"rules":[{"op":"lt","value":18}]}}"#;
        assert_eq!(text(bare), "Switch on Trigger value");
        let time = r#"{"id":"t","type":"time.between","params":{"from":"07:00","to":"22:30"}}"#;
        assert_eq!(text(time), "Between 07:00 and 22:30");
        assert_eq!(text_in(Lang::Fr, time), "Entre 07:00 et 22:30");
        assert_eq!(role(&node(switch).kind), Role::Check);
        assert_eq!(role(&node(time).kind), Role::Check);
    }

    #[test]
    fn ask_sentence_en_fr() {
        let ask = r#"{"id":"a","type":"ask","params":{"text":"Heat up?","room":"salon","answers":[{"label":"Oui"},{"label":"Non"}]}}"#;
        assert_eq!(
            text(ask),
            "Ask \"Heat up?\" in Salon: Oui / Non / no answer"
        );
        assert_eq!(
            text_in(Lang::Fr, ask),
            "Demande «\u{a0}Heat up?\u{a0}» dans Salon\u{a0}: Oui / Non / pas de réponse"
        );
        let anywhere =
            r#"{"id":"a","type":"ask","params":{"text":"Q","answers":[{"label":"Yes"}]}}"#;
        assert_eq!(text(anywhere), "Ask \"Q\" — All rooms: Yes / no answer");
        assert_eq!(role(&node(ask).kind), Role::Action);
    }

    #[test]
    fn ask_exit_labels() {
        let ask = r#"{"id":"a","type":"ask","params":{"text":"Q","answers":[{"label":"Oui"},{"label":"Non"}]}}"#;
        assert_eq!(label(Lang::En, ask, "1"), "Oui");
        assert_eq!(label(Lang::En, ask, "2"), "Non");
        assert_eq!(label(Lang::En, ask, "none"), "No answer");
        assert_eq!(label(Lang::Fr, ask, "none"), "Pas de réponse");
        assert_eq!(label(Lang::En, ask, "3"), "");
    }

    fn label(lang: Lang, json: &str, port: &str) -> String {
        let entities = house();
        let areas = areas();
        let ctx = Ctx {
            entities: &entities,
            areas: &areas,
            flows: &[],
            labels: &crate::flow_text::NO_LABELS,
            lang,
        };
        exit_label(lang, &node(json).kind, port, &ctx)
    }

    #[test]
    fn exit_labels() {
        let switch = r#"{"id":"s","type":"switch","params":{"entity":"lux","otherwise":true,"rules":[
            {"op":"lt","value":18},{"op":"between","value":18,"to":22},{"op":"eq","value":true}]}}"#;
        assert_eq!(label(Lang::En, switch, "1"), "< 18\u{a0}lx");
        assert_eq!(label(Lang::En, switch, "2"), "18–22\u{a0}lx");
        assert_eq!(label(Lang::En, switch, "3"), "= on");
        assert_eq!(label(Lang::En, switch, "otherwise"), "otherwise");
        assert_eq!(label(Lang::Fr, switch, "otherwise"), "sinon");
        assert_eq!(label(Lang::Fr, switch, "2"), "18–22\u{a0}lx");
        assert_eq!(label(Lang::En, switch, "9"), "");
        let light = r#"{"id":"s","type":"switch","params":{"entity":"l1","rules":[{"op":"eq","value":true}]}}"#;
        assert_eq!(label(Lang::Fr, light, "1"), "= allumée");
        let time = r#"{"id":"t","type":"time.between","params":{"from":"07:00","to":"22:30"}}"#;
        assert_eq!(label(Lang::En, time, "in"), "07:00–22:30");
        assert_eq!(label(Lang::En, time, "out"), "outside");
        assert_eq!(label(Lang::Fr, time, "out"), "en dehors");
        let ifnode = r#"{"id":"a","type":"if","params":{"entity":"l1","op":"eq","value":true}}"#;
        assert_eq!(label(Lang::En, ifnode, "true"), "");
    }

    #[test]
    fn french_colours_and_numbers() {
        let fr = |json: &str| text_in(Lang::Fr, json);
        let on = |cmd: &str| {
            fr(&format!(
                r#"{{"id":"a","type":"command","params":{{"entity":"led","command":{cmd}}}}}"#
            ))
        };
        assert_eq!(
            on(r#"{"type":"turn_on","brightness":100,"color":{"h":295.0,"s":100.0}}"#),
            "Allumer LED à 39\u{a0}% en magenta"
        );
        assert_eq!(
            on(r#"{"type":"turn_on","color":{"h":30.0,"s":4.0},"effect":"breathe"}"#),
            "Allumer LED en blanc avec breathe"
        );
        assert_eq!(
            on(r#"{"type":"set_temperature","temperature":21.5}"#),
            "Régler LED sur 21,5\u{a0}°C"
        );
    }

    #[test]
    fn manual_card_reads_with_its_value() {
        let card =
            |value: &str| format!(r#"{{"id":"m","type":"trigger.manual","params":{value}}}"#);
        let none = card("{}");
        let on = card(r#"{"value":true}"#);
        let num = card(r#"{"value":21.5}"#);
        assert_eq!(text(&none), "Manual");
        assert_eq!(text(&on), "Manual · on");
        assert_eq!(text(&num), "Manual · 21.5");
        assert_eq!(text_in(Lang::Fr, &none), "Manuel");
        assert_eq!(text_in(Lang::Fr, &on), "Manuel · activé");
        assert_eq!(text_in(Lang::Fr, &num), "Manuel · 21,5");
    }

    #[test]
    fn sun_and_is_dark_cards_read_as_sentences() {
        let sun =
            r#"{"id":"s","type":"trigger.sun","params":{"which":"dusk","offset_minutes":-15}}"#;
        assert_eq!(text(sun), "15\u{a0}min before civil dusk");
        let dark = r#"{"id":"d","type":"if","params":{"op":"is_dark"}}"#;
        assert_eq!(text(dark), "Is it dark now? (sun below \u{2212}6°)");
        let early = r#"{"id":"d","type":"if","params":{"op":"is_dark","offset_minutes":-30}}"#;
        assert_eq!(
            text(early),
            "Is it dark? (from 30\u{a0}min before civil dusk)"
        );
    }

    #[test]
    fn off_peak_cards_read_as_sentences() {
        let start = r#"{"id":"o","type":"trigger.off_peak","params":{"edge":"start"}}"#;
        assert_eq!(text(start), "Off-peak starts");
        let end = r#"{"id":"o","type":"trigger.off_peak","params":{"edge":"end"}}"#;
        assert_eq!(text(end), "Off-peak ends");
        assert_eq!(text_in(Lang::Fr, end), "Les heures creuses finissent");
        let check = r#"{"id":"c","type":"if","params":{"op":"is_off_peak"}}"#;
        assert_eq!(text(check), "Is it off-peak?");
        assert_eq!(text_in(Lang::Fr, check), "Est-ce les heures creuses\u{a0}?");
    }

    #[test]
    fn silent_and_limit_cards_read_as_sentences() {
        let silent = r#"{"id":"s","type":"trigger.silent","params":{"entity":"l1","for":"50m"}}"#;
        assert_eq!(text(silent), "Lumière TV silent for 50m");
        assert_eq!(
            text_in(Lang::Fr, silent),
            "Lumière TV silencieux depuis 50m"
        );
        let unset = r#"{"id":"s","type":"trigger.silent","params":{"entity":"","for":"50m"}}"#;
        assert_eq!(text(unset), "Silent for 50m");
        let limit = r#"{"id":"l","type":"limit","params":{"max":4,"per":"24h"}}"#;
        assert_eq!(text(limit), "At most 4 every 24h");
        assert_eq!(text_in(Lang::Fr, limit), "Au plus 4 fois par 24h");
    }

    #[test]
    fn if_eq_bool_reads_on() {
        let n =
            r#"{"id":"a","type":"if","name":"  ","params":{"entity":"l1","op":"eq","value":true}}"#;
        assert_eq!(text(n), "Lumière TV is on?");
    }

    #[test]
    fn if_sentence_names_the_field() {
        let n = r#"{"id":"a","type":"if","params":{"entity":"robo","field":"battery","op":"lte","value":55}}"#;
        assert_eq!(text(n), "Roborock · Battery ≤ 55\u{a0}%?");
        assert_eq!(
            text_in(Lang::Fr, n),
            "Roborock · Batterie ≤ 55\u{a0}%\u{a0}?"
        );
        let when =
            r#"{"id":"a","type":"trigger.state","params":{"entity":"robo","field":"battery"}}"#;
        assert_eq!(text(when), "When Roborock · Battery changes");
        let switch = r#"{"id":"a","type":"switch","params":{"entity":"robo","field":"battery","rules":[{"op":"lte","value":20}]}}"#;
        assert_eq!(text(switch), switch_on_text("Roborock · Battery"));
    }

    fn switch_on_text(e: &str) -> String {
        crate::i18n::flows::switch_on(Lang::En, e)
    }

    #[test]
    fn if_gt_uses_unit() {
        let n = r#"{"id":"a","type":"if","params":{"entity":"lux","op":"gt","value":16}}"#;
        assert_eq!(text(n), "Entrée > 16\u{a0}lx?");
    }

    #[test]
    fn command_turn_off() {
        let n = r#"{"id":"a","type":"command","params":{"entity":"led","command":{"type":"turn_off"}}}"#;
        assert_eq!(text(n), "Turn off LED");
    }

    #[test]
    fn command_turn_on_names_colour_and_effect() {
        let on = |cmd: &str| {
            text(&format!(
                r#"{{"id":"a","type":"command","params":{{"entity":"led","command":{cmd}}}}}"#
            ))
        };
        assert_eq!(on(r#"{"type":"turn_on"}"#), "Turn on LED");
        assert_eq!(
            on(r#"{"type":"turn_on","brightness":100,"color":{"h":295.0,"s":100.0}}"#),
            "Turn on LED at 39\u{a0}% in magenta"
        );
        assert_eq!(
            on(r#"{"type":"turn_on","color":{"h":30.0,"s":4.0}}"#),
            "Turn on LED in white"
        );
        assert_eq!(
            on(r#"{"type":"turn_on","color":{"h":355.0,"s":90.0},"effect":"breathe"}"#),
            "Turn on LED in red with breathe"
        );
    }

    #[test]
    fn command_set_number() {
        let n = r#"{"id":"a","type":"command","params":{"entity":"p1","command":{"type":"set_number","value":100}}}"#;
        assert_eq!(text(n), "Set Power 1 to 100\u{a0}%");
    }

    #[test]
    fn trigger_to_true() {
        let n = r#"{"id":"a","type":"trigger.state","params":{"entity":"mot","to":true}}"#;
        assert_eq!(text(n), "When Motion turns on");
    }

    #[test]
    fn delay_words() {
        let d = |s: &str| {
            text(&format!(
                r#"{{"id":"a","type":"delay","params":{{"for":"{s}"}}}}"#
            ))
        };
        assert_eq!(d("5m"), "Wait 5\u{a0}min");
        assert_eq!(d("1h30m"), "Wait 1\u{a0}h 30\u{a0}min");
        assert_eq!(duration_words("45s"), "45\u{a0}s");
        assert_eq!(duration_words("junk"), "junk");
        let random = text(r#"{"id":"a","type":"delay","params":{"for":"1m","up_to":"2h"}}"#);
        assert_eq!(random, "Wait 1\u{a0}min to 2\u{a0}h");
    }

    #[test]
    fn notify_text() {
        let n = r#"{"id":"a","type":"notify","params":{"title":"Door open"}}"#;
        assert_eq!(text(n), "Notify \"Door open\"");
        let empty = r#"{"id":"a","type":"notify","params":{"title":""}}"#;
        assert_eq!(text(empty), "Notify");
    }

    #[test]
    fn notify_title_names_placeholder_entities() {
        let n = r#"{"id":"a","type":"notify","params":{"title":"Light {lux:0} {gone} {trigger}"}}"#;
        assert_eq!(text(n), "Notify \"Light {Entrée:0} {gone} {trigger}\"");
        let broken = r#"{"id":"a","type":"notify","params":{"title":"Light {lux"}}"#;
        assert_eq!(text(broken), "Notify \"Light {lux\"");
    }

    #[test]
    fn flow_enable_names() {
        let named = r#"{"id":"a","type":"flow.enable","params":{"flow":"force-heating-loop","enabled":false}}"#;
        assert_eq!(text(named), "Disable Force heating loop");
        let own = r#"{"id":"a","type":"flow.enable","params":{"enabled":false}}"#;
        assert_eq!(text(own), "Disable this flow");
    }

    #[test]
    fn name_wins_and_subline() {
        let entities = house();
        let areas = areas();
        let ctx = Ctx {
            entities: &entities,
            areas: &areas,
            flows: &[],
            labels: &crate::flow_text::NO_LABELS,
            lang: crate::i18n::Lang::En,
        };
        let n = node(
            r#"{"id":"a","name":"LED is on","type":"if","params":{"entity":"led","op":"eq","value":true}}"#,
        );
        assert_eq!(title(&n, &ctx), "LED is on");
        assert_eq!(subline(&n, &ctx), Some("Salon · LED strip".to_owned()));
    }

    #[test]
    fn unnamed_delay_has_no_subline() {
        let entities = house();
        let ctx = Ctx {
            entities: &entities,
            areas: &[],
            flows: &[],
            labels: &crate::flow_text::NO_LABELS,
            lang: crate::i18n::Lang::En,
        };
        let n = node(r#"{"id":"a","type":"delay","params":{"for":"5m"}}"#);
        assert_eq!(subline(&n, &ctx), None);
        assert_eq!(role(&n.kind), Role::Wait);
    }

    #[test]
    fn cards_that_read_a_math_result_name_it() {
        let set = r#"{"id":"a","type":"command","params":{"entity":"led","command":{"type":"set_number","value":0},"value_from":"half"}}"#;
        assert_eq!(text(set), "Set LED to half");
        assert_eq!(text_in(Lang::Fr, set), "Régler LED sur half");
        let cmp = r#"{"id":"a","type":"if","params":{"result":"half","op":"gt","value":300}}"#;
        assert_eq!(text(cmp), "half > 300?");
        assert_eq!(text_in(Lang::Fr, cmp), "half > 300\u{a0}?");
        let switch = r#"{"id":"s","type":"switch","params":{"result":"half","rules":[{"op":"lt","value":18}]}}"#;
        assert_eq!(text(switch), "Switch on half");
        assert_eq!(text_in(Lang::Fr, switch), "Commuter selon half");
        // A command with no number ignores a stale link.
        let off = r#"{"id":"a","type":"command","params":{"entity":"led","command":{"type":"turn_off"},"value_from":"half"}}"#;
        assert_eq!(text(off), "Turn off LED");
    }
}

#[cfg(test)]
mod generic_tests {
    use std::collections::BTreeMap;

    use nidoo_api::EntityView;
    use nidoo_core::flow::{Node, NodeKind, TriggerSignalParams};
    use nidoo_core::model::{Capabilities, Command, EntityCategory, EntityState, Kind, UiOverride};
    use serde_json::json;

    use super::*;

    fn ent(id: &str, name: &str) -> EntityView {
        EntityView {
            id: id.into(),
            device_id: None,
            name: name.into(),
            device_name: None,
            label: None,
            kind: Kind::Generic,
            component: "lock".into(),
            protocol_kind: Kind::Generic,
            device_class: None,
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: "salon".into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride::default(),
            state: EntityState::default(),
        }
    }

    fn ctx_of(entities: &BTreeMap<String, EntityView>) -> Ctx<'_> {
        Ctx {
            entities,
            areas: &[],
            flows: &[],
            labels: &crate::flow_text::NO_LABELS,
            lang: crate::i18n::Lang::En,
        }
    }

    #[test]
    fn control_command_sentence_reads_naturally() {
        let entities = BTreeMap::new();
        let ctx = ctx_of(&entities);
        let c = |control: &str, value| Command::Control {
            control: control.into(),
            value,
        };
        assert_eq!(
            command_sentence(&c("locked", Some(json!(false))), "Front door", &ctx),
            "Unlock Front door"
        );
        assert_eq!(
            command_sentence(&c("percentage", Some(json!(60))), "Bedroom fan", &ctx),
            "Set Bedroom fan percentage to 60"
        );
        assert_eq!(
            command_sentence(&c("activate", None), "Scene 1", &ctx),
            "Activate Scene 1"
        );
        let fr = Ctx {
            lang: crate::i18n::Lang::Fr,
            ..ctx_of(&entities)
        };
        assert_eq!(
            command_sentence(&c("locked", Some(json!(true))), "Porte", &fr),
            "Verrouiller Porte"
        );
        assert_eq!(
            command_sentence(&c("percentage", Some(json!(60))), "Ventilo", &fr),
            "Régler Ventilo sur 60 (percentage)"
        );
        assert_eq!(
            command_sentence(
                &Command::SetHvacMode {
                    mode: "heat".into()
                },
                "Clim",
                &fr
            ),
            "Régler Clim en mode chauffage"
        );
    }

    #[test]
    fn string_values_follow_the_entity_kind() {
        let mut climate = ent("c", "Clim");
        climate.kind = Kind::Climate;
        let mut cover = ent("v", "Volet");
        cover.kind = Kind::Cover;
        let value = |lang, v: &str, e: &EntityView| value_words(lang, &json!(v), None, Some(e));
        assert_eq!(value(crate::i18n::Lang::Fr, "heat", &climate), "chauffage");
        assert_eq!(value(crate::i18n::Lang::Fr, "open", &cover), "ouvert");
        assert_eq!(value(crate::i18n::Lang::Fr, "whatever", &cover), "whatever");
    }

    #[test]
    fn signal_trigger_sentence_names_the_signal() {
        let entities: BTreeMap<_, _> = [("r1".to_owned(), ent("r1", "Remote"))].into();
        let ctx = ctx_of(&entities);
        let node = |signal: Option<&str>| Node {
            id: String::new(),
            name: None,
            position: None,
            kind: NodeKind::TriggerSignal(TriggerSignalParams {
                entity: "r1".into(),
                signal: signal.map(Into::into),
                ..Default::default()
            }),
            extra: Default::default(),
        };
        assert_eq!(sentence(&node(Some("on")), &ctx), "When Remote sends on");
        assert_eq!(sentence(&node(None), &ctx), "When Remote sends a signal");
        let fr = Ctx {
            lang: crate::i18n::Lang::Fr,
            ..ctx_of(&entities)
        };
        assert_eq!(sentence(&node(Some("on")), &fr), "Quand Remote envoie on");
        assert_eq!(sentence(&node(None), &fr), "Quand Remote envoie un signal");
        assert_eq!(
            sentence(&node(Some("brightness_up")), &ctx),
            "When Remote sends brightness_up"
        );
        assert_eq!(sentence(&node(Some("1_hold")), &ctx), "When button 1 held");
        assert_eq!(sentence(&node(Some("double")), &fr), "Quand appuyé 2×");
    }

    #[test]
    fn press_signals_read_as_button_words() {
        use crate::i18n::Lang::{En, Fr};
        use crate::i18n::flows::{press_label, signal_chip_label};
        let cases = [
            ("1_single", "button 1 pressed", "bouton 1 appuyé"),
            ("1_double", "button 1 pressed 2×", "bouton 1 appuyé 2×"),
            ("2_triple", "button 2 pressed 3×", "bouton 2 appuyé 3×"),
            ("3_hold", "button 3 held", "bouton 3 maintenu"),
            ("1_long", "button 1 held", "bouton 1 maintenu"),
            ("2_release", "button 2 released", "bouton 2 relâché"),
            ("double", "pressed 2×", "appuyé 2×"),
        ];
        for (signal, en, fr) in cases {
            assert_eq!(press_label(En, signal).as_deref(), Some(en), "{signal}");
            assert_eq!(press_label(Fr, signal).as_deref(), Some(fr), "{signal}");
        }
        assert_eq!(press_label(En, "brightness_up"), None);
        assert_eq!(press_label(En, "_hold"), None);
        assert_eq!(signal_chip_label(En, "1_hold"), "Button 1 held");
        assert_eq!(signal_chip_label(Fr, "single"), "Appuyé");
        assert_eq!(signal_chip_label(Fr, "brightness_up"), "brightness_up");
    }

    #[test]
    fn if_with_an_average_names_the_window() {
        let entities = BTreeMap::new();
        let node = |average: Option<&str>| Node {
            id: "n".into(),
            name: None,
            position: None,
            kind: NodeKind::If(nidoo_core::flow::IfParams {
                op: Op::Lte,
                value: Some(json!(19)),
                entity: Some("Temperature".into()),
                field: None,
                missing: None,
                average: average.map(Into::into),
                offset_minutes: None,
                result: None,
                extra: Default::default(),
            }),
            extra: Default::default(),
        };
        let en = ctx_of(&entities);
        let fr = Ctx {
            lang: crate::i18n::Lang::Fr,
            ..ctx_of(&entities)
        };
        assert_eq!(sentence(&node(None), &en), "Temperature \u{2264} 19?");
        assert_eq!(
            sentence(&node(Some("15m")), &en),
            "Temperature avg 15\u{a0}min \u{2264} 19?"
        );
        assert_eq!(
            sentence(&node(Some("1h30m")), &fr),
            "Temperature moy. 1\u{a0}h 30\u{a0}min \u{2264} 19\u{a0}?"
        );
    }
}
