//! Entities with an energy role to the Energy screen: the nodes, their flows, the house derived
//! from its inputs and the throttled consumer order. A port of the app's `EnergyFlowBuilder.kt`
//! without history, cache or link state.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::meters::{EnergyEntity, EnergyMeters};
use super::units::{ConsumerIcon, EnergyRole, EnergyUnits, PowerUnits};

/// Consumers are re-ordered at most this often.
pub const REORDER_MS: u64 = 5_000;
/// Below this many watts an edge is idle.
pub const IDLE_W: f64 = 10.0;
/// Self-consumption is hidden under this much solar.
pub const SELF_CONSUMPTION_MIN_W: f64 = 50.0;
/// Consumer slots on the diagram arc; with more, the last slot folds the rest into "more".
pub const ARC_SLOTS: usize = 3;
/// Consumer node ids never collide with the fixed role ids ("house", "solar"...), whatever the
/// entity's id.
pub const CONSUMER_PREFIX: &str = "consumer:";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Solar,
    Grid,
    Battery,
    House,
    Consumer,
    More,
    Water,
}

/// The direction of a node's edge, relative to the house.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    In,
    Out,
    Idle,
}

/// Worst last: an aggregate takes the max.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NodeStatus {
    Live,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnergyNode {
    /// "solar", "grid", "battery", "house", "water", "more", or `consumer:` + the entity id.
    pub id: String,
    pub kind: NodeKind,
    /// A consumer's name; `None` for role nodes, which the UI labels with the role name.
    pub name: Option<String>,
    /// Watts, always >= 0 (`flow` carries the direction); water: its own unit. `None` when unknown.
    pub value: Option<f64>,
    /// Water only: the unit as reported.
    pub unit: Option<String>,
    pub flow: Flow,
    pub status: NodeStatus,
    /// `More` only: how many consumers it folds.
    pub count: usize,
    /// The entities behind this node, sorted by name then id; empty for a derived house.
    pub sensor_ids: Vec<String>,
    /// Consumers only: the user's chosen glyph; `None` draws the default plug.
    pub icon: Option<ConsumerIcon>,
    /// Lifetime energy in kWh summed over the node's paired meters; `None` when unknown or
    /// `has_meter` is false.
    pub total_kwh: Option<f64>,
    /// Worst status of the paired meters.
    pub total_status: NodeStatus,
    /// At least one of the node's sensors has an energy meter.
    pub has_meter: bool,
    /// Water only: the meter index, in `water_total_unit`; `None` without one.
    pub water_total: Option<f64>,
    pub water_total_unit: Option<String>,
}

impl EnergyNode {
    fn new(id: impl Into<String>, kind: NodeKind) -> EnergyNode {
        EnergyNode {
            id: id.into(),
            kind,
            name: None,
            value: None,
            unit: None,
            flow: Flow::Idle,
            status: NodeStatus::Live,
            count: 0,
            sensor_ids: Vec::new(),
            icon: None,
            total_kwh: None,
            total_status: NodeStatus::Live,
            has_meter: false,
            water_total: None,
            water_total_unit: None,
        }
    }
}

