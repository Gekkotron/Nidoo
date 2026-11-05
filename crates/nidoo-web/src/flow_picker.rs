//! Entries of the "Add" picker: what a query offers to put on the canvas. Pure, tested natively.

use nidoo_core::flow::TriggerSignalParams;
use nidoo_core::flow::{
    AskAnswer, AskParams, CommandParams, DelayParams, FlowEnableParams, IfParams, LimitParams,
    MathParams, Node, NodeKind, NotifyParams, OffPeakEdge, Op, RuleOp, Scalar, SpeakParams,
    SunWhich, SwitchParams, SwitchRule, TimeBetweenParams, TriggerManualParams,
    TriggerOffPeakParams, TriggerScheduleParams, TriggerSilentParams, TriggerStateParams,
};
use nidoo_core::model::Command;
use serde_json::Value;

use crate::command_form::entity_actions;
use crate::flow_edit::fold_accents;
use crate::flow_text::{Ctx, sentence, subline};
use crate::format::{entity_title, kind_search_words, room_name};
use crate::i18n::Lang;
use crate::i18n::flows::{FlowText, flow_text};
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

/// Extra words that find a generic card, in both languages: what other tools call a Manual card.
fn card_search_words(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::TriggerManual(_) => "inject injecter button bouton",
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
        let mut matches: Vec<(String, &_)> = ctx
            .entities
            .values()
            .filter(|e| e.is_primary())
            .map(|e| (fold_accents(&entity_title(ctx.lang, e)), e))
            .filter(|(title, e)| {
                let [en, fr] = kind_search_words(e);
                let hay = fold_accents(&format!(
                    "{title} {} {} {en} {fr}",
                    e.device_name.as_deref().unwrap_or_default(),
                    room_name(ctx.areas, &e.area).unwrap_or_default()
                ));
                words.iter().all(|w| hay.contains(w))
            })
            .collect();
        matches.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, e) in matches {
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
        }
    }
    out.sort_by_key(|e| e.group);
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use nidoo_api::{AreaView, EntityView};
    use nidoo_core::model::{
        AlarmMode, Capabilities, EntityCategory, EntityState, Kind, StateValue, UiOverride,
    };
    use serde_json::json;

    use super::*;

    fn ent(id: &str, name: &str, kind: Kind) -> EntityView {
        EntityView {
            id: id.into(),
            device_id: None,
            name: name.into(),
            device_name: None,
            label: None,
            kind,
            component: kind.component().to_string(),
            protocol_kind: kind,
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

    fn house(list: Vec<EntityView>) -> BTreeMap<String, EntityView> {
        list.into_iter().map(|e| (e.id.clone(), e)).collect()
    }

    fn run(query: &str, entities: &BTreeMap<String, EntityView>) -> Vec<Entry> {
        let areas: Vec<AreaView> = Vec::new();
        let ctx = Ctx {
            entities,
            areas: &areas,
            flows: &[],
            lang: crate::i18n::Lang::En,
        };
        entries(query, &ctx)
    }

    #[test]
    fn the_kind_word_finds_entities_in_either_language() {
        let entities = house(vec![ent("c1", "Shutter living", Kind::Cover)]);
        let areas: Vec<AreaView> = Vec::new();
        for (lang, query) in [
            (crate::i18n::Lang::Fr, "cover"),
            (crate::i18n::Lang::En, "volet"),
        ] {
            let ctx = Ctx {
                entities: &entities,
                areas: &areas,
                flows: &[],
                lang,
            };
            let got = entries(query, &ctx);
            assert!(
                got.iter().any(|e| e.label.contains("Shutter living")),
                "{query}: {:?}",
                got.iter().map(|e| e.label.clone()).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn generic_cards_read_and_search_in_both_languages() {
        let entities = BTreeMap::new();
        let areas: Vec<AreaView> = Vec::new();
        let ctx = |lang| Ctx {
            entities: &entities,
            areas: &areas,
            flows: &[],
            lang,
        };
        let fr = entries("", &ctx(crate::i18n::Lang::Fr));
        let labels: Vec<_> = fr.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels[0], "Programmation");
        assert!(labels.contains(&"Quand il fait nuit"));
        assert!(
            matches!(&fr[10].kind, NodeKind::Notify(p) if p.title == "Notification"),
            "the stored default follows the language at creation"
        );
        // The English word finds the card on the French UI, and the French word on the English UI.
        let wait_fr = entries("wait", &ctx(crate::i18n::Lang::Fr));
        assert_eq!(wait_fr.len(), 1);
        assert_eq!(wait_fr[0].label, "Attente");
        let wait_en = entries("attente", &ctx(crate::i18n::Lang::En));
        assert_eq!(wait_en.len(), 1);
        assert_eq!(wait_en[0].label, "Wait");
    }

    #[test]
    fn group_tag_follows_the_language() {
        let entities = house(vec![ent("group_a", "Salon", Kind::Light)]);
        let areas: Vec<AreaView> = Vec::new();
        let ctx = Ctx {
            entities: &entities,
            areas: &areas,
            flows: &[],
            lang: crate::i18n::Lang::Fr,
        };
        let got = entries("salon", &ctx);
        assert!(
            got.iter().all(|e| e.label.ends_with(" (Groupe)")),
            "{got:?}"
        );
        assert!(got.iter().any(|e| e.label == "Allumer Salon (Groupe)"));
    }

    fn count(entries: &[Entry], group: Group) -> usize {
        entries.iter().filter(|e| e.group == group).count()
    }

    #[test]
    fn empty_query_has_generic_entries() {
        let entities = house(vec![ent("l1", "Lumière table", Kind::Light)]);
        let got = run("", &entities);
        let labels: Vec<_> = got.iter().map(|e| (e.group, e.label.as_str())).collect();
        assert_eq!(
            labels,
            [
                (Group::When, "Schedule"),
                (Group::When, "Sun event"),
                (Group::When, "Off-peak"),
                (Group::When, "Silent for"),
                (Group::When, "Manual"),
                (Group::And, "When it's dark"),
                (Group::And, "Is it off-peak?"),
                (Group::And, "Switch"),
                (Group::And, "Math"),
                (Group::And, "Time of day"),
                (Group::Then, "Notify"),
                (Group::Then, "Speak"),
                (Group::Then, "Ask"),
                (Group::Wait, "Wait"),
                (Group::Wait, "Limit"),
                (Group::Flow, "Disable this flow"),
                (Group::Flow, "Enable a flow"),
            ]
        );
        let subs: Vec<_> = got.iter().map(|e| e.sub.as_deref()).collect();
        assert_eq!(
            subs,
            [
                Some("At a set time, or on a repeat"),
                Some("Sunrise, sunset, dusk or dawn"),
                Some("When off-peak starts or ends"),
                Some("When an entity stops reporting"),
                Some("Start by hand from the ▶ button"),
                Some("Between dusk and dawn"),
                Some("Off-peak hours right now"),
                Some("Several exits, one per rule"),
                Some("Compute a number from your devices"),
                Some("Inside or outside two times"),
                Some("A push to your phone"),
                Some("Said out loud on the speakers"),
                Some("Asks out loud and waits for the answer"),
                Some("A fixed or random delay"),
                Some("Lets only so many runs through"),
                Some("Stops it from running again"),
                Some("Switches another flow on or off"),
            ]
        );
        assert!(matches!(
            &got[0].kind,
            NodeKind::TriggerSchedule(p) if p.cron == "0 * * * *"
        ));
        assert!(matches!(&got[8].kind, NodeKind::Math(p) if p.name.is_empty()));
        assert!(matches!(&got[10].kind, NodeKind::Notify(p) if p.title == "Notification"));
        assert!(matches!(
            &got[11].kind,
            NodeKind::Speak(p) if p.text == "Hello" && p.room.is_none()
        ));
        assert!(matches!(
            &got[12].kind,
            NodeKind::Ask(p) if p.answers.len() == 2 && p.room.is_none()
                && p.answers[0].label == "Yes" && p.answers[1].label == "No"
        ));
        assert!(matches!(&got[13].kind, NodeKind::Delay(p) if p.for_ == "5m"));
        assert!(matches!(
            &got[7].kind,
            NodeKind::Switch(p) if p.otherwise && p.entity.is_none()
                && p.rules.len() == 1 && p.rules[0].op == RuleOp::Eq
                && p.rules[0].value == Value::Bool(true)
        ));
        assert!(matches!(
            &got[9].kind,
            NodeKind::TimeBetween(p) if p.from == "07:00" && p.to == "22:00"
        ));
    }

    #[test]
    fn library_lists_silent_and_limit() {
        let entities = BTreeMap::new();
        let got = run("", &entities);
        let silent = got
            .iter()
            .find(|e| matches!(e.kind, NodeKind::TriggerSilent(_)))
            .expect("a silent entry");
        assert_eq!(silent.group, Group::When);
        assert_eq!(silent.label, "Silent for");
        assert!(matches!(
            &silent.kind,
            NodeKind::TriggerSilent(p) if p.entity.is_empty() && p.for_ == "50m"
        ));
        let limit = got
            .iter()
            .find(|e| matches!(e.kind, NodeKind::Limit(_)))
            .expect("a limit entry");
        assert_eq!(limit.group, Group::Wait);
        assert_eq!(limit.label, "Limit");
        assert!(matches!(
            &limit.kind,
            NodeKind::Limit(p) if p.max == 4 && p.per == "24h"
        ));
        // Either language's word finds the cards.
        assert_eq!(run("silencieux", &entities).len(), 1);
        assert_eq!(run("limite", &entities).len(), 1);
    }

    #[test]
    fn manual_card_is_found_by_its_names_in_either_language() {
        let entities = BTreeMap::new();
        for query in ["manual", "inject", "button", "manuel", "bouton"] {
            let got = run(query, &entities);
            assert_eq!(got.len(), 1, "{query}: {got:?}");
            assert_eq!(got[0].group, Group::When);
            assert!(matches!(&got[0].kind, NodeKind::TriggerManual(p) if p.value.is_none()));
        }
        let empty = run("", &entities);
        assert!(
            empty
                .iter()
                .any(|e| e.group == Group::When && matches!(e.kind, NodeKind::TriggerManual(_)))
        );
    }

    #[test]
    fn library_lists_switch_and_time() {
        let entities = BTreeMap::new();
        let areas: Vec<AreaView> = Vec::new();
        let ctx = |lang| Ctx {
            entities: &entities,
            areas: &areas,
            flows: &[],
            lang,
        };
        let fr = entries("", &ctx(crate::i18n::Lang::Fr));
        let labels: Vec<_> = fr.iter().map(|e| e.label.as_str()).collect();
        assert!(labels.contains(&"Commuter") && labels.contains(&"Plage horaire"));
        let sub = |label: &str| {
            fr.iter()
                .find(|e| e.label == label)
                .and_then(|e| e.sub.clone())
        };
        assert_eq!(
            sub("Commuter").as_deref(),
            Some("Plusieurs sorties, une par règle")
        );
        assert_eq!(
            sub("Plage horaire").as_deref(),
            Some("Dans ou hors d'une plage horaire")
        );
        // Either language's word finds the card.
        let found = entries("plage", &ctx(crate::i18n::Lang::En));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].label, "Time of day");
        assert_eq!(
            entries("commuter", &ctx(crate::i18n::Lang::En))[0].label,
            "Switch"
        );
    }

    #[test]
    fn an_entity_gets_a_switch_card_next_to_its_if() {
        let entities = house(vec![ent("l1", "Lumière table", Kind::Light)]);
        let got = run("table", &entities);
        let switch = got
            .iter()
            .find(|e| matches!(&e.kind, NodeKind::Switch(_)))
            .expect("a switch entry");
        assert_eq!(switch.group, Group::And);
        assert_eq!(switch.label, "Switch on Lumière table");
        assert!(matches!(
            &switch.kind,
            NodeKind::Switch(p) if p.entity.as_deref() == Some("l1") && p.otherwise && p.rules.len() == 1
        ));
    }

    #[test]
    fn entity_entries_carry_the_subline() {
        let entities = house(vec![ent("l1", "Lumière table", Kind::Light)]);
        let areas = vec![AreaView {
            id: "salon".into(),
            name: "Salon".into(),
            icon: None,
            order: None,
            entity_count: 1,
            primary_count: 1,
            group_names: Default::default(),
            group_favorites: Default::default(),
        }];
        let ctx = Ctx {
            entities: &entities,
            areas: &areas,
            flows: &[],
            lang: crate::i18n::Lang::En,
        };
        let got = entries("table", &ctx);
        let entity: Vec<_> = got.iter().filter(|e| e.label.contains("Lumière")).collect();
        assert!(!entity.is_empty());
        for e in entity {
            assert_eq!(e.sub.as_deref(), Some("Salon"), "{}", e.label);
        }
    }

    #[test]
    fn picker_offers_lock_controls() {
        let mut lock = ent("k1", "Front door", Kind::Generic);
        lock.component = "lock".into();
        lock.capabilities.generic = Some(nidoo_core::types::GenericCaps {
            controls: vec!["locked".into(), "open".into()],
            ..Default::default()
        });
        let cmds: Vec<Command> = entity_actions(&lock)
            .into_iter()
            .map(|a| a.command)
            .collect();
        assert!(cmds.contains(&Command::Control {
            control: "locked".into(),
            value: Some(json!(true))
        }));
        assert!(cmds.contains(&Command::Control {
            control: "open".into(),
            value: None
        }));
    }

    #[test]
    fn picker_offers_alarm_disarm_and_announced_modes() {
        let mut panel = ent("a1", "Alarm", Kind::AlarmPanel);
        panel.capabilities.alarm_modes = [AlarmMode::Away, AlarmMode::Night].into();
        assert_eq!(
            entity_actions(&panel)
                .into_iter()
                .map(|a| a.command)
                .collect::<Vec<_>>(),
            vec![
                Command::Disarm { code: None },
                Command::Arm {
                    mode: AlarmMode::Away,
                    code: None
                },
                Command::Arm {
                    mode: AlarmMode::Night,
                    code: None
                },
            ]
        );
        panel.capabilities.alarm_modes.clear();
        // Discovery fills the modes; a panel that announces none offers only Disarm.
        assert_eq!(entity_actions(&panel).len(), 1);
    }

    #[test]
    fn alarm_cards_read_in_both_languages() {
        let mut panel = ent("a1", "Alarme", Kind::AlarmPanel);
        panel.capabilities.alarm_modes = [AlarmMode::Away].into();
        let entities = house(vec![panel]);
        let areas: Vec<AreaView> = Vec::new();
        for (lang, want) in [
            (
                crate::i18n::Lang::En,
                ["Disarm Alarme", "Arm Alarme (away)"],
            ),
            (
                crate::i18n::Lang::Fr,
                ["Désactiver Alarme", "Activer Alarme (absent)"],
            ),
        ] {
            let ctx = Ctx {
                entities: &entities,
                areas: &areas,
                flows: &[],
                lang,
            };
            let labels: Vec<String> = entries("alarme", &ctx)
                .into_iter()
                .filter(|e| e.group == Group::Then)
                .map(|e| e.label)
                .collect();
            assert_eq!(labels, want);
        }
    }

    #[test]
    fn picker_offers_remote_signals() {
        let mut remote = ent("r1", "Remote", Kind::Generic);
        remote.component = "device_automation".into();
        remote.capabilities.generic = Some(nidoo_core::types::GenericCaps {
            signals: vec!["on".into(), "off".into()],
            ..Default::default()
        });
        let got = run("remote", &house(vec![remote]));
        let signals: Vec<Option<String>> = got
            .iter()
            .filter_map(|e| match &e.kind {
                NodeKind::TriggerSignal(p) => Some(p.signal.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(signals, [Some("on".into()), Some("off".into()), None]);
    }

    #[test]
    fn table_query_lists_light_entries() {
        let mut light = ent("l1", "Lumière table", Kind::Light);
        light.state.value = StateValue::OnOff(true);
        let entities = house(vec![light, ent("l2", "Bedroom", Kind::Light)]);
        let got = run("table", &entities);
        assert_eq!(count(&got, Group::When), 1);
        // An `if` and a switch on the entity.
        assert_eq!(count(&got, Group::And), 2);
        assert_eq!(count(&got, Group::Then), 3);
        let label = |g| got.iter().find(|e| e.group == g).unwrap().label.clone();
        assert_eq!(label(Group::When), "When Lumière table changes");
        assert_eq!(label(Group::And), "Lumière table is on?");
        let then: Vec<_> = got
            .iter()
            .filter(|e| e.group == Group::Then)
            .map(|e| e.label.as_str())
            .collect();
        assert_eq!(
            then,
            [
                "Turn on Lumière table",
                "Turn off Lumière table",
                "Toggle Lumière table"
            ]
        );
        let groups: Vec<_> = got.iter().map(|e| e.group).collect();
        assert!(groups.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn accent_insensitive() {
        let entities = house(vec![ent("l1", "Lumière", Kind::Light)]);
        assert_eq!(count(&run("lumiere", &entities), Group::When), 1);
        assert_eq!(count(&run("LUMIÈRE", &entities), Group::When), 1);
    }

    #[test]
    fn device_name_and_words_match() {
        let weather = |id: &str, name: &str| {
            let mut e = ent(id, name, Kind::Sensor);
            e.device_name = Some("Weather".into());
            e
        };
        let entities = house(vec![
            weather("w1", "Weather Temperature"),
            weather("w2", "Weather Rain next hour"),
            ent("l1", "Lumière", Kind::Light),
        ]);
        assert_eq!(count(&run("weather", &entities), Group::When), 2);
        let rain = run("weather rain", &entities);
        assert_eq!(count(&rain, Group::When), 1);
        assert_eq!(
            rain.iter().find(|e| e.group == Group::When).unwrap().label,
            "When Rain next hour changes"
        );
    }

    #[test]
    fn lists_every_matching_entity() {
        let list = (0..12)
            .map(|i| ent(&format!("l{i}"), &format!("Lampe {i:02}"), Kind::Light))
            .collect();
        let got = run("lampe", &house(list));
        assert_eq!(count(&got, Group::When), 12);
        assert_eq!(count(&got, Group::Then), 36);
    }

    #[test]
    fn room_name_matches() {
        let mut bedroom = ent("c1", "Shutter", Kind::Cover);
        bedroom.area = "bedroom".into();
        let entities = house(vec![bedroom, ent("c2", "Volet nord", Kind::Cover)]);
        let areas = vec![AreaView {
            id: "bedroom".into(),
            name: "Chambre".into(),
            icon: None,
            order: None,
            entity_count: 1,
            primary_count: 1,
            group_names: Default::default(),
            group_favorites: Default::default(),
        }];
        let ctx = Ctx {
            entities: &entities,
            areas: &areas,
            flows: &[],
            lang: crate::i18n::Lang::En,
        };
        let got = entries("chambre volet", &ctx);
        assert_eq!(count(&got, Group::When), 1);
        assert!(got.iter().all(|e| !e.label.contains("Volet nord")));
    }

    #[test]
    fn sorts_ignoring_case_and_accents() {
        let entities = house(vec![
            ent("l1", "lampe b", Kind::Light),
            ent("l2", "Lampe C", Kind::Light),
            ent("l3", "Lampé a", Kind::Light),
        ]);
        let order: Vec<_> = run("lampe", &entities)
            .into_iter()
            .filter_map(|e| match e.kind {
                NodeKind::TriggerState(p) => Some(p.entity),
                _ => None,
            })
            .collect();
        assert_eq!(order, ["l3", "l1", "l2"]);
    }

    #[test]
    fn group_entries_carry_the_group_tag() {
        let entities = house(vec![
            ent("group_living", "Living lights", Kind::Light),
            ent("l1", "Living lamp", Kind::Light),
        ]);
        let got = run("living", &entities);
        let tagged = |e: &Entry| e.label.ends_with(" (Group)");
        let group_entries: Vec<_> = got
            .iter()
            .filter(|e| e.label.contains("Living lights"))
            .collect();
        assert!(!group_entries.is_empty() && group_entries.iter().all(|e| tagged(e)));
        assert!(
            got.iter()
                .filter(|e| e.label.contains("Living lamp"))
                .all(|e| !tagged(e))
        );
    }

    #[test]
    fn sensor_has_no_actions() {
        let entities = house(vec![ent("s1", "Entrée", Kind::Sensor)]);
        let got = run("entree", &entities);
        assert_eq!(count(&got, Group::When), 1);
        assert_eq!(count(&got, Group::And), 2);
        assert_eq!(count(&got, Group::Then), 0);
    }

    #[test]
    fn hidden_entities_excluded() {
        let mut hidden = ent("l1", "Lumière", Kind::Light);
        hidden.ui.hidden = true;
        let mut diagnostic = ent("l2", "Lumière diag", Kind::Light);
        diagnostic.category = EntityCategory::Diagnostic;
        let entities = house(vec![hidden, diagnostic]);
        assert_eq!(run("lumiere", &entities).len(), 0);
    }

    #[test]
    fn generic_entries_filter_by_query() {
        let got = run("wai", &BTreeMap::new());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].group, Group::Wait);
    }

    #[test]
    fn number_and_climate_defaults() {
        let mut n = ent("n1", "Seuil", Kind::Number);
        n.state.value = StateValue::Number(42.0);
        let entities = house(vec![n, ent("c1", "Clim", Kind::Climate)]);
        let got = run("seuil", &entities);
        let NodeKind::Command(p) = &got.last().unwrap().kind else {
            panic!()
        };
        assert_eq!(p.command, Command::SetNumber { value: 42.0 });
        let got = run("clim", &entities);
        let NodeKind::Command(p) = &got.last().unwrap().kind else {
            panic!()
        };
        assert_eq!(p.command, Command::SetTemperature { temperature: 20.0 });
    }
}
