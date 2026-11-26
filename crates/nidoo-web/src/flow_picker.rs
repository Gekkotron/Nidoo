//! Entries of the "Add" picker: what a query offers to put on the canvas. Pure, tested natively.

use nidoo_core::flow::TriggerSignalParams;
use nidoo_core::flow::{
    AlarmModeFilter, AskAnswer, AskParams, BackupRunParams, ClimateEcoParams, ClimateHoldParams,
    ClimateSetpoint, CommandParams, DelayParams, FlowEnableParams, FlowLabelParams, IfParams,
    LabelAction, LimitParams, MathParams, Node, NodeKind, NotifyParams, OffPeakEdge, Op, RuleOp,
    Scalar, SpeakParams, SunWhich, SwitchParams, SwitchRule, TimeBetweenParams, TriggerAlarmParams,
    TriggerManualParams, TriggerOffPeakParams, TriggerScheduleParams, TriggerSilentParams,
    TriggerStateParams, TriggerWakeUpParams,
};
use nidoo_core::model::{Command, Kind};
use serde_json::Value;

use crate::command_form::entity_actions;
use crate::flow_edit::fold_accents;
use crate::flow_text::{Ctx, sentence, subline};
use crate::format::{entity_title, kind_search_words, room_name};
use crate::i18n::Lang;
use crate::i18n::flows::{FlowText, flow_text};
use crate::if_value::{ValueInput, value_input};
use crate::screens::flow_canvas::sun::sun_params_from_preset;

/// The section of the picker an entry sits in, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    When,
    And,
    Then,
    Wait,
    Flow,
}

/// One card the picker can add.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub group: Group,
    /// The sentence the card will read.
    pub label: String,
    /// The row's second line: what a generic card does, or an entity card's room and device.
    pub sub: Option<String>,
    pub kind: NodeKind,
}

fn entry(group: Group, kind: NodeKind, ctx: &Ctx) -> Entry {
    let node = Node {
        id: String::new(),
        name: None,
        position: None,
        kind,
        extra: Default::default(),
    };
    Entry {
        group,
        label: sentence(&node, ctx),
        sub: subline(&node, ctx),
        kind: node.kind,
    }
}

/// What the picker appends to a card that targets a group entity: " (Group)" / " (Groupe)".
fn group_suffix(lang: Lang) -> String {
    format!(" {}", flow_text(lang, FlowText::GroupTag))
}

fn is_group_id(entity_id: &str) -> bool {
    entity_id.starts_with("group_")
}

fn command(entity: &str, command: Command) -> NodeKind {
    NodeKind::Command(CommandParams {
        entity: entity.to_owned(),
        command,
        value_from: None,
        extra: Default::default(),
    })
}

/// Whether a search lists `e`: a primary entity, hidden from the dashboard or not (hiding is a
/// display choice), but not one disabled by default or folded into another.
fn findable(e: &nidoo_api::EntityView) -> bool {
    e.category == nidoo_core::model::EntityCategory::Primary
        && e.enabled_by_default
        && e.part_of.is_none()
}

/// Extra words that find a generic card, in both languages: what other tools call a Manual card.
fn card_search_words(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::TriggerState(_) => {
            "trigger declencheur event evenement sensor capteur seuil threshold above below depasse pendant for duree"
        }
        NodeKind::TriggerManual(_) => "inject injecter button bouton",
        NodeKind::TriggerAlarm(_) => "alarm alarme armed armee security securite intrusion",
        NodeKind::FlowLabel(_) => "choice choix season saison tag",
        NodeKind::BackupRun(_) => "backup sauvegarde ftp",
        NodeKind::TriggerWakeUp(_) => "wake reveil alarm alarme morning matin",
        NodeKind::ClimateHold(_) => "heating chauffage thermostat temperature maintien hold",
        NodeKind::ClimateEco(_) => "eco heating chauffage thermostat setback",
        _ => "",
    }
}