/// The consumer order last shown, and when it was computed.
#[derive(Clone, Debug, PartialEq)]
pub struct ConsumerOrder {
    pub ids: Vec<String>,
    pub at_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnergyHint {
    CheckSign,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct EnergyUi {
    /// Nothing discovered yet.
    pub loading: bool,
    /// Entities exist but none has an energy role.
    pub empty: bool,
    /// Drawn nodes: roles, up to 3 consumer slots (2 + More when folded), water when there is no
    /// battery.
    pub diagram: Vec<EnergyNode>,
    /// The cards under the diagram: role nodes, every consumer, water.
    pub cards: Vec<EnergyNode>,
    /// Every consumer in display order, for the "+N" list.
    pub consumers: Vec<EnergyNode>,
    /// 0..=100, rounded down; `None` when hidden.
    pub self_consumption: Option<u8>,
    pub hint: Option<EnergyHint>,
    pub order: Option<ConsumerOrder>,
}

impl EnergyUi {
    pub fn node(&self, id: &str) -> Option<&EnergyNode> {
        self.diagram
            .iter()
            .chain(self.cards.iter())
            .find(|n| n.id == id)
    }
}

/// One role's value in watts (`None` = unknown) and its worst status.
#[derive(Clone, Copy)]
struct Reading {
    watts: Option<f64>,
    status: NodeStatus,
}

/// A node's lifetime energy.
struct Total {
    has_meter: bool,
    kwh: Option<f64>,
    status: NodeStatus,
}

const NO_TOTAL: Total = Total {
    has_meter: false,
    kwh: None,
    status: NodeStatus::Live,
};

type ByRole<'a> = BTreeMap<EnergyRole, Vec<&'a EnergyEntity>>;

pub struct EnergyFlow;

impl EnergyFlow {
    pub fn build(
        entities: &[EnergyEntity],
        now_ms: u64,
        previous: Option<&ConsumerOrder>,
    ) -> EnergyUi {
        if entities.is_empty() {
            return EnergyUi {
                loading: true,
                ..EnergyUi::default()
            };
        }
        let mut by_role: ByRole = BTreeMap::new();
        for e in entities {
            if let Some(r) = e.role {
                by_role.entry(r).or_default().push(e);
            }
        }
        if by_role.is_empty() {
            return EnergyUi {
                empty: true,
                ..EnergyUi::default()
            };
        }
        let of = |r: EnergyRole| by_role.get(&r).map(Vec::as_slice).unwrap_or(&[]);

        let solar = sum(of(EnergyRole::Solar)).map(|r| Reading {
            watts: r.watts.map(|w| w.max(0.0)),
            ..r
        });
        let grid = grid(sum(of(EnergyRole::Grid)), sum(of(EnergyRole::GridExport)));
        let battery = sum(of(EnergyRole::Battery));
        let consumer_entities = of(EnergyRole::Consumer);

        let mut hint = None;
        let measured = sum(of(EnergyRole::House));
        let house: Option<Reading> = match measured {
            Some(m) => Some(m),
            None => {
                let inputs: Vec<Reading> = [solar, grid, battery].into_iter().flatten().collect();
                if !inputs.is_empty() {
                    let total = if inputs.iter().any(|r| r.watts.is_none()) {
                        None
                    } else {
                        Some(inputs.iter().filter_map(|r| r.watts).sum::<f64>())
                    };
                    if total.is_some_and(|t| t < 0.0) {
                        hint = Some(EnergyHint::CheckSign);
                    }
                    // An unavailable input (an inverter offline at night) leaves the house
                    // unknown, not unavailable: the house itself still has power.
                    Some(Reading {
                        watts: total.map(|t| t.max(0.0)),
                        status: worst(inputs.iter().map(|r| match r.status {
                            NodeStatus::Unavailable => NodeStatus::Live,
                            s => s,
                        })),
                    })
                } else if !consumer_entities.is_empty() {
                    Some(Reading {
                        watts: None,
                        status: NodeStatus::Live,
                    })
                } else {
                    None
                }
            }
        };

        let ids = |roles: &[EnergyRole]| -> Vec<String> {
            let mut v: Vec<&EnergyEntity> = roles.iter().flat_map(|r| of(*r)).copied().collect();
            v.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
            v.into_iter().map(|e| e.id.clone()).collect()
        };

        let with_total = |mut node: EnergyNode, sensors: &[&EnergyEntity]| -> EnergyNode {
            let t = total(sensors, entities);
            node.total_kwh = t.kwh;
            node.total_status = t.status;
            node.has_meter = t.has_meter;
            node
        };

        let mut consumer_nodes: HashMap<&str, EnergyNode> = HashMap::new();
        for e in consumer_entities {
            let r = sum(&[e]).expect("one entity gives a reading");
            let w = r.watts.map(|w| w.max(0.0));
            let mut node =
                EnergyNode::new(format!("{CONSUMER_PREFIX}{}", e.id), NodeKind::Consumer);
            node.name = Some(e.name.clone());
            node.value = w;
            node.flow = outward(w);
            node.status = r.status;
            node.sensor_ids = vec![e.id.clone()];
            node.icon = e.icon;
            consumer_nodes.insert(&e.id, with_total(node, &[e]));
        }
        let fresh = rank(consumer_entities, &consumer_nodes);
        let order = match previous {
            Some(p)
                if now_ms.saturating_sub(p.at_ms) < REORDER_MS
                    && p.ids.iter().collect::<BTreeSet<_>>()
                        == fresh.iter().collect::<BTreeSet<_>>() =>
            {
                p.clone()
            }
            _ => ConsumerOrder {
                ids: fresh,
                at_ms: now_ms,
            },
        };
        let consumers: Vec<EnergyNode> = order
            .ids
            .iter()
            .map(|id| consumer_nodes[id.as_str()].clone())
            .collect();
        let arc: Vec<EnergyNode> = if consumers.len() <= ARC_SLOTS {
            consumers.clone()
        } else {
            let mut a = consumers[..ARC_SLOTS - 1].to_vec();
            a.push(more(&consumers[ARC_SLOTS - 1..]));
            a
        };

        let solar_node = solar.map(|r| {
            let mut n = EnergyNode::new("solar", NodeKind::Solar);
            n.value = r.watts;
            n.flow = inward(r.watts);
            n.status = r.status;
            n.sensor_ids = ids(&[EnergyRole::Solar]);
            with_total(n, of(EnergyRole::Solar))
        });
        let grid_node = grid.map(|r| {
            let mut n = EnergyNode::new("grid", NodeKind::Grid);
            n.value = r.watts.map(f64::abs);
            n.flow = signed(r.watts);
            n.status = r.status;
            n.sensor_ids = ids(&[EnergyRole::Grid, EnergyRole::GridExport]);
            with_total(n, of(EnergyRole::Grid))
        });
        let battery_node = battery.map(|r| {
            let mut n = EnergyNode::new("battery", NodeKind::Battery);
            n.value = r.watts.map(f64::abs);
            n.flow = signed(r.watts);
            n.status = r.status;
            n.sensor_ids = ids(&[EnergyRole::Battery]);
            with_total(n, of(EnergyRole::Battery))
        });
        let house_node = house.map(|r| {
            let mut n = EnergyNode::new("house", NodeKind::House);
            n.value = r.watts;
            n.status = r.status;
            n.sensor_ids = ids(&[EnergyRole::House]);
            if measured.is_some() {
                with_total(n, of(EnergyRole::House))
            } else {
                let t = derived_total(&by_role, entities);
                n.total_kwh = t.kwh;
                n.total_status = t.status;
                n.has_meter = t.has_meter;
                n
            }
        });
        let water_node = of(EnergyRole::Water).first().map(|e| water(e, entities));

        let roles: Vec<EnergyNode> = [solar_node, grid_node, battery_node.clone(), house_node]
            .into_iter()
            .flatten()
            .collect();
        let self_consumption = match solar.and_then(|r| r.watts) {
            Some(s) if s >= SELF_CONSUMPTION_MIN_W && grid.is_none_or(|g| g.watts.is_some()) => {
                let export = (-grid.and_then(|g| g.watts).unwrap_or(0.0)).max(0.0);
                Some(((s - export) * 100.0 / s).trunc().clamp(0.0, 100.0) as u8)
            }
            _ => None,
        };

        let mut diagram = roles.clone();
        diagram.extend(arc);
        if battery_node.is_none() {
            diagram.extend(water_node.clone());
        }
        let mut cards = roles;
        cards.extend(consumers.iter().cloned());
        cards.extend(water_node);

        EnergyUi {
            loading: false,
            empty: false,
            diagram,
            cards,
            consumers,
            self_consumption,
            hint,
            order: Some(order),
        }
    }
}

/// A node's lifetime energy: the paired meters of its power `sensors` (each counted once),
/// summed in kWh.
fn total(sensors: &[&EnergyEntity], all: &[EnergyEntity]) -> Total {
    let meters = EnergyMeters::node_meters(sensors, all);
    if meters.is_empty() {
        return NO_TOTAL;
    }
    let parts: Vec<Reading> = meters.iter().map(|m| kwh(m)).collect();
    let sum = if parts.iter().any(|p| p.watts.is_none()) {
        None
    } else {
        Some(parts.iter().filter_map(|p| p.watts).sum())
    };
    Total {
        has_meter: true,
        kwh: sum,
        status: worst(parts.iter().map(|p| p.status)),
    }
}

/// A house without its own sensor: sum of solar + grid import - grid export - inverter draw.
fn derived_total(by_role: &ByRole, all: &[EnergyEntity]) -> Total {
    let Some(meters) = EnergyMeters::house_meters(by_role, all) else {
        return NO_TOTAL;
    };
    let plus: Vec<Reading> = meters.plus.iter().map(|m| kwh(m)).collect();
    let minus: Vec<Reading> = meters.minus.iter().map(|m| kwh(m)).collect();
    let unknown = plus.iter().chain(&minus).any(|p| p.watts.is_none());
    let sum_of = |v: &[Reading]| v.iter().filter_map(|p| p.watts).sum::<f64>();
    Total {
        has_meter: true,
        kwh: if unknown {
            None
        } else {
            Some((sum_of(&plus) - sum_of(&minus)).max(0.0))
        },
        status: worst(plus.iter().chain(&minus).map(|p| p.status)),
    }
}

/// A counter's reading in kWh, carried in `Reading::watts`.
fn kwh(meter: &EnergyEntity) -> Reading {
    let status = status(meter);
    let factor = EnergyUnits::factor(meter.unit.as_deref());
    let value = match (status, meter.value, factor) {
        (NodeStatus::Live, Some(v), Some(f)) => Some(v * f),
        _ => None,
    };
    Reading {
        watts: value,
        status,
    }
}

/// Summed watts of `entities` (`None` if any is unknown), or `None` when the role has no entity.
fn sum(entities: &[&EnergyEntity]) -> Option<Reading> {
    if entities.is_empty() {
        return None;
    }
    let parts: Vec<Reading> = entities.iter().map(|e| reading(e)).collect();
    let watts = if parts.iter().any(|p| p.watts.is_none()) {
        None
    } else {
        Some(parts.iter().filter_map(|p| p.watts).sum())
    };
    Some(Reading {
        watts,
        status: worst(parts.iter().map(|p| p.status)),
    })
}

fn reading(e: &EnergyEntity) -> Reading {
    let status = status(e);
    let factor = PowerUnits::factor(e.unit.as_deref());
    let sign = if e.invert && e.role.is_some_and(EnergyRole::signed) {
        -1.0
    } else {
        1.0
    };
    let watts = match (status, e.value, factor) {
        (NodeStatus::Live, Some(v), Some(f)) => Some(v * f * sign),
        _ => None,
    };
    Reading { watts, status }
}

fn status(e: &EnergyEntity) -> NodeStatus {
    if e.available {
        NodeStatus::Live
    } else {
        NodeStatus::Unavailable
    }
}

fn worst(statuses: impl IntoIterator<Item = NodeStatus>) -> NodeStatus {
    statuses.into_iter().max().unwrap_or(NodeStatus::Live)
}

/// G = sum of grid - sum of grid export.
fn grid(import: Option<Reading>, export: Option<Reading>) -> Option<Reading> {
    match (import, export) {
        (None, None) => None,
        (Some(i), None) => Some(i),
        (None, Some(x)) => Some(Reading {
            watts: x.watts.map(|w| -w),
            status: x.status,
        }),
        (Some(i), Some(x)) => Some(Reading {
            watts: match (i.watts, x.watts) {
                (Some(a), Some(b)) => Some(a - b),
                _ => None,
            },
            status: worst([i.status, x.status]),
        }),
    }
}

fn inward(w: Option<f64>) -> Flow {
    if w.is_some_and(|w| w >= IDLE_W) {
        Flow::In
    } else {
        Flow::Idle
    }
}

fn outward(w: Option<f64>) -> Flow {
    if w.is_some_and(|w| w >= IDLE_W) {
        Flow::Out
    } else {
        Flow::Idle
    }
}

/// + flows into the house (import, discharge), - flows out (export, charge).
fn signed(w: Option<f64>) -> Flow {
    match w {
        Some(w) if w >= IDLE_W => Flow::In,
        Some(w) if w <= -IDLE_W => Flow::Out,
        _ => Flow::Idle,
    }
}

/// `order` override first, then power high to low (unknown last), then name.
fn rank(entities: &[&EnergyEntity], nodes: &HashMap<&str, EnergyNode>) -> Vec<String> {
    let power = |e: &EnergyEntity| nodes[e.id.as_str()].value.unwrap_or(-1.0);
    let mut v: Vec<&EnergyEntity> = entities.to_vec();
    v.sort_by(|a, b| {
        a.order
            .unwrap_or(i32::MAX)
            .cmp(&b.order.unwrap_or(i32::MAX))
            .then_with(|| (-power(a)).total_cmp(&(-power(b))))
            .then_with(|| a.name.cmp(&b.name))
    });
    v.into_iter().map(|e| e.id.clone()).collect()
}

fn more(rest: &[EnergyNode]) -> EnergyNode {
    let total = if rest.iter().any(|n| n.value.is_none()) {
        None
    } else {
        Some(rest.iter().filter_map(|n| n.value).sum::<f64>())
    };
    let mut n = EnergyNode::new("more", NodeKind::More);
    n.value = total;
    n.flow = outward(total);
    n.status = worst(rest.iter().map(|r| r.status));
    n.count = rest.len();
    n.sensor_ids = rest.iter().flat_map(|r| r.sensor_ids.clone()).collect();
    n
}

fn water(e: &EnergyEntity, all: &[EnergyEntity]) -> EnergyNode {
    let status = status(e);
    let mut node = EnergyNode::new("water", NodeKind::Water);
    node.value = e.value.filter(|_| status != NodeStatus::Unavailable);
    node.unit = e.unit.clone();
    node.status = status;
    node.sensor_ids = vec![e.id.clone()];
    if let Some(meter) = EnergyMeters::water_meter_for(e, all) {
        node.water_total = meter.value.filter(|_| meter.available);
        node.water_total_unit = meter.unit.clone();
    }
    node
}

#[cfg(test)]
mod tests {
    use super::super::testkit::e;
    use super::*;

    fn p(id: &str, role: EnergyRole, v: f64) -> EnergyEntity {
        EnergyEntity {
            role: Some(role),
            value: Some(v),
            ..e(id)
        }
    }

    fn consumer(id: &str, name: &str, v: f64) -> EnergyEntity {
        EnergyEntity {
            name: name.into(),
            ..p(id, EnergyRole::Consumer, v)
        }
    }

    fn build(all: &[EnergyEntity]) -> EnergyUi {
        EnergyFlow::build(all, 0, None)
    }

    /// Signed grid, solar, two consumers, derived house.
    fn house() -> Vec<EnergyEntity> {
        vec![
            p("grid", EnergyRole::Grid, -400.0),
            p("solar", EnergyRole::Solar, 3200.0),
            consumer("ac", "AC", 600.0),
            consumer("heater", "Water heater", 1700.0),
        ]
    }

    fn set(all: &mut [EnergyEntity], id: &str, v: f64) {
        all.iter_mut().find(|x| x.id == id).unwrap().value = Some(v);
    }

    fn kinds_ids(ui: &EnergyUi) -> Vec<&str> {
        ui.diagram.iter().map(|n| n.id.as_str()).collect()
    }

    #[test]
    fn empty_and_loading() {
        let ui = build(&[]);
        assert!(ui.loading && !ui.empty);
        let ui = build(&[e("t")]);
        assert!(!ui.loading && ui.empty);
        assert!(ui.diagram.is_empty());
    }

    #[test]
    fn derived_house_sums_inputs() {
        let ui = build(&house());
        assert_eq!(ui.node("house").unwrap().value, Some(2800.0));
        assert_eq!(ui.node("grid").unwrap().value, Some(400.0));
        assert_eq!(ui.node("grid").unwrap().flow, Flow::Out);
        assert_eq!(ui.node("solar").unwrap().flow, Flow::In);
        assert_eq!(ui.self_consumption, Some(87));
        assert_eq!(ui.hint, None);
        assert_eq!(
            kinds_ids(&ui),
            ["solar", "grid", "house", "consumer:heater", "consumer:ac"]
        );
    }

    #[test]
    fn measured_house_wins() {
        let mut all = house();
        all.push(p("load", EnergyRole::House, 2500.0));
        let ui = build(&all);
        assert_eq!(ui.node("house").unwrap().value, Some(2500.0));
        assert_eq!(ui.node("house").unwrap().sensor_ids, ["load"]);
        assert!(build(&house()).node("house").unwrap().sensor_ids.is_empty());
    }

    #[test]
    fn inverts_signed_roles_only() {
        let grid = EnergyEntity {
            invert: true,
            ..p("grid", EnergyRole::Grid, 400.0)
        };
        assert_eq!(build(&[grid]).node("grid").unwrap().flow, Flow::Out);
        let solar = EnergyEntity {
            invert: true,
            ..p("solar", EnergyRole::Solar, 1000.0)
        };
        assert_eq!(build(&[solar]).node("solar").unwrap().value, Some(1000.0));
    }

    #[test]
    fn grid_minus_export() {
        let ui = build(&[
            p("imp", EnergyRole::Grid, 0.0),
            p("exp", EnergyRole::GridExport, 700.0),
            p("solar", EnergyRole::Solar, 2000.0),
        ]);
        assert_eq!(ui.node("grid").unwrap().value, Some(700.0));
        assert_eq!(ui.node("grid").unwrap().flow, Flow::Out);
        assert_eq!(ui.node("house").unwrap().value, Some(1300.0));
        assert_eq!(ui.node("grid").unwrap().sensor_ids, ["exp", "imp"]);
    }

    #[test]
    fn export_alone_is_a_negative_grid() {
        let ui = build(&[p("exp", EnergyRole::GridExport, 700.0)]);
        assert_eq!(ui.node("grid").unwrap().flow, Flow::Out);
    }

    #[test]
    fn battery_directions() {
        let grid = p("grid", EnergyRole::Grid, 100.0);
        let ui = build(&[p("bat", EnergyRole::Battery, 300.0), grid.clone()]);
        assert_eq!(ui.node("battery").unwrap().flow, Flow::In);
        assert_eq!(ui.node("house").unwrap().value, Some(400.0));
        let ui = build(&[p("bat", EnergyRole::Battery, -300.0), grid.clone()]);
        assert_eq!(ui.node("battery").unwrap().flow, Flow::Out);
        assert_eq!(ui.node("battery").unwrap().value, Some(300.0));
        let ui = build(&[p("bat", EnergyRole::Battery, 5.0), grid]);
        assert_eq!(ui.node("battery").unwrap().flow, Flow::Idle);
    }

    #[test]
    fn units_are_converted_to_watts() {
        let solar = EnergyEntity {
            unit: Some("kW".into()),
            ..p("solar", EnergyRole::Solar, 3.2)
        };
        assert_eq!(build(&[solar]).node("solar").unwrap().value, Some(3200.0));
    }

    #[test]
    fn bad_unit_is_unknown() {
        let grid = EnergyEntity {
            unit: Some("kWh".into()),
            ..p("grid", EnergyRole::Grid, 6584.0)
        };
        let ui = build(&[grid]);
        let n = ui.node("grid").unwrap();
        assert_eq!(n.value, None);
        assert_eq!(n.flow, Flow::Idle);
        let solar = EnergyEntity {
            unit: Some("kWh".into()),
            ..p("solar", EnergyRole::Solar, 3.2)
        };
        let ui = build(&[solar, p("grid", EnergyRole::Grid, 100.0)]);
        assert_eq!(ui.node("solar").unwrap().value, None);
        assert_eq!(ui.node("house").unwrap().value, None);
    }

    #[test]
    fn summed_roles_add_up() {
        let ui = build(&[
            p("s1", EnergyRole::Solar, 1000.0),
            p("s2", EnergyRole::Solar, 500.0),
        ]);
        assert_eq!(ui.node("solar").unwrap().value, Some(1500.0));
        assert_eq!(
            ui.diagram
                .iter()
                .filter(|n| n.kind == NodeKind::Solar)
                .count(),
            1
        );
    }

    #[test]
    fn negative_house_is_zero_with_hint() {
        let ui = build(&[
            p("grid", EnergyRole::Grid, -3000.0),
            p("solar", EnergyRole::Solar, 1000.0),
        ]);
        assert_eq!(ui.node("house").unwrap().value, Some(0.0));
        assert_eq!(ui.hint, Some(EnergyHint::CheckSign));
    }

    #[test]
    fn unknown_input_makes_house_unknown() {
        let mut all = house();
        all[1].value = None;
        let ui = build(&all);
        assert_eq!(ui.node("house").unwrap().value, None);
        assert_eq!(ui.hint, None);
        assert_eq!(ui.self_consumption, None);
        assert!(
            ui.diagram
                .iter()
                .filter(|n| n.kind == NodeKind::Solar)
                .all(|n| n.flow == Flow::Idle)
        );
    }

    #[test]
    fn unavailable_input_leaves_house_unknown_not_unavailable() {
        let mut all = house();
        set(&mut all, "grid", 900.0);
        set(&mut all, "solar", 0.0);
        all[1].available = false;
        let ui = build(&all);
        assert_eq!(ui.node("solar").unwrap().status, NodeStatus::Unavailable);
        assert_eq!(ui.node("house").unwrap().value, None);
        assert_eq!(ui.node("house").unwrap().status, NodeStatus::Live);
    }

    #[test]
    fn unavailable_consumer_is_unknown_and_flagged() {
        let mut all = house();
        all[2].available = false;
        let ui = build(&all);
        let n = ui.node("consumer:ac").unwrap();
        assert_eq!((n.value, n.status), (None, NodeStatus::Unavailable));
    }

    #[test]
    fn self_consumption_hidden_under_50w() {
        let mut all = house();
        set(&mut all, "grid", 300.0);
        set(&mut all, "solar", 49.0);
        assert_eq!(build(&all).self_consumption, None);
        set(&mut all, "solar", 50.0);
        assert_eq!(build(&all).self_consumption, Some(100));
        set(&mut all, "solar", 0.0);
        assert_eq!(build(&all).self_consumption, None);
    }

    #[test]
    fn edges_below_10w_are_idle_and_negative_consumers_clamp() {
        let mut all = house();
        set(&mut all, "grid", 5.0);
        set(&mut all, "ac", 9.0);
        let ui = build(&all);
        assert_eq!(ui.node("grid").unwrap().flow, Flow::Idle);
        assert_eq!(ui.node("consumer:ac").unwrap().flow, Flow::Idle);
        assert_eq!(ui.node("consumer:heater").unwrap().flow, Flow::Out);
        set(&mut all, "ac", -3.0);
        let n = build(&all);
        assert_eq!(n.node("consumer:ac").unwrap().value, Some(0.0));
    }

    #[test]
    fn folds_four_consumers_into_more() {
        let mut all = house();
        all.push(consumer("ev", "EV", 3000.0));
        all.push(consumer("pump", "Pump", 50.0));
        let ui = build(&all);
        let arc: Vec<&str> = ui
            .diagram
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Consumer | NodeKind::More))
            .map(|n| n.id.as_str())
            .collect();
        assert_eq!(arc, ["consumer:ev", "consumer:heater", "more"]);
        let more = ui.node("more");
        // `more` is a diagram node only; `node` finds it there.
        let more = more.unwrap();
        assert_eq!(more.count, 2);
        assert_eq!(more.value, Some(650.0));
        assert_eq!(more.sensor_ids, ["ac", "pump"]);
        let order: Vec<&str> = ui.consumers.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(
            order,
            [
                "consumer:ev",
                "consumer:heater",
                "consumer:ac",
                "consumer:pump"
            ]
        );
        assert_eq!(
            ui.cards
                .iter()
                .filter(|n| n.kind == NodeKind::Consumer)
                .count(),
            4
        );
    }

