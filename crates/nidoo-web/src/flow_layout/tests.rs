use super::*;

#[test]
fn free_spot_keeps_a_free_start() {
    let taken = BTreeMap::from([("a".to_owned(), Pt { x: 500.0, y: 500.0 })]);
    assert_eq!(
        free_spot(
            &Flow::default(),
            &taken,
            Pt { x: 0.0, y: 0.0 },
            WIDE,
            NODE_H
        ),
        Pt { x: 0.0, y: 0.0 }
    );
}

#[test]
fn free_spot_steps_below_overlapping_cards() {
    let taken = BTreeMap::from([
        ("a".to_owned(), Pt { x: 0.0, y: 0.0 }),
        (
            "b".to_owned(),
            Pt {
                x: 10.0,
                y: NODE_H + STACK_GAP,
            },
        ),
    ]);
    // Two cards in the way: it lands two steps down.
    assert_eq!(
        free_spot(
            &Flow::default(),
            &taken,
            Pt { x: 0.0, y: 0.0 },
            WIDE,
            NODE_H
        ),
        Pt {
            x: 0.0,
            y: 2.0 * (NODE_H + STACK_GAP)
        }
    );
}

/// An area much larger than any test graph.
const WIDE: Rect = Rect {
    x: -5000.0,
    y: -5000.0,
    w: 10000.0,
    h: 10000.0,
};

fn overlaps_any(taken: &BTreeMap<String, Pt>, at: Pt) -> bool {
    taken.values().any(|p| {
        intersects(
            Rect {
                x: at.x,
                y: at.y,
                w: NODE_W,
                h: NODE_H,
            },
            Rect {
                x: p.x,
                y: p.y,
                w: NODE_W,
                h: NODE_H,
            },
        )
    })
}

#[test]
fn free_spot_stays_inside_the_area() {
    // A full column from the start point down past the bottom of the visible area.
    let area = Rect {
        x: 0.0,
        y: 0.0,
        w: 1000.0,
        h: 400.0,
    };
    let taken: BTreeMap<String, Pt> = (0..10)
        .map(|i| {
            (
                format!("n{i}"),
                Pt {
                    x: 300.0,
                    y: 150.0 + f64::from(i) * 40.0,
                },
            )
        })
        .collect();
    let at = free_spot(
        &Flow::default(),
        &taken,
        Pt { x: 300.0, y: 150.0 },
        area,
        NODE_H,
    );
    assert!(!overlaps_any(&taken, at), "{at:?}");
    assert!(at.x >= area.x && at.x + NODE_W <= area.x + area.w, "{at:?}");
    assert!(at.y >= area.y && at.y + NODE_H <= area.y + area.h, "{at:?}");
}

#[test]
fn free_spot_falls_back_to_the_start_when_full() {
    let area = Rect {
        x: 0.0,
        y: 0.0,
        w: 300.0,
        h: 100.0,
    };
    let taken = BTreeMap::from([
        ("a".to_owned(), Pt { x: 0.0, y: 0.0 }),
        ("b".to_owned(), Pt { x: 0.0, y: 50.0 }),
    ]);
    let start = Pt { x: 20.0, y: 20.0 };
    assert_eq!(
        free_spot(&Flow::default(), &taken, start, area, NODE_H),
        start
    );
}

fn led() -> Flow {
    Flow::decode(include_str!("../../tests/fixtures/living_led_motion.json").as_bytes()).unwrap()
}

