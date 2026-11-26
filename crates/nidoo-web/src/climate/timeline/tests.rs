use super::*;
use nidoo_climate::doc::{Block, Rules};

fn block(days: &[Day], from: &str, to: &str, t: f64) -> Block {
    Block {
        days: days.to_vec(),
        from: HhMm::parse(from).unwrap(),
        to: HhMm::parse(to).unwrap(),
        setpoint: Setpoint::Temp(t),
        evidence: None,
        heated_by: None,
    }
}

fn mode(blocks: Vec<Block>) -> ModeRules {
    ModeRules {
        base: Setpoint::Temp(17.0),
        blocks,
    }
}

fn valid(m: &ModeRules) {
    Rules {
        heat: Some(m.clone()),
        cool: None,
    }
    .validate()
    .expect("still valid");
}

fn at(s: &str) -> HhMm {
    HhMm::parse(s).unwrap()
}

#[test]
fn set_base_clamps_a_temperature_and_keeps_the_blocks() {
    let m = mode(vec![block(&[Day::Mon], "07:00", "09:00", 21.0)]);
    let next = set_base(&m, Setpoint::Temp(15.5));
    assert_eq!(next.base, Setpoint::Temp(15.5));
    assert_eq!(next.blocks, m.blocks);
    assert_eq!(
        set_base(&m, Setpoint::Temp(55.0)).base,
        Setpoint::Temp(40.0)
    );
    assert_eq!(set_base(&m, Setpoint::Off).base, Setpoint::Off);
    assert_eq!(set_base(&m, Setpoint::Temp(f64::NAN)), m);
    valid(&set_base(&m, Setpoint::Off));
}

#[test]
fn peek_index_follows_the_block_not_its_place() {
    let morning = block(&[Day::Mon, Day::Tue], "07:00", "09:00", 21.0);
    let evening = block(&[Day::Mon], "18:00", "22:00", 20.0);
    let key = PeekKey::of(&evening, Day::Mon);
    assert_eq!(
        peek_index(&mode(vec![morning.clone(), evening.clone()]), key),
        Some(1)
    );
    // Reordered: still the evening block.
    assert_eq!(
        peek_index(&mode(vec![evening.clone(), morning.clone()]), key),
        Some(0)
    );
    // Removed, or moved to other times: nothing is peeked.
    assert_eq!(peek_index(&mode(vec![morning.clone()]), key), None);
    let later = block(&[Day::Mon], "19:00", "22:00", 20.0);
    assert_eq!(peek_index(&mode(vec![morning, later]), key), None);
}

#[test]
fn first_day_is_the_earliest_in_the_week() {
    assert_eq!(
        first_day(&block(&[Day::Sun, Day::Wed], "07:00", "09:00", 21.0)),
        Some(Day::Wed)
    );
    assert_eq!(first_day(&block(&[], "07:00", "09:00", 21.0)), None);
}

#[test]
fn peek_text_reads_times_temp_and_evidence() {
    let mut b = block(&[Day::Mon], "07:15", "14:00", 18.5);
    assert_eq!(peek_text(Lang::En, &b), "07:15 – 14:00 · 18.5°");
    b.evidence = Some(nidoo_climate::doc::Evidence { agree: 3, of: 5 });
    assert_eq!(peek_text(Lang::En, &b), "07:15 – 14:00 · 18.5° · 3/5");
    b.evidence = None;
    b.setpoint = Setpoint::Off;
    assert_eq!(peek_text(Lang::En, &b), "07:15 – 14:00 · Off");
    assert_eq!(peek_text(Lang::Fr, &b), "07:15 – 14:00 · Arrêt");
    b.setpoint = Setpoint::Temp(18.5);
    assert_eq!(peek_text(Lang::Fr, &b), "07:15 – 14:00 · 18,5°");
}

#[test]
fn move_edge_snaps_to_15_min() {
    let m = mode(vec![block(&[Day::Mon], "07:00", "09:00", 21.0)]);
    let a = move_edge(&m, 0, Edge::From, at("06:53"));
    assert_eq!(a.blocks[0].from, at("07:00"));
    let b = move_edge(&m, 0, Edge::From, at("06:38"));
    assert_eq!(b.blocks[0].from, at("06:45"));
    let c = move_edge(&m, 0, Edge::To, at("10:07"));
    assert_eq!(c.blocks[0].to, at("10:00"));
    let d = move_edge(&m, 0, Edge::To, at("23:59"));
    assert_eq!(d.blocks[0].to, at("24:00"));
    for r in [a, b, c, d] {
        valid(&r);
    }
}

