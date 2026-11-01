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
mod tests {
    use super::*;
    use nidoo_core::energy::NodeKind;

    fn node(id: &str, kind: NodeKind, watts: f64, flow: Flow) -> EnergyNode {
        EnergyNode {
            id: id.into(),
            kind,
            name: None,
            value: Some(watts),
            unit: None,
            flow,
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

    fn consumer(name: &str, watts: f64) -> EnergyNode {
        EnergyNode {
            name: Some(name.into()),
            ..node(
                &format!("consumer:{}", name.to_lowercase().replace(' ', "-")),
                NodeKind::Consumer,
                watts,
                Flow::In,
            )
        }
    }

    fn ui(nodes: &[EnergyNode], consumers: &[EnergyNode]) -> EnergyUi {
        EnergyUi {
            diagram: nodes.to_vec(),
            cards: consumers.to_vec(),
            consumers: consumers.to_vec(),
            ..EnergyUi::default()
        }
    }

    fn solar(w: f64) -> EnergyNode {
        node("solar", NodeKind::Solar, w, Flow::In)
    }
    fn grid(w: f64, flow: Flow) -> EnergyNode {
        node("grid", NodeKind::Grid, w, flow)
    }
    fn battery(w: f64, flow: Flow) -> EnergyNode {
        node("battery", NodeKind::Battery, w, flow)
    }
    fn house(w: f64) -> EnergyNode {
        node("house", NodeKind::House, w, Flow::In)
    }

    fn link_w(s: &Sankey, from: &str, to: &str) -> Option<f64> {
        s.links
            .iter()
            .find(|l| l.from == from && l.to == to)
            .map(|l| l.watts)
    }
    fn find<'a>(s: &'a Sankey, id: &str) -> Option<&'a SankeyNode> {
        s.nodes.iter().find(|n| n.id == id)
    }

    #[test]
    fn import_only() {
        let oven = consumer("Oven", 300.0);
        let lamp = consumer("Lamp", 5.0);
        let u = ui(
            &[grid(500.0, Flow::In), house(500.0)],
            &[oven.clone(), lamp],
        );
        let s = build(&u, FOLD_WIDE);
        assert_eq!(link_w(&s, "grid", "house"), Some(500.0));
        assert_eq!(link_w(&s, "house", &oven.id), Some(300.0));
        assert_eq!(link_w(&s, "house", "unitemised"), Some(200.0));
        assert!(s.nodes.iter().all(|n| n.id != "consumer:lamp"));
        assert_eq!(s.import_w, Some(500.0));
        assert_eq!(s.solar_share, None);
    }

    #[test]
    fn solar_surplus_exports() {
        let u = ui(
            &[solar(3000.0), grid(1200.0, Flow::Out), house(1800.0)],
            &[consumer("Heater", 900.0)],
        );
        let s = build(&u, FOLD_WIDE);
        assert_eq!(link_w(&s, "solar", "house"), Some(1800.0));
        assert_eq!(link_w(&s, "solar", "export"), Some(1200.0));
        let e = find(&s, "export").unwrap();
        assert_eq!((e.column, e.size), (1, 1200.0));
        assert_eq!(find(&s, "unitemised").unwrap().watts, 900.0);
        assert_eq!(s.solar_share, Some(100));
    }

    #[test]
    fn solar_share_does_not_lose_a_percent_to_rounding() {
        // 290 W of 1000 W: 290 / 1000 * 100 is 28.999…
        let u = ui(&[solar(290.0), grid(710.0, Flow::In), house(1000.0)], &[]);
        let s = build(&u, FOLD_WIDE);
        assert_eq!(link_w(&s, "solar", "house"), Some(290.0));
        assert_eq!(s.solar_share, Some(29));
    }

    #[test]
    fn charge_from_solar_then_grid() {
        let u = ui(
            &[
                solar(300.0),
                grid(400.0, Flow::In),
                battery(500.0, Flow::Out),
                house(200.0),
            ],
            &[],
        );
        let s = build(&u, FOLD_WIDE);
        assert_eq!(link_w(&s, "solar", "battery-charge"), Some(300.0));
        assert_eq!(link_w(&s, "grid", "battery-charge"), Some(200.0));
        assert_eq!(link_w(&s, "grid", "house"), Some(200.0));
        assert_eq!(s.solar_share, Some(0));
    }

    #[test]
    fn battery_feeds_export_without_solar() {
        let u = ui(
            &[
                battery(800.0, Flow::In),
                grid(300.0, Flow::Out),
                house(500.0),
            ],
            &[],
        );
        let s = build(&u, FOLD_WIDE);
        assert_eq!(link_w(&s, "battery", "export"), Some(300.0));
        assert_eq!(link_w(&s, "battery", "house"), Some(500.0));
    }