fn intersects(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

fn inside(p: Pt, r: Rect) -> bool {
    p.x > r.x && p.x < r.x + r.w && p.y > r.y && p.y < r.y + r.h
}

fn lerp(a: Pt, b: Pt, t: f64) -> Pt {
    Pt {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
    }
}

/// `n` points along an SVG path of `M`, `C` and `L` commands, spread evenly over its
/// segments (by parameter, not by length).
fn route_samples(d: &str, n: usize) -> Vec<Pt> {
    let mut spaced = String::new();
    for ch in d.chars() {
        if ch.is_ascii_alphabetic() {
            spaced.push(' ');
            spaced.push(ch);
            spaced.push(' ');
        } else if ch == ',' {
            spaced.push(' ');
        } else {
            spaced.push(ch);
        }
    }
    let mut commands: Vec<(char, Vec<f64>)> = Vec::new();
    for token in spaced.split_whitespace() {
        match token.parse::<f64>() {
            Ok(v) => commands
                .last_mut()
                .expect("path starts with a command")
                .1
                .push(v),
            Err(_) => commands.push((token.chars().next().unwrap(), Vec::new())),
        }
    }
    let pt = |v: &[f64], i: usize| Pt {
        x: v[2 * i],
        y: v[2 * i + 1],
    };
    let mut segments: Vec<Vec<Pt>> = Vec::new();
    let mut cur = Pt { x: 0.0, y: 0.0 };
    for (c, v) in &commands {
        match c {
            'M' => cur = pt(v, 0),
            'L' => {
                segments.push(vec![cur, pt(v, 0)]);
                cur = pt(v, 0);
            }
            'C' => {
                segments.push(vec![cur, pt(v, 0), pt(v, 1), pt(v, 2)]);
                cur = pt(v, 2);
            }
            other => panic!("unexpected path command {other} in {d}"),
        }
    }
    (0..n)
        .map(|i| {
            let g = i as f64 / (n - 1) as f64 * segments.len() as f64;
            let s = (g.floor() as usize).min(segments.len() - 1);
            let t = g - s as f64;
            let p = &segments[s];
            if p.len() == 2 {
                lerp(p[0], p[1], t)
            } else {
                let a = lerp(lerp(p[0], p[1], t), lerp(p[1], p[2], t), t);
                let b = lerp(lerp(p[1], p[2], t), lerp(p[2], p[3], t), t);
                lerp(a, b, t)
            }
        })
        .collect()
}

#[test]
fn led_layout_has_no_overlaps() {
    let pos = auto_layout(&led());
    assert_eq!(pos.len(), 23);
    let cards: Vec<_> = pos.iter().collect();
    for (i, (a, pa)) in cards.iter().enumerate() {
        assert!(pa.x >= PAD && pa.y >= PAD, "{a} at {pa:?}");
        for (b, pb) in &cards[i + 1..] {
            assert!(
                !intersects(card(**pa, NODE_H), card(**pb, NODE_H)),
                "{a} overlaps {b}"
            );
        }
    }
}

#[test]
fn led_routes_avoid_cards() {
    let flow = led();
    let pos = auto_layout(&flow);
    let all = routes(&flow, &pos);
    assert_eq!(all.len(), flow.edges.len());
    for ((from, port, to), route) in &all {
        for p in route_samples(&route.d, 48) {
            for (id, at) in &pos {
                if id == from || id == to {
                    continue;
                }
                assert!(
                    !inside(p, card(*at, NODE_H)),
                    "{from} -{port:?}-> {to} crosses {id} at {p:?}: {}",
                    route.d
                );
            }
        }
    }
}

#[test]
fn wires_end_on_the_input_pin() {
    // The canvas draws one input pin, halfway down the left edge (graph.rs), so every wire
    // into a card, however many there are, must end on it.
    let flow = led();
    let pos = auto_layout(&flow);
    let all = routes(&flow, &pos);
    let into_tv = all.iter().filter(|((_, _, to), _)| to == "tv_on").count();
    assert_eq!(into_tv, 5);
    for ((from, port, to), route) in &all {
        let end = *route_samples(&route.d, 48).last().unwrap();
        let pin = Pt {
            x: pos[to].x,
            y: pos[to].y + NODE_H / 2.0,
        };
        assert!(
            (end.x - pin.x).abs() < 1e-6 && (end.y - pin.y).abs() < 1e-6,
            "{from} -{port:?}-> {to} ends at {end:?}, pin at {pin:?}"
        );
    }
}

#[test]
fn if_ports() {
    let flow = led();
    let kind = &flow.node("tv_on").unwrap().kind;
    let origin = Pt { x: 0.0, y: 0.0 };
    assert_eq!(out_anchor(kind, origin, Some("true")).y, NODE_H * 0.3);
    assert_eq!(out_anchor(kind, origin, Some("false")).y, NODE_H * 0.7);
    assert_eq!(out_anchor(kind, origin, Some("true")).x, NODE_W);
    let wait = &flow.node("wait").unwrap().kind;
    assert_eq!(
        out_anchor(wait, Pt { x: 10.0, y: 20.0 }, None),
        Pt {
            x: 10.0 + NODE_W,
            y: 20.0 + NODE_H / 2.0
        }
    );
}

fn switch_kind(rules: usize, otherwise: bool) -> NodeKind {
    let node: nidoo_core::flow::Node = serde_json::from_value(serde_json::json!({
        "id": "s", "type": "switch",
        "params": { "entity": "e", "otherwise": otherwise,
            "rules": (0..rules).map(|i| serde_json::json!({ "op": "gt", "value": i })).collect::<Vec<_>>() }
    }))
    .unwrap();
    node.kind
}

#[test]
fn switch_card_grows_with_exits() {
    assert_eq!(node_height(&switch_kind(1, false)), NODE_H);
    assert_eq!(node_height(&switch_kind(1, true)), 54.0);
    assert_eq!(node_height(&switch_kind(5, true)), 134.0);
    let kind = switch_kind(5, true);
    let height = node_height(&kind);
    let origin = Pt { x: 0.0, y: 0.0 };
    let ports = kind.ports();
    let ys: Vec<f64> = ports
        .iter()
        .map(|p| out_anchor(&kind, origin, Some(p)).y)
        .collect();
    assert!(ys.windows(2).all(|w| w[0] < w[1]), "{ys:?}");
    assert!(ys.iter().all(|y| *y > 0.0 && *y < height), "{ys:?}");
    // Other cards keep the standard height.
    assert_eq!(node_height(&led().node("tv_on").unwrap().kind), NODE_H);
}

#[test]
fn time_card_pins_sit_at_30_and_70_percent() {
    let node: nidoo_core::flow::Node = serde_json::from_value(serde_json::json!({
        "id": "t", "type": "time.between", "params": { "from": "07:00", "to": "22:00" }
    }))
    .unwrap();
    let o = Pt { x: 0.0, y: 0.0 };
    assert_eq!(out_anchor(&node.kind, o, Some("in")).y, NODE_H * 0.3);
    assert_eq!(out_anchor(&node.kind, o, Some("out")).y, NODE_H * 0.7);
}

#[test]
fn backup_card_grows_and_spreads_its_three_exits() {
    let node: nidoo_core::flow::Node = serde_json::from_value(serde_json::json!({
        "id": "b", "type": "backup.run", "params": {}
    }))
    .unwrap();
    let height = node_height(&node.kind);
    assert_eq!(height, 74.0);
    let o = Pt { x: 0.0, y: 0.0 };
    let ys: Vec<f64> = ["ok", "partial", "failed"]
        .iter()
        .map(|p| out_anchor(&node.kind, o, Some(p)).y)
        .collect();
    // The same spacing as a switch: one row of EXIT_H per exit, centred in its row.
    assert!(ys.windows(2).all(|w| w[1] - w[0] >= EXIT_H), "{ys:?}");
    assert_eq!(ys[1], height * 0.5);
    assert!(ys[0] > 0.0 && ys[2] < height);
}

#[test]
fn switch_wires_end_on_their_pins() {
    let flow = Flow::decode(
        br#"{
          "id": "f", "name": "F",
          "nodes": [
            { "id": "t", "type": "trigger.state", "params": { "entity": "e" } },
            { "id": "s", "type": "switch", "params": { "entity": "e", "otherwise": true,
              "rules": [ { "op": "gt", "value": 1 }, { "op": "gt", "value": 2 }, { "op": "gt", "value": 3 } ] } },
            { "id": "a", "type": "delay", "params": { "for": "1m" } },
            { "id": "b", "type": "delay", "params": { "for": "1m" } },
            { "id": "c", "type": "delay", "params": { "for": "1m" } },
            { "id": "d", "type": "delay", "params": { "for": "1m" } }
          ],
          "edges": [
            { "from": "t", "to": "s" },
            { "from": "s", "port": "1", "to": "a" },
            { "from": "s", "port": "2", "to": "b" },
            { "from": "s", "port": "3", "to": "c" },
            { "from": "s", "port": "otherwise", "to": "d" }
          ]
        }"#,
    )
    .unwrap();
    let pos = auto_layout(&flow);
    let kind = &flow.node("s").unwrap().kind;
    assert_eq!(node_height(kind), 94.0);
    for ((from, port, to), route) in &routes(&flow, &pos) {
        let samples = route_samples(&route.d, 48);
        let (start, end) = (samples[0], *samples.last().unwrap());
        let pin_out = out_anchor(&flow.node(from).unwrap().kind, pos[from], port.as_deref());
        let pin_in = in_anchor(pos[to], height_of(&flow, to));
        // The path text keeps one decimal.
        let near = |a: Pt, b: Pt| (a.x - b.x).abs() < 0.06 && (a.y - b.y).abs() < 0.06;
        assert!(
            near(start, pin_out),
            "{from} {port:?}: {start:?} vs {pin_out:?}"
        );
        assert!(near(end, pin_in), "{to}: {end:?} vs {pin_in:?}");
    }
    // No two cards of the laid-out flow overlap, the tall switch included.
    let rects: Vec<Rect> = pos
        .iter()
        .map(|(id, at)| card(*at, height_of(&flow, id)))
        .collect();
    for (i, a) in rects.iter().enumerate() {
        for b in &rects[i + 1..] {
            assert!(!intersects(*a, *b), "{a:?} {b:?}");
        }
    }
}

