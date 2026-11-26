//! The live energy flow as a balanced Sankey: nodes in three columns (sources, the house and its
//! sinks, consumers) and the links between them. Pure; `screens::energy_flux` draws it.

use nidoo_core::energy::{EnergyNode, EnergyUi, Flow, IDLE_W, NodeStatus};

/// Wide screens fold the consumers when more than this many are powered.
pub const FOLD_WIDE: usize = 8;
/// Narrow screens (< 600 px) fold when more than this many are powered.
pub const FOLD_NARROW: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SankeyRole {
    Solar,
    Grid,
    Battery,
    House,
    Export,
    BatteryCharge,
    Consumer,
    Others,
    Unitemised,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SankeyNode {
    /// "solar" "grid" "battery" "house" "export" "battery-charge" "others" "unitemised", or the
    /// consumer's `EnergyNode` id.
    pub id: String,
    pub role: SankeyRole,
    /// 0 sources, 1 house + sinks, 2 consumers.
    pub column: u8,
    /// A consumer's name; `None` for the others (UI words).
    pub name: Option<String>,
    /// What the label shows: the measured value (Others/Unitemised: their sum).
    pub watts: f64,
    /// What the bar is drawn for: max(in, out) of the drawn links, in watts.
    pub size: f64,
    /// The single sensor id to open, if exactly one.
    pub sensor: Option<String>,
    /// Others only: how many consumers it folds.
    pub count: usize,
    /// Consumer only: 0-based position among shown consumers (colour rotation).
    pub slot: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SankeyLink {
    pub from: String,
    pub to: String,
    pub watts: f64,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Sankey {
    /// Column 0 then 1 then 2, top to bottom within a column.
    pub nodes: Vec<SankeyNode>,
    /// Ordered by source node order, then target node order.
    pub links: Vec<SankeyLink>,
    pub house_w: Option<f64>,
    /// Measured production; `None` without a live solar node.
    pub solar_w: Option<f64>,
    /// Drawn from the grid; `None` without a live grid reading.
    pub import_w: Option<f64>,
    pub export_w: f64,
    /// 0..=100, rounded down.
    pub solar_share: Option<u8>,
    /// An Others node exists.
    pub folded: bool,
}

impl Sankey {
    /// No node is big enough to draw.
    pub fn is_empty(&self) -> bool {
        !self.nodes.iter().any(|n| n.size >= IDLE_W)
    }
}

/// A reading in watts for allocation: unknown, unavailable or idle counts as 0.
fn active(node: Option<&EnergyNode>) -> f64 {
    match node {
        Some(n) if n.status == NodeStatus::Live => n.value.filter(|v| *v >= IDLE_W).unwrap_or(0.0),
        _ => 0.0,
    }
}

/// The measured value for display, whatever its size; 0 when unknown or unavailable.
fn measured(node: Option<&EnergyNode>) -> f64 {
    match node {
        Some(n) if n.status == NodeStatus::Live => n.value.unwrap_or(0.0),
        _ => 0.0,
    }
}

fn sensor(node: Option<&EnergyNode>) -> Option<String> {
    node.filter(|n| n.sensor_ids.len() == 1)
        .map(|n| n.sensor_ids[0].clone())
}

/// Moves up to `want` out of `pool` and returns how much moved.
fn take(pool: &mut f64, want: f64) -> f64 {
    let t = pool.min(want).max(0.0);
    *pool -= t;
    t
}

pub fn build(ui: &EnergyUi, fold_at: usize) -> Sankey {
    let solar = ui.node("solar");
    let grid = ui.node("grid");
    let battery = ui.node("battery");
    let house = ui.node("house");

    let s_in = active(solar);
    let g_in = if grid.is_some_and(|n| n.flow == Flow::In) {
        active(grid)
    } else {
        0.0
    };
    let b_in = if battery.is_some_and(|n| n.flow == Flow::In) {
        active(battery)
    } else {
        0.0
    };
    let export = if grid.is_some_and(|n| n.flow == Flow::Out) {
        active(grid)
    } else {
        0.0
    };
    let charge = if battery.is_some_and(|n| n.flow == Flow::Out) {
        active(battery)
    } else {
        0.0
    };

    let (mut s, mut g, mut b) = (s_in, g_in, b_in);
    let solar_export = take(&mut s, export);
    let battery_export = take(&mut b, export - solar_export);
    let solar_charge = take(&mut s, charge);
    let grid_charge = take(&mut g, charge - solar_charge);
    let (solar_house, grid_house, battery_house) = (s, g, b);

    let house_measured = house
        .filter(|n| n.status == NodeStatus::Live)
        .and_then(|n| n.value);
    let any_input = [solar, grid, battery]
        .into_iter()
        .flatten()
        .any(|n| n.status == NodeStatus::Live && n.value.is_some());
    let house_w = match house_measured {
        Some(v) => Some(v),
        None if any_input => Some(solar_house + grid_house + battery_house),
        None => None,
    };
    let house_val = house_w.unwrap_or(0.0);

    let mut shown: Vec<&EnergyNode> = ui
        .consumers
        .iter()
        .filter(|c| c.status == NodeStatus::Live && c.value.is_some_and(|v| v >= IDLE_W))
        .collect();
    let mut others: Option<(f64, usize)> = None;
    if shown.len() > fold_at {
        let rest = shown.split_off(fold_at.saturating_sub(1));
        others = Some((
            rest.iter().map(|c| c.value.unwrap_or(0.0)).sum(),
            rest.len(),
        ));
    }
    let shown_sum: f64 = shown.iter().map(|c| c.value.unwrap_or(0.0)).sum();
    let others_w = others.map_or(0.0, |(w, _)| w);
    let unitemised = (house_val - shown_sum - others_w).max(0.0);

    // Links in node order: (from, to, watts), dropped under IDLE_W.
    let mut links: Vec<SankeyLink> = Vec::new();
    let mut link = |from: &str, to: &str, watts: f64| {
        if watts >= IDLE_W {
            links.push(SankeyLink {
                from: from.into(),
                to: to.into(),
                watts,
            });
        }
    };
    link("solar", "house", solar_house);
    link("solar", "battery-charge", solar_charge);
    link("solar", "export", solar_export);
    link("grid", "house", grid_house);
    link("grid", "battery-charge", grid_charge);
    link("battery", "house", battery_house);
    link("battery", "export", battery_export);
    for c in &shown {
        link("house", &c.id, c.value.unwrap_or(0.0));
    }
    link("house", "others", others_w);
    link("house", "unitemised", unitemised);

    let into = |id: &str| -> f64 { links.iter().filter(|l| l.to == id).map(|l| l.watts).sum() };
    let out_of = |id: &str| -> f64 { links.iter().filter(|l| l.from == id).map(|l| l.watts).sum() };

    let mut nodes: Vec<SankeyNode> = Vec::new();
    let mut push = |id: &str,
                    role: SankeyRole,
                    column: u8,
                    name: Option<String>,
                    watts: f64,
                    size: f64,
                    sensor: Option<String>,
                    count: usize,
                    slot: usize| {
        nodes.push(SankeyNode {
            id: id.into(),
            role,
            column,
            name,
            watts,
            size,
            sensor,
            count,
            slot,
        });
    };

    // Column 0: sources, sized by what they actually feed.
    for (id, role, node) in [
        ("solar", SankeyRole::Solar, solar),
        ("grid", SankeyRole::Grid, grid),
        ("battery", SankeyRole::Battery, battery),
    ] {
        let size = out_of(id);
        if size >= IDLE_W {
            push(id, role, 0, None, measured(node), size, sensor(node), 0, 0);
        }
    }

    // Column 1: the house, then the sinks that take from the sources directly.
    let inflow = into("house");
    let outflow = out_of("house");
    let house_size = inflow.max(house_val).max(outflow);
    if house_size >= IDLE_W {
        push(
            "house",
            SankeyRole::House,
            1,
            None,
            house_val,
            house_size,
            sensor(house),
            0,
            0,
        );
    }
    let charge_size = into("battery-charge");
    if charge_size >= IDLE_W {
        push(
            "battery-charge",
            SankeyRole::BatteryCharge,
            1,
            None,
            charge,
            charge_size,
            sensor(battery),
            0,
            0,
        );
    }
    let export_size = into("export");
    if export_size >= IDLE_W {
        push(
            "export",
            SankeyRole::Export,
            1,
            None,
            export,
            export_size,
            sensor(grid),
            0,
            0,
        );
    }

    // Column 2: consumers, the folded rest, what no consumer accounts for.
    for (slot, c) in shown.iter().enumerate() {
        let w = c.value.unwrap_or(0.0);
        push(
            &c.id,
            SankeyRole::Consumer,
            2,
            c.name.clone(),
            w,
            w,
            sensor(Some(c)),
            0,
            slot,
        );
    }
    if let Some((w, count)) = others {
        push("others", SankeyRole::Others, 2, None, w, w, None, count, 0);
    }
    if unitemised >= IDLE_W {
        push(
            "unitemised",
            SankeyRole::Unitemised,
            2,
            None,
            unitemised,
            unitemised,
            None,
            0,
            0,
        );
    }

    // Every link must join two drawn nodes; a sink without a drawn in-link is not drawn above,
    // and the house is drawn whenever it has a link, so nothing dangles.
    let solar_w = solar
        .filter(|n| n.status == NodeStatus::Live)
        .and_then(|n| n.value);
    let solar_share = match solar_w {
        Some(_) if house_val > 0.0 => {
            // Scale before dividing: 290 / 1000 * 100 is 28.999…, 290 * 100 / 1000 is 29.
            Some((solar_house * 100.0 / house_val).floor().clamp(0.0, 100.0) as u8)
        }
        _ => None,
    };
    let folded = others.is_some();

    Sankey {
        nodes,
        links,
        house_w,
        solar_w,
        import_w: grid
            .filter(|n| n.status == NodeStatus::Live && n.value.is_some())
            .map(|n| {
                if n.flow == Flow::In {
                    active(Some(n))
                } else {
                    0.0
                }
            }),
        export_w: export,
        solar_share,
        folded,
    }
}

#[cfg(test)]
mod tests;