#[test]
fn move_edge_clamps_to_neighbour() {
    // Block 0 shares Monday with both neighbours; block 3 is on another day.
    let m = mode(vec![
        block(&[Day::Mon, Day::Tue], "09:00", "11:00", 21.0),
        block(&[Day::Mon], "06:00", "08:10", 19.0),
        block(&[Day::Mon], "12:00", "14:00", 19.0),
        block(&[Day::Wed], "08:30", "08:45", 19.0),
    ]);
    let earlier = move_edge(&m, 0, Edge::From, at("05:00"));
    assert_eq!(earlier.blocks[0].from, at("08:10"));
    let later = move_edge(&m, 0, Edge::To, at("13:00"));
    assert_eq!(later.blocks[0].to, at("12:00"));
    // Wednesday has no neighbour that blocks the move.
    let free = move_edge(&m, 3, Edge::To, at("20:00"));
    assert_eq!(free.blocks[3].to, at("20:00"));
    // Block 1 shares Monday with block 0, so its end stops at 09:00.
    let open = move_edge(&m, 1, Edge::To, at("08:55"));
    assert_eq!(open.blocks[1].to, at("09:00"));
    for r in [earlier, later, free, open] {
        valid(&r);
    }
}

#[test]
fn move_edge_keeps_from_before_to() {
    let m = mode(vec![block(&[Day::Mon], "07:00", "09:00", 21.0)]);
    let a = move_edge(&m, 0, Edge::From, at("12:00"));
    assert_eq!(a.blocks[0].from, at("08:45"));
    let b = move_edge(&m, 0, Edge::To, at("03:00"));
    assert_eq!(b.blocks[0].to, at("07:15"));
    let c = move_edge(&m, 0, Edge::From, at("00:00"));
    assert_eq!(c.blocks[0].from, at("00:00"));
    // Unknown block: unchanged.
    assert_eq!(move_edge(&m, 5, Edge::To, at("10:00")), m);
    for r in [a, b, c] {
        valid(&r);
    }
}

#[test]
fn move_edge_reaches_an_off_grid_neighbour_edge() {
    // Block 0 ends at 08:12; the next one starts at 08:14, so no 15-minute step fits.
    let m = mode(vec![
        block(&[Day::Mon], "08:10", "08:12", 21.0),
        block(&[Day::Mon], "08:14", "09:00", 19.0),
    ]);
    let out = move_edge(&m, 0, Edge::To, at("09:00"));
    assert_eq!(out.blocks[0].to, at("08:14"));
    valid(&out);
    // And the other way: block 1 can still go back to 08:12.
    let m = mode(vec![
        block(&[Day::Mon], "07:00", "08:10", 21.0),
        block(&[Day::Mon], "08:12", "08:14", 19.0),
    ]);
    let out = move_edge(&m, 1, Edge::From, at("06:00"));
    assert_eq!(out.blocks[1].from, at("08:10"));
    valid(&out);
}

#[test]
fn editing_a_block_drops_its_evidence_only() {
    let mut m = mode(vec![
        block(&[Day::Mon], "07:00", "09:00", 21.0),
        block(&[Day::Tue], "07:00", "09:00", 21.0),
    ]);
    for b in &mut m.blocks {
        b.evidence = Some(nidoo_climate::doc::Evidence { agree: 17, of: 20 });
    }
    let moved = move_edge(&m, 0, Edge::To, at("10:00"));
    assert_eq!(moved.blocks[0].evidence, None);
    assert!(moved.blocks[1].evidence.is_some());
    // A refused move or an unchanged value keeps it.
    assert_eq!(move_edge(&m, 0, Edge::To, at("09:00")), m);
    assert_eq!(set_setpoint(&m, 0, Setpoint::Temp(21.0)), m);
    let off = set_setpoint(&m, 1, Setpoint::Off);
    assert_eq!(off.blocks[1].evidence, None);
    assert!(off.blocks[0].evidence.is_some());
}