fn placed(flow: &mut Flow) {
    for (i, n) in flow.nodes.iter_mut().enumerate() {
        n.position = Some(Position {
            x: 10.0 * i as f64,
            y: 5.0,
        });
    }
}

#[test]
fn stored_positions_win() {
    let mut flow = led();
    placed(&mut flow);
    let pos = positions(&flow);
    assert_eq!(pos.len(), 23);
    for (i, n) in flow.nodes.iter().enumerate() {
        assert_eq!(
            pos[&n.id],
            Pt {
                x: 10.0 * i as f64,
                y: 5.0
            }
        );
    }
}

#[test]
fn partial_positions_autolayout() {
    let mut flow = led();
    placed(&mut flow);
    flow.nodes[3].position = None;
    assert_eq!(positions(&flow), auto_layout(&flow));
}

#[test]
fn tidy_overwrites_positions() {
    let mut flow = led();
    placed(&mut flow);
    let auto = auto_layout(&flow);
    let tidied = tidy(&flow);
    for n in &tidied.nodes {
        let p = n.position.expect("tidy sets every position");
        assert_eq!(Pt { x: p.x, y: p.y }, auto[&n.id], "{}", n.id);
    }
}

#[test]
fn resolve_positions_fills_only_the_missing_ones() {
    let mut flow = led();
    flow.nodes[0].position = Some(Position { x: 10.0, y: 20.0 });
    let auto = auto_layout(&flow);
    let resolved = with_resolved_positions(&flow);
    assert_eq!(
        resolved.nodes[0].position,
        Some(Position { x: 10.0, y: 20.0 })
    );
    for n in &resolved.nodes[1..] {
        let p = n.position.expect("filled");
        assert_eq!(Pt { x: p.x, y: p.y }, auto[&n.id], "{}", n.id);
    }
}