    #[test]
    fn override_order_comes_before_power_order() {
        let ac = EnergyEntity {
            order: Some(1),
            ..consumer("ac", "AC", 10.0)
        };
        let ui = build(&[ac, consumer("heater", "Heater", 2000.0)]);
        let ids: Vec<&str> = ui.consumers.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["consumer:ac", "consumer:heater"]);
    }

    #[test]
    fn ties_and_unknowns_sort_by_name_with_unknown_last() {
        let mut unknown = consumer("a", "A", 0.0);
        unknown.value = None;
        let ui = build(&[
            unknown,
            consumer("c", "C", 100.0),
            consumer("b", "B", 100.0),
        ]);
        let ids: Vec<&str> = ui.consumers.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["consumer:b", "consumer:c", "consumer:a"]);
    }

    #[test]
    fn order_is_throttled() {
        let mut all = house();
        let first = EnergyFlow::build(&all, 0, None);
        let ids =
            |ui: &EnergyUi| -> Vec<String> { ui.consumers.iter().map(|n| n.id.clone()).collect() };
        assert_eq!(ids(&first), ["consumer:heater", "consumer:ac"]);
        set(&mut all, "ac", 2500.0);
        let kept = EnergyFlow::build(&all, 2_000, first.order.as_ref());
        assert_eq!(ids(&kept), ["consumer:heater", "consumer:ac"]);
        assert_eq!(kept.order.as_ref().unwrap().at_ms, 0);
        let kept = EnergyFlow::build(&all, 4_999, first.order.as_ref());
        assert_eq!(ids(&kept), ["consumer:heater", "consumer:ac"]);
        let fresh = EnergyFlow::build(&all, 6_000, first.order.as_ref());
        assert_eq!(ids(&fresh), ["consumer:ac", "consumer:heater"]);
        assert_eq!(fresh.order.as_ref().unwrap().at_ms, 6_000);
        let at_limit = EnergyFlow::build(&all, 5_000, first.order.as_ref());
        assert_eq!(ids(&at_limit), ["consumer:ac", "consumer:heater"]);
    }

    #[test]
    fn order_is_recomputed_when_the_set_changes() {
        let all = house();
        let first = EnergyFlow::build(&all, 0, None);
        let fewer: Vec<EnergyEntity> = all.into_iter().filter(|x| x.id != "heater").collect();
        let ui = EnergyFlow::build(&fewer, 1_000, first.order.as_ref());
        assert_eq!(ui.consumers.len(), 1);
        assert_eq!(ui.order.unwrap().at_ms, 1_000);
    }

    #[test]
    fn consumer_icon_and_name_reach_the_node() {
        let oven = EnergyEntity {
            icon: Some(ConsumerIcon::Oven),
            ..consumer("oven", "Oven", 900.0)
        };
        let ui = build(&[oven]);
        let n = ui.node("consumer:oven").unwrap();
        assert_eq!(n.icon, Some(ConsumerIcon::Oven));
        assert_eq!(n.name.as_deref(), Some("Oven"));
    }

    #[test]
    fn consumers_alone_still_show_an_unknown_house() {
        let ui = build(&[consumer("ac", "AC", 300.0)]);
        assert_eq!(ui.node("house").unwrap().value, None);
        assert!(ui.diagram.iter().any(|n| n.id == "consumer:ac"));
    }

    #[test]
    fn a_consumer_named_like_a_role_does_not_collide() {
        let ui = build(&[
            p("grid", EnergyRole::Grid, 500.0),
            consumer("house", "House plug", 200.0),
        ]);
        let ids: BTreeSet<&str> = ui.cards.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids.len(), ui.cards.len());
        assert_eq!(ui.node("consumer:house").unwrap().value, Some(200.0));
    }

    fn water_entity(id: &str, unit: &str, v: f64, device: Option<&str>) -> EnergyEntity {
        EnergyEntity {
            unit: Some(unit.into()),
            device_id: device.map(String::from),
            ..p(id, EnergyRole::Water, v)
        }
    }

    #[test]
    fn water_takes_bottom_left_only_without_battery() {
        let mut all = house();
        all.push(water_entity("water", "L", 312.0, None));
        let ui = build(&all);
        let w = ui.node("water").unwrap();
        assert_eq!((w.value, w.unit.as_deref()), (Some(312.0), Some("L")));
        assert!(ui.diagram.iter().any(|n| n.id == "water"));
        all.push(p("bat", EnergyRole::Battery, 0.0));
        let ui = build(&all);
        assert!(ui.diagram.iter().all(|n| n.id != "water"));
        assert!(ui.cards.iter().any(|n| n.id == "water"));
    }

    #[test]
    fn a_water_card_carries_the_m3_meter_of_its_device() {
        let all = vec![
            water_entity("daily", "L", 172.0, Some("suez")),
            EnergyEntity {
                device_id: Some("suez".into()),
                unit: Some("m³".into()),
                value: Some(601.359),
                ..e("index")
            },
        ];
        let w = build(&all).node("water").cloned().unwrap();
        assert_eq!(w.value, Some(172.0));
        assert_eq!(w.water_total, Some(601.359));
        assert_eq!(w.water_total_unit.as_deref(), Some("m³"));
    }

    #[test]
    fn a_water_card_on_the_meter_itself_or_alone_has_no_total() {
        let all = vec![
            water_entity("index", "m³", 601.0, Some("suez")),
            EnergyEntity {
                device_id: Some("suez".into()),
                unit: Some("m3".into()),
                value: Some(1.0),
                ..e("other")
            },
        ];
        assert_eq!(build(&all).node("water").unwrap().water_total, None);
        let alone = [water_entity("daily", "L", 172.0, Some("suez"))];
        assert_eq!(build(&alone).node("water").unwrap().water_total_unit, None);
    }

    #[test]
    fn an_unavailable_water_meter_has_no_total_but_keeps_its_unit() {
        let all = vec![
            water_entity("daily", "L", 172.0, Some("suez")),
            EnergyEntity {
                device_id: Some("suez".into()),
                unit: Some("m³".into()),
                value: Some(601.0),
                available: false,
                ..e("index")
            },
        ];
        let w = build(&all).node("water").cloned().unwrap();
        assert_eq!(w.water_total, None);
        assert_eq!(w.water_total_unit.as_deref(), Some("m³"));
    }

    /// The live house: grid on `power_a`, solar on `power_b` of one clamp, with its counters.
    fn clamp(extra: impl Fn(&str) -> Option<f64>) -> Vec<EnergyEntity> {
        let dev = "zigbee2mqtt_0xa4c13898428fae3a";
        let mk = |kind: &str, unit: &str, v: f64, role: Option<EnergyRole>| EnergyEntity {
            device_id: Some(dev.into()),
            label: Some(kind.into()),
            unit: Some(unit.into()),
            value: extra(kind).or(Some(v)),
            role,
            ..e(&format!("0xa4c13898428fae3a_{kind}_zigbee2mqtt"))
        };
        vec![
            mk("power_a", "W", 300.0, Some(EnergyRole::Grid)),
            mk("power_b", "W", 500.0, Some(EnergyRole::Solar)),
            mk("energy_a", "kWh", 100.0, None),
            mk("energy_b", "kWh", 40.0, None),
            mk("energy_produced_a", "kWh", 30.0, None),
            mk("energy_produced_b", "kWh", 200.0, None),
        ]
    }

    #[test]
    fn lifetime_total_from_paired_meters() {
        let ui = build(&clamp(|_| None));
        let grid = ui.node("grid").unwrap();
        assert_eq!((grid.total_kwh, grid.has_meter), (Some(100.0), true));
        assert_eq!(ui.node("solar").unwrap().total_kwh, Some(200.0));
        let house = ui.node("house").unwrap();
        assert!(house.has_meter);
        assert_eq!(house.total_kwh, Some(100.0 - 30.0 + 200.0 - 40.0));
        assert_eq!(house.total_status, NodeStatus::Live);
    }

    #[test]
    fn derived_house_total_is_clamped_at_zero() {
        let ui = build(&clamp(|k| (k == "energy_produced_a").then_some(900.0)));
        assert_eq!(ui.node("house").unwrap().total_kwh, Some(0.0));
    }

    #[test]
    fn a_house_without_an_export_counter_has_no_total() {
        let all: Vec<EnergyEntity> = clamp(|_| None)
            .into_iter()
            .filter(|x| !x.id.contains("produced_a"))
            .collect();
        assert!(!build(&all).node("house").unwrap().has_meter);
    }

    #[test]
    fn a_derived_house_follows_its_inputs_status() {
        let mut all = clamp(|_| None);
        all.iter_mut()
            .find(|x| x.id.contains("energy_produced_a"))
            .unwrap()
            .available = false;
        let house = build(&all).node("house").cloned().unwrap();
        assert_eq!(house.total_kwh, None);
        assert_eq!(house.total_status, NodeStatus::Unavailable);
    }

    #[test]
    fn a_meter_in_wh_is_converted_and_an_unavailable_one_gives_no_total() {
        let dev = Some("d".to_string());
        let heater = EnergyEntity {
            device_id: dev.clone(),
            ..consumer("heater", "Heater", 100.0)
        };
        let meter = |unit: &str, v: f64, available: bool| EnergyEntity {
            device_id: dev.clone(),
            unit: Some(unit.into()),
            value: Some(v),
            available,
            ..e("m")
        };
        let ui = build(&[heater.clone(), meter("Wh", 2500.0, true)]);
        assert_eq!(ui.node("consumer:heater").unwrap().total_kwh, Some(2.5));
        let ui = build(&[heater.clone(), meter("kWh", 1.0, false)]);
        let n = ui.node("consumer:heater").unwrap();
        assert!(n.has_meter);
        assert_eq!(n.total_kwh, None);
        assert_eq!(n.total_status, NodeStatus::Unavailable);
        let mut unknown = meter("kWh", 1.0, true);
        unknown.value = None;
        let ui = build(&[heater, unknown]);
        assert_eq!(ui.node("consumer:heater").unwrap().total_kwh, None);
    }

    #[test]
    fn grid_totals_ignore_the_export_counter() {
        let mk = |id: &str, role: Option<EnergyRole>, unit: &str, v: f64, dev: &str| EnergyEntity {
            device_id: Some(dev.into()),
            unit: Some(unit.into()),
            value: Some(v),
            role,
            ..e(id)
        };
        let all = vec![
            mk("grid", Some(EnergyRole::Grid), "W", 100.0, "g"),
            mk("export", Some(EnergyRole::GridExport), "W", 50.0, "x"),
            mk("grid_kwh", None, "kWh", 10.0, "g"),
            mk("export_kwh", None, "kWh", 99.0, "x"),
        ];
        assert_eq!(build(&all).node("grid").unwrap().total_kwh, Some(10.0));
        let export_only = vec![all[1].clone(), all[3].clone()];
        assert!(!build(&export_only).node("grid").unwrap().has_meter);
    }

    #[test]
    fn solar_sums_its_inverters_meters() {
        let mk = |id: &str, role: Option<EnergyRole>, unit: &str, v: f64, dev: &str| EnergyEntity {
            device_id: Some(dev.into()),
            unit: Some(unit.into()),
            value: Some(v),
            role,
            ..e(id)
        };
        let all = vec![
            mk("s1", Some(EnergyRole::Solar), "W", 1.0, "a"),
            mk("s2", Some(EnergyRole::Solar), "W", 1.0, "b"),
            mk("k1", None, "kWh", 10.0, "a"),
            mk("k2", None, "kWh", 5.0, "b"),
        ];
        assert_eq!(build(&all).node("solar").unwrap().total_kwh, Some(15.0));
    }

    #[test]
    fn a_derived_house_has_no_meter_without_solar_or_grid_counters() {
        let mut all = house();
        all.push(EnergyEntity {
            device_id: Some("other".into()),
            unit: Some("kWh".into()),
            ..e("x")
        });
        assert!(!build(&all).node("house").unwrap().has_meter);
    }
}