#[test]
fn restore_temp_prefers_base_then_nearest_block() {
    let mut m = mode(vec![
        block(&[Day::Mon], "06:00", "07:00", 19.0),
        block(&[Day::Mon], "08:00", "09:00", 21.0),
        block(&[Day::Mon], "20:00", "22:00", 23.0),
    ]);
    m.blocks[1].setpoint = Setpoint::Off;
    assert_eq!(restore_temp(&m, 1), Some(17.0));
    m.base = Setpoint::Off;
    // 06:00-07:00 is an hour before 08:00; 20:00 is eleven hours after 09:00.
    assert_eq!(restore_temp(&m, 1), Some(19.0));
    m.blocks.truncate(2);
    m.blocks[0].setpoint = Setpoint::Off;
    assert_eq!(restore_temp(&m, 1), None);
}

#[test]
fn time_choices_include_an_off_grid_edge() {
    let c = time_choices(490, 0);
    assert!(c.contains(&490));
    assert!(c.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(time_choices(495, 15).len(), 96);
}

#[test]
fn set_setpoint_off() {
    let m = mode(vec![
        block(&[Day::Mon], "07:00", "09:00", 21.0),
        block(&[Day::Tue], "07:00", "09:00", 21.0),
    ]);
    let off = set_setpoint(&m, 1, Setpoint::Off);
    assert_eq!(off.blocks[1].setpoint, Setpoint::Off);
    assert_eq!(off.blocks[0].setpoint, Setpoint::Temp(21.0));
    let hot = set_setpoint(&m, 0, Setpoint::Temp(99.0));
    assert_eq!(hot.blocks[0].setpoint, Setpoint::Temp(40.0));
    assert_eq!(set_setpoint(&m, 0, Setpoint::Temp(f64::NAN)), m);
    valid(&off);
    valid(&hot);
}

#[test]
fn delete_block_keeps_others() {
    let m = mode(vec![
        block(&[Day::Mon], "07:00", "09:00", 21.0),
        block(&[Day::Tue], "08:00", "10:00", 20.0),
        block(&[Day::Wed], "09:00", "11:00", 19.0),
    ]);
    let out = delete_block(&m, 1);
    assert_eq!(out.blocks, vec![m.blocks[0].clone(), m.blocks[2].clone()]);
    assert_eq!(out.base, m.base);
    assert_eq!(delete_block(&m, 9), m);
    valid(&out);
}

#[test]
fn toggle_day_adds_a_free_day_and_removes_one() {
    let mut m = mode(vec![
        block(&[Day::Mon, Day::Tue, Day::Wed], "00:00", "07:30", 18.0),
        block(&[Day::Sat], "00:00", "07:30", 18.0),
    ]);
    m.blocks[1].evidence = Some(nidoo_climate::doc::Evidence { agree: 1, of: 1 });
    let sunday = toggle_day(&m, 1, Day::Sun);
    assert_eq!(sunday.blocks[1].days, vec![Day::Sat, Day::Sun]);
    assert_eq!(sunday.blocks[1].evidence, None);
    let monday = toggle_day(&m, 1, Day::Mon);
    assert_eq!(monday, m, "Monday already has an overlapping block");
    let tue_off = toggle_day(&m, 0, Day::Tue);
    assert_eq!(tue_off.blocks[0].days, vec![Day::Mon, Day::Wed]);
    // Days stay in week order.
    let fri = toggle_day(&m, 0, Day::Fri);
    assert_eq!(
        fri.blocks[0].days,
        vec![Day::Mon, Day::Tue, Day::Wed, Day::Fri]
    );
    for r in [sunday, tue_off, fri] {
        valid(&r);
    }
}

fn one_block() -> ModeRules {
    mode(vec![block(&[Day::Mon], "06:00", "08:00", 19.0)])
}

fn heaters() -> Vec<(String, String)> {
    vec![
        ("heat_pump".to_string(), "Heat pump".to_string()),
        ("stove".to_string(), "Stove".to_string()),
    ]
}

#[test]
fn toggle_heated_by_adds_then_removes() {
    let r = toggle_heated_by(&one_block(), 0, "stove");
    assert_eq!(r.blocks[0].heated_by, Some(vec!["stove".to_string()]));
    let both = toggle_heated_by(&r, 0, "heat_pump");
    assert_eq!(
        both.blocks[0].heated_by,
        Some(vec!["stove".to_string(), "heat_pump".to_string()])
    );
    assert_eq!(toggle_heated_by(&r, 0, "stove").blocks[0].heated_by, None);
    assert_eq!(toggle_heated_by(&r, 3, "stove"), r, "unknown block");
    valid(&both);
}

#[test]
fn heated_by_label_joins_names_and_skips_unknown_ids() {
    let mut b = block(&[Day::Mon], "06:00", "08:00", 19.0);
    assert_eq!(heated_by_label(&b, &heaters()), None);
    b.heated_by = Some(vec!["stove".into()]);
    assert_eq!(heated_by_label(&b, &heaters()).as_deref(), Some("Stove"));
    // The order of the heaters list, not of the block.
    b.heated_by = Some(vec!["stove".into(), "heat_pump".into(), "gone".into()]);
    assert_eq!(
        heated_by_label(&b, &heaters()).as_deref(),
        Some("Heat pump + Stove")
    );
    b.heated_by = Some(vec!["gone".into()]);
    assert_eq!(heated_by_label(&b, &heaters()), None);
    assert_eq!(heated_by_label(&b, &[]), None);
}

#[test]
fn toggle_day_keeps_the_last_day() {
    let m = mode(vec![block(&[Day::Sat], "00:00", "07:30", 18.0)]);
    assert_eq!(toggle_day(&m, 0, Day::Sat), m);
    assert_eq!(toggle_day(&m, 4, Day::Sun), m);
}

#[test]
fn day_fits_only_where_no_block_overlaps() {
    let m = mode(vec![
        block(&[Day::Mon], "06:00", "08:00", 19.0),
        block(&[Day::Tue], "07:00", "09:00", 21.0),
        block(&[Day::Wed], "08:00", "10:00", 21.0),
    ]);
    // Block 1 (07:00-09:00) overlaps Monday's 06:00-08:00 and Wednesday's 08:00-10:00.
    assert!(!day_fits(&m, 1, Day::Mon));
    assert!(!day_fits(&m, 1, Day::Wed));
    assert!(day_fits(&m, 1, Day::Thu));
    // Its own day always fits.
    assert!(day_fits(&m, 1, Day::Tue));
    let touching = mode(vec![
        block(&[Day::Mon], "06:00", "07:00", 19.0),
        block(&[Day::Tue], "07:00", "09:00", 21.0),
    ]);
    assert!(day_fits(&touching, 1, Day::Mon));
}

#[test]
fn add_block_starts_on_the_tapped_quarter_for_two_hours() {
    let m = mode(vec![block(&[Day::Sat], "00:00", "07:30", 18.0)]);
    let (out, i) = add_block(&m, Day::Sun, 18 * 60 + 7).expect("free spot");
    assert_eq!(i, 1);
    assert_eq!(out.blocks[0], m.blocks[0]);
    let b = &out.blocks[1];
    assert_eq!(b.days, vec![Day::Sun]);
    assert_eq!((b.from, b.to), (at("18:00"), at("20:00")));
    // Base 17 °C + 2.
    assert_eq!(b.setpoint, Setpoint::Temp(19.0));
    assert_eq!(b.evidence, None);
    valid(&out);
}

#[test]
fn add_block_stops_at_the_next_block_and_the_day_end() {
    let m = mode(vec![block(&[Day::Mon, Day::Tue], "09:00", "11:00", 21.0)]);
    let (short, _) = add_block(&m, Day::Mon, 8 * 60 + 20).unwrap();
    assert_eq!(
        (short.blocks[1].from, short.blocks[1].to),
        (at("08:15"), at("09:00"))
    );
    let (late, _) = add_block(&m, Day::Mon, 23 * 60).unwrap();
    assert_eq!(late.blocks[1].to, at("24:00"));
    let (end, _) = add_block(&m, Day::Mon, 1440).unwrap();
    assert_eq!(
        (end.blocks[1].from, end.blocks[1].to),
        (at("23:45"), at("24:00"))
    );
    // Inside a block on that day: nothing to add.
    assert_eq!(add_block(&m, Day::Tue, 10 * 60), None);
    // The same time on a day the block does not cover is free.
    assert!(add_block(&m, Day::Wed, 10 * 60).is_some());
    for r in [short, late, end] {
        valid(&r);
    }
}

#[test]
fn add_block_temperature_follows_the_base() {
    let mut m = mode(Vec::new());
    m.base = Setpoint::Temp(39.0);
    let (hot, _) = add_block(&m, Day::Mon, 0).unwrap();
    assert_eq!(hot.blocks[0].setpoint, Setpoint::Temp(40.0));
    m.base = Setpoint::Off;
    let (off, _) = add_block(&m, Day::Mon, 0).unwrap();
    assert_eq!(off.blocks[0].setpoint, Setpoint::Temp(19.0));
}