#[test]
fn crossings_reduced() {
    let flow = Flow::decode(
        br#"{
          "id": "x", "name": "X",
          "nodes": [
            { "id": "a", "type": "trigger.state", "params": { "entity": "m" } },
            { "id": "b", "type": "trigger.state", "params": { "entity": "n" } },
            { "id": "c", "type": "command", "params": { "entity": "l", "command": { "type": "turn_on" } } },
            { "id": "d", "type": "command", "params": { "entity": "l", "command": { "type": "turn_off" } } }
          ],
          "edges": [ { "from": "a", "to": "d" }, { "from": "b", "to": "c" } ]
        }"#,
    )
    .unwrap();
    let pos = auto_layout(&flow);
    assert!(pos["a"].y < pos["b"].y);
    assert!(pos["d"].y < pos["c"].y, "{pos:?}");
    assert_eq!(pos["a"].x, PAD);
    assert_eq!(pos["c"].x, PAD + NODE_W + COL_GAP);
}

#[test]
fn bounds_cover_every_card() {
    let pos = BTreeMap::from([
        ("a".to_owned(), Pt { x: 40.0, y: 100.0 }),
        ("b".to_owned(), Pt { x: 400.0, y: 10.0 }),
    ]);
    assert_eq!(
        bounds(&Flow::default(), &pos),
        Rect {
            x: 40.0,
            y: 10.0,
            w: 360.0 + NODE_W,
            h: 90.0 + NODE_H
        }
    );
    assert_eq!(
        bounds(&Flow::default(), &BTreeMap::new()),
        Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0
        }
    );
}