/// The picker entries for `query`, ordered by [`Group`] then entity title. An empty query gives
/// only the generic cards; otherwise those whose label matches, plus When, And and Then cards for
/// every primary entity whose title, device name or room holds every word of the query
/// (ignoring case and accents), sorted by title ignoring case and accents.
pub fn entries(query: &str, ctx: &Ctx) -> Vec<Entry> {
    let needle = fold_accents(query.trim());
    let generic = [
        (
            Group::When,
            NodeKind::TriggerState(TriggerStateParams::default()),
            Some(FlowText::NodeTrigger),
            FlowText::DeviceSub,
        ),
        (
            Group::When,
            NodeKind::TriggerSchedule(TriggerScheduleParams {
                cron: "0 * * * *".into(),
                ..Default::default()
            }),
            Some(FlowText::Schedule),
            FlowText::ScheduleSub,
        ),
        (
            Group::When,
            NodeKind::TriggerSun(sun_params_from_preset(SunWhich::Sunset, None)),
            Some(FlowText::SunEvent),
            FlowText::SunEventSub,
        ),
        (
            Group::When,
            NodeKind::TriggerOffPeak(TriggerOffPeakParams {
                edge: OffPeakEdge::Start,
                extra: Default::default(),
            }),
            Some(FlowText::NodeOffPeak),
            FlowText::OffPeakSub,
        ),
        (
            Group::When,
            NodeKind::TriggerAlarm(TriggerAlarmParams {
                to: Some(AlarmModeFilter::Armed),
                ..Default::default()
            }),
            Some(FlowText::NodeAlarm),
            FlowText::AlarmSub,
        ),
        (
            Group::When,
            NodeKind::TriggerSilent(TriggerSilentParams {
                entity: String::new(),
                for_: "50m".into(),
                extra: Default::default(),
            }),
            Some(FlowText::NodeSilent),
            FlowText::SilentSub,
        ),
        (
            Group::When,
            NodeKind::TriggerManual(TriggerManualParams::default()),
            Some(FlowText::NodeManual),
            FlowText::ManualSub,
        ),
        (
            Group::When,
            NodeKind::TriggerWakeUp(TriggerWakeUpParams {
                before_minutes: 45,
                // A new wake-up card starts catching up: a hub that was off at the time still
                // warms the house for the alarm.
                catch_up_minutes: Some(60),
                ..Default::default()
            }),
            Some(FlowText::NodeWakeUp),
            FlowText::NodeWakeUpSub,
        ),
        (
            Group::And,
            NodeKind::If(IfParams {
                op: Op::IsDark,
                value: None,
                entity: None,
                field: None,
                missing: None,
                average: None,
                offset_minutes: None,
                result: None,
                extra: Default::default(),
            }),
            Some(FlowText::WhenDark),
            FlowText::WhenDarkSub,
        ),
        (
            Group::And,
            NodeKind::If(IfParams {
                op: Op::IsOffPeak,
                value: None,
                entity: None,
                field: None,
                missing: None,
                average: None,
                offset_minutes: None,
                result: None,
                extra: Default::default(),
            }),
            Some(FlowText::IsOffPeak),
            FlowText::IsOffPeakSub,
        ),
        (
            Group::And,
            NodeKind::Switch(SwitchParams {
                rules: vec![SwitchRule {
                    op: RuleOp::Eq,
                    value: Value::Bool(true),
                    to: None,
                    extra: Default::default(),
                }],
                otherwise: true,
                ..Default::default()
            }),
            Some(FlowText::SwitchCard),
            FlowText::SwitchSub,
        ),
        (
            Group::And,
            NodeKind::Math(MathParams::default()),
            Some(FlowText::MathCard),
            FlowText::MathSub,
        ),
        (
            Group::And,
            NodeKind::TimeBetween(TimeBetweenParams {
                from: "07:00".into(),
                to: "22:00".into(),
                extra: Default::default(),
            }),
            Some(FlowText::TimeCard),
            FlowText::TimeSub,
        ),
        (
            Group::Wait,
            NodeKind::Delay(DelayParams {
                for_: "5m".into(),
                ..Default::default()
            }),
            Some(FlowText::WaitCard),
            FlowText::WaitSub,
        ),
        (
            Group::Wait,
            NodeKind::Limit(LimitParams {
                max: 4,
                per: "24h".into(),
                extra: Default::default(),
            }),
            Some(FlowText::NodeLimit),
            FlowText::LimitSub,
        ),
        (
            Group::Then,
            NodeKind::Notify(NotifyParams {
                // A default written into the stored flow: the language active now.
                title: flow_text(ctx.lang, FlowText::DefaultNotifyTitle).into(),
                ..Default::default()
            }),
            Some(FlowText::Notify),
            FlowText::NotifySub,
        ),
        (
            Group::Then,
            NodeKind::Speak(SpeakParams {
                // A default written into the stored flow: the language active now.
                text: flow_text(ctx.lang, FlowText::DefaultSpeakText).into(),
                room: None,
            }),
            Some(FlowText::Speak),
            FlowText::SpeakSub,
        ),
        (
            Group::Then,
            NodeKind::Ask(AskParams {
                // Defaults written into the stored flow: the language active now.
                text: flow_text(ctx.lang, FlowText::DefaultAskText).into(),
                answers: [FlowText::DefaultAnswerYes, FlowText::DefaultAnswerNo]
                    .into_iter()
                    .map(|t| AskAnswer {
                        label: flow_text(ctx.lang, t).into(),
                        words: Vec::new(),
                    })
                    .collect(),
                ..Default::default()
            }),
            Some(FlowText::Ask),
            FlowText::AskSub,
        ),
        (
            Group::Then,
            NodeKind::ClimateHold(ClimateHoldParams {
                zones: Vec::new(),
                setpoint: ClimateSetpoint::Temp(20.0),
                mode: None,
                minutes: Some(120),
                until: None,
                extra: Default::default(),
            }),
            Some(FlowText::NodeClimateHold),
            FlowText::NodeClimateHoldSub,
        ),
        (
            Group::Then,
            NodeKind::ClimateEco(ClimateEcoParams {
                zones: Vec::new(),
                minutes: None,
                until: Some("18:00".into()),
                extra: Default::default(),
            }),
            Some(FlowText::NodeClimateEco),
            FlowText::NodeClimateEcoSub,
        ),
        (
            Group::Flow,
            NodeKind::FlowEnable(FlowEnableParams {
                enabled: false,
                flow: None,
                ..Default::default()
            }),
            None,
            FlowText::DisableSelfSub,
        ),
        (
            Group::Flow,
            NodeKind::FlowEnable(FlowEnableParams {
                enabled: true,
                flow: None,
                ..Default::default()
            }),
            Some(FlowText::EnableFlow),
            FlowText::EnableFlowSub,
        ),
        (
            Group::Flow,
            NodeKind::FlowLabel(FlowLabelParams {
                label: String::new(),
                action: LabelAction::Pick,
                ..Default::default()
            }),
            Some(FlowText::NodeFlowLabel),
            FlowText::NodeFlowLabelSub,
        ),
        (
            Group::Flow,
            NodeKind::BackupRun(BackupRunParams::default()),
            Some(FlowText::NodeBackupRun),
            FlowText::NodeBackupRunSub,
        ),
    ];
    // The label a generic card reads in `lang`; the sentence of a card with no target reads
    // poorly, so the picker names it.
    let label_in = |lang: Lang, kind: &NodeKind, label: Option<FlowText>| match label {
        Some(l) => flow_text(lang, l).to_owned(),
        None => {
            let other = Ctx {
                entities: ctx.entities,
                areas: ctx.areas,
                flows: ctx.flows,
                labels: ctx.labels,
                lang,
            };
            entry(Group::Flow, kind.clone(), &other).label
        }
    };
    let mut out: Vec<Entry> = generic
        .into_iter()
        // Typing the English or the French word finds a card, whatever the UI language.
        .filter(|(_, kind, label, _)| {
            needle.is_empty()
                || [Lang::En, Lang::Fr]
                    .into_iter()
                    .any(|l| fold_accents(&label_in(l, kind, *label)).contains(&needle))
                || card_search_words(kind)
                    .split(' ')
                    .any(|w| w.contains(&needle))
        })
        .map(|(group, kind, label, sub)| {
            let mut e = entry(group, kind, ctx);
            e.sub = Some(flow_text(ctx.lang, sub).to_owned());
            if let Some(label) = label {
                e.label = flow_text(ctx.lang, label).to_owned();
            }
            e
        })
        .collect();

    if !needle.is_empty() {
        let words: Vec<&str> = needle.split_whitespace().collect();
        let mut matches: Vec<(String, &_, String)> = ctx
            .entities
            .values()
            .filter(|e| findable(e))
            .map(|e| {
                let title = fold_accents(&entity_title(ctx.lang, e));
                let [en, fr] = kind_search_words(e);
                let hay = fold_accents(&format!(
                    "{title} {} {} {en} {fr}",
                    e.device_name.as_deref().unwrap_or_default(),
                    room_name(ctx.areas, &e.area).unwrap_or_default()
                ));
                (title, e, hay)
            })
            .collect();
        matches.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, e, hay) in matches {
            let first = out.len();
            let current = Scalar::of_state(&e.state.value);
            out.push(entry(
                Group::When,
                NodeKind::TriggerState(TriggerStateParams {
                    entity: e.id.clone(),
                    ..Default::default()
                }),
                ctx,
            ));
            // A sensor's values are its events: "door opens", "motion detected".
            if e.kind == Kind::BinarySensor
                && let ValueInput::List(values) = value_input(ctx.lang, Some(e))
            {
                for choice in values {
                    out.push(entry(
                        Group::When,
                        NodeKind::TriggerState(TriggerStateParams {
                            entity: e.id.clone(),
                            to: Some(choice.value),
                            ..Default::default()
                        }),
                        ctx,
                    ));
                }
            }
            out.push(entry(
                Group::And,
                NodeKind::If(IfParams {
                    entity: Some(e.id.clone()),
                    field: None,
                    op: Op::Eq,
                    value: Some(current.as_ref().map_or(Value::Bool(true), Scalar::to_json)),
                    missing: None,
                    average: None,
                    offset_minutes: None,
                    result: None,
                    extra: Default::default(),
                }),
                ctx,
            ));
            out.push(entry(
                Group::And,
                NodeKind::Switch(SwitchParams {
                    entity: Some(e.id.clone()),
                    rules: vec![SwitchRule {
                        op: RuleOp::Eq,
                        value: current.as_ref().map_or(Value::Bool(true), Scalar::to_json),
                        to: None,
                        extra: Default::default(),
                    }],
                    otherwise: true,
                    ..Default::default()
                }),
                ctx,
            ));
            if let Some(caps) = e
                .capabilities
                .generic
                .as_ref()
                .filter(|c| !c.signals.is_empty())
            {
                let signals = caps
                    .signals
                    .iter()
                    .map(|s| Some(s.clone()))
                    .chain(std::iter::once(None));
                for signal in signals {
                    out.push(entry(
                        Group::When,
                        NodeKind::TriggerSignal(TriggerSignalParams {
                            entity: e.id.clone(),
                            signal,
                            ..Default::default()
                        }),
                        ctx,
                    ));
                }
            }
            for action in entity_actions(e) {
                out.push(entry(Group::Then, command(&e.id, action.command), ctx));
            }
            if is_group_id(&e.id) {
                for entry in &mut out[first..] {
                    entry.label.push_str(&group_suffix(ctx.lang));
                }
            }
            // A word the entity lacks may name what a card does ("ferm" for "Fermer"): keep
            // the cards whose label holds it, drop the entity when none does.
            if !words.iter().all(|w| hay.contains(w)) {
                let cards = out.split_off(first);
                out.extend(cards.into_iter().filter(|c| {
                    let label = fold_accents(&c.label);
                    words.iter().all(|w| hay.contains(w) || label.contains(w))
                }));
            }
        }
    }
    out.sort_by_key(|e| e.group);
    out
}

#[cfg(test)]
mod tests;