    #[test]
    fn night_with_solar_offline() {
        let mut sol = solar(0.0);
        sol.value = None;
        sol.status = NodeStatus::Unavailable;
        let u = ui(&[sol, grid(250.0, Flow::In), house(250.0)], &[]);
        let s = build(&u, FOLD_WIDE);
        assert!(find(&s, "solar").is_none());
        assert_eq!(link_w(&s, "grid", "house"), Some(250.0));
        assert_eq!(s.solar_w, None);
    }

    #[test]
    fn house_meter_lower_than_inflow() {
        let u = ui(
            &[grid(1000.0, Flow::In), house(900.0)],
            &[consumer("Oven", 950.0)],
        );
        let s = build(&u, FOLD_WIDE);
        let h = find(&s, "house").unwrap();
        assert_eq!((h.size, h.watts), (1000.0, 900.0));
        assert!(find(&s, "unitemised").is_none());
    }

    #[test]
    fn wide_folds_after_eight() {
        let cs: Vec<EnergyNode> = (0..9)
            .map(|i| consumer(&format!("Load {i}"), 500.0 - 50.0 * i as f64))
            .collect();
        let u = ui(&[grid(5000.0, Flow::In), house(5000.0)], &cs);
        let s = build(&u, FOLD_WIDE);
        let n = s
            .nodes
            .iter()
            .filter(|n| n.role == SankeyRole::Consumer)
            .count();
        assert_eq!(n, 7);
        assert_eq!(find(&s, "others").unwrap().count, 2);
        assert!(s.folded);
    }

    #[test]
    fn narrow_folds_at_four() {
        let cs: Vec<EnergyNode> = (0..12)
            .map(|i| consumer(&format!("Load {i}"), 100.0))
            .collect();
        let u = ui(&[grid(1500.0, Flow::In), house(1500.0)], &cs);
        let s = build(&u, FOLD_NARROW);
        let n = s
            .nodes
            .iter()
            .filter(|n| n.role == SankeyRole::Consumer)
            .count();
        assert_eq!(n, 3);
        let o = find(&s, "others").unwrap();
        assert_eq!((o.count, o.watts), (9, 900.0));
        assert_eq!(find(&s, "unitemised").unwrap().watts, 300.0);
    }

    #[test]
    fn every_drawn_node_balances() {
        let fixtures = vec![
            ui(
                &[grid(500.0, Flow::In), house(500.0)],
                &[consumer("Oven", 300.0)],
            ),
            ui(
                &[solar(3000.0), grid(1200.0, Flow::Out), house(1800.0)],
                &[consumer("Heater", 900.0)],
            ),
            ui(
                &[
                    solar(300.0),
                    grid(400.0, Flow::In),
                    battery(500.0, Flow::Out),
                    house(200.0),
                ],
                &[],
            ),
            ui(
                &[
                    battery(800.0, Flow::In),
                    grid(300.0, Flow::Out),
                    house(500.0),
                ],
                &[],
            ),
            ui(
                &[grid(1000.0, Flow::In), house(900.0)],
                &[consumer("Oven", 950.0)],
            ),
        ];
        for u in &fixtures {
            let s = build(u, FOLD_WIDE);
            for n in &s.nodes {
                let i: f64 = s
                    .links
                    .iter()
                    .filter(|l| l.to == n.id)
                    .map(|l| l.watts)
                    .sum();
                let o: f64 = s
                    .links
                    .iter()
                    .filter(|l| l.from == n.id)
                    .map(|l| l.watts)
                    .sum();
                match n.column {
                    1 => assert!(i <= n.size + 1e-9 && o <= n.size + 1e-9, "{}", n.id),
                    0 => assert!(o <= n.watts + 1e-9, "{}", n.id),
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn unknown_grid_has_no_import() {
        let s = build(&ui(&[solar(800.0), house(800.0)], &[]), FOLD_WIDE);
        assert_eq!(s.import_w, None);
        let s = build(
            &ui(
                &[solar(3000.0), grid(1200.0, Flow::Out), house(1800.0)],
                &[],
            ),
            FOLD_WIDE,
        );
        assert_eq!(s.import_w, Some(0.0));
    }

    #[test]
    fn nothing_known_is_empty() {
        let mut h = house(0.0);
        h.value = None;
        let s = build(&ui(&[h], &[]), FOLD_WIDE);
        assert!(s.is_empty());
        assert_eq!(s.house_w, None);
    }
}