#[test]
fn clear_route_is_one_cubic_with_label_and_middle() {
    let r = route(Pt { x: 0.0, y: 0.0 }, Pt { x: 400.0, y: 0.0 }, &[]);
    assert_eq!(r.d, "M0.0 0.0 C200.0 0.0, 200.0 0.0, 400.0 0.0");
    assert_eq!(
        r.label_at,
        Pt {
            x: LABEL_DX,
            y: 0.0
        }
    );
    assert!((r.mid.x - 200.0).abs() < 1.0, "{:?}", r.mid);
}

#[test]
fn if_labels_stay_apart_when_branches_bend_toward_each_other() {
    // `true` leaves the upper pin but goes down, `false` leaves the lower pin but goes up:
    // the two wires cross right after the card.
    let flow = Flow::decode(
        br#"{
          "id": "f", "name": "F",
          "nodes": [
            { "id": "check", "type": "if", "params": { "op": "eq", "value": true },
              "position": { "x": 0, "y": 200 } },
            { "id": "down", "type": "delay", "params": { "for": "1m" },
              "position": { "x": 328, "y": 320 } },
            { "id": "up", "type": "delay", "params": { "for": "1m" },
              "position": { "x": 328, "y": 80 } }
          ],
          "edges": [
            { "from": "check", "port": "true", "to": "down" },
            { "from": "check", "port": "false", "to": "up" }
          ]
        }"#,
    )
    .unwrap();
    let wires = routes(&flow, &positions(&flow));
    let label = |port: &str| {
        wires
            .iter()
            .find(|(k, _)| k.1.as_deref() == Some(port))
            .map(|(_, r)| r.label_at)
            .unwrap()
    };
    let (yes, no) = (label("true"), label("false"));
    assert!((no.y - yes.y).abs() >= 16.0, "yes {yes:?} no {no:?}");
    // Each label sits just right of its own pin.
    assert!(yes.x > NODE_W && yes.x < NODE_W + COL_GAP / 2.0, "{yes:?}");
}

#[test]
fn blocked_route_takes_a_lane() {
    let block = Rect {
        x: 150.0,
        y: 74.0,
        w: NODE_W,
        h: NODE_H,
    };
    let from = Pt { x: 0.0, y: 100.0 };
    let to = Pt { x: 600.0, y: 100.0 };
    let r = route(from, to, &[block]);
    assert!(r.d.contains('L'), "{}", r.d);
    for p in route_samples(&r.d, 48) {
        assert!(!inside(p, block), "{p:?} in {}", r.d);
    }
    // Lane at the top, ROW_GAP/2 above the card (both lanes cost the same; top comes first).
    assert!(
        r.d.contains(&format!("{:.1}", block.y - ROW_GAP / 2.0)),
        "{}",
        r.d
    );
}

/// A trigger wired to a delay 700 px to its right, with a loose delay between them.
fn wire_and_loose_card(loose_y: f64) -> Flow {
    Flow::decode(
        format!(
            r#"{{
          "id": "f", "name": "F",
          "nodes": [
            {{ "id": "t", "type": "trigger.state", "params": {{ "entity": "e" }},
              "position": {{ "x": 0, "y": 100 }} }},
            {{ "id": "d", "type": "delay", "params": {{ "for": "1m" }},
              "position": {{ "x": 700, "y": 100 }} }},
            {{ "id": "n", "type": "delay", "params": {{ "for": "1m" }},
              "position": {{ "x": 350, "y": {loose_y} }} }}
          ],
          "edges": [ {{ "from": "t", "to": "d" }} ]
        }}"#
        )
        .as_bytes(),
    )
    .unwrap()
}

#[test]
fn loose_card_leaves_the_wire_under_it() {
    let flow = wire_and_loose_card(90.0);
    let pos = positions(&flow);
    // Routed around it, the wire dodges the card; ignoring it, the wire runs straight under.
    assert_eq!(wire_under(&routes(&flow, &pos), pos["n"], NODE_H), None);
    let wires = routes_past(&flow, &pos, Some("n"));
    assert_eq!(
        wire_under(&wires, pos["n"], NODE_H),
        Some(("t".into(), None, "d".into()))
    );
}

