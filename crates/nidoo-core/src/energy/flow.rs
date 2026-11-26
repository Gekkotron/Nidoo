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
mod tests;