#[test]
fn card_away_from_the_wire_finds_none() {
    let flow = wire_and_loose_card(300.0);
    let pos = positions(&flow);
    assert_eq!(
        wire_under(&routes_past(&flow, &pos, Some("n")), pos["n"], NODE_H),
        None
    );
}

/// Three triggers into one `if` whose two exits go to two commands, then a plain chain.
fn fan_flow() -> Flow {
    Flow::decode(
        br#"{
          "id": "f", "name": "F",
          "nodes": [
            { "id": "t1", "type": "trigger.state", "params": { "entity": "a" } },
            { "id": "t2", "type": "trigger.state", "params": { "entity": "b" } },
            { "id": "t3", "type": "trigger.state", "params": { "entity": "c" } },
            { "id": "check", "type": "if", "params": { "op": "eq", "value": true } },
            { "id": "yes", "type": "command", "params": { "entity": "l", "command": { "type": "turn_on" } } },
            { "id": "no", "type": "delay", "params": { "for": "1m" } },
            { "id": "off", "type": "command", "params": { "entity": "l", "command": { "type": "turn_off" } } }
          ],
          "edges": [
            { "from": "t1", "to": "check" }, { "from": "t2", "to": "check" }, { "from": "t3", "to": "check" },
            { "from": "check", "port": "true", "to": "yes" },
            { "from": "check", "port": "false", "to": "no" },
            { "from": "no", "to": "off" }
          ]
        }"#,
    )
    .unwrap()
}

#[test]
fn fans_get_wider_gaps() {
    let pos = auto_layout(&fan_flow());
    // Fan-in: the triggers sit further apart, and so do their column and the if's.
    let mut ys: Vec<f64> = ["t1", "t2", "t3"].iter().map(|id| pos[*id].y).collect();
    ys.sort_by(f64::total_cmp);
    for w in ys.windows(2) {
        assert_eq!(w[1] - w[0], NODE_H + FAN_ROW_GAP, "{ys:?}");
    }
    assert_eq!(pos["check"].x - pos["t1"].x, NODE_W + FAN_COL_GAP);
    // Fan-out: the if's two exits sit further apart, one column further right.
    assert_eq!((pos["yes"].y - pos["no"].y).abs(), NODE_H + FAN_ROW_GAP);
    assert_eq!(pos["yes"].x - pos["check"].x, NODE_W + FAN_COL_GAP);
    // A plain one-to-one wire keeps the standard gap.
    assert_eq!(pos["off"].x - pos["no"].x, NODE_W + COL_GAP);
}

#[test]
fn plain_column_keeps_the_standard_row_gap() {
    // Two independent chains: their cards share no neighbour and none fans.
    let flow = Flow::decode(
        br#"{
          "id": "p", "name": "P",
          "nodes": [
            { "id": "a", "type": "trigger.state", "params": { "entity": "a" } },
            { "id": "b", "type": "trigger.state", "params": { "entity": "b" } },
            { "id": "c", "type": "command", "params": { "entity": "l", "command": { "type": "turn_on" } } },
            { "id": "d", "type": "command", "params": { "entity": "l", "command": { "type": "turn_off" } } }
          ],
          "edges": [ { "from": "a", "to": "c" }, { "from": "b", "to": "d" } ]
        }"#,
    )
    .unwrap();
    let pos = auto_layout(&flow);
    assert_eq!((pos["b"].y - pos["a"].y).abs(), NODE_H + ROW_GAP);
    assert_eq!(pos["c"].x - pos["a"].x, NODE_W + COL_GAP);
}

#[test]
fn fan_layout_has_no_overlaps_and_routes_avoid_cards() {
    let flow = fan_flow();
    let pos = auto_layout(&flow);
    for ((from, port, to), route) in &routes(&flow, &pos) {
        for p in route_samples(&route.d, 48) {
            for (id, at) in &pos {
                if id != from && id != to {
                    assert!(
                        !inside(p, card(*at, NODE_H)),
                        "{from} -{port:?}-> {to} crosses {id}"
                    );
                }
            }
        }
    }
}
