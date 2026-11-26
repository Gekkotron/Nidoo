use super::*;

fn text(s: &str) -> Option<Scalar> {
    Some(Scalar::Text(s.into()))
}

#[test]
fn x_frac_spans_the_window_and_clamps() {
    assert_eq!(x_frac(100, 100, 200), 0.0);
    assert_eq!(x_frac(150, 100, 200), 0.5);
    assert_eq!(x_frac(200, 100, 200), 1.0);
    assert_eq!(x_frac(50, 100, 200), 0.0);
    assert_eq!(x_frac(300, 100, 200), 1.0);
    assert_eq!(x_frac(5, 10, 10), 0.0);
}

#[test]
fn bands_run_to_the_next_change_and_the_last_to_the_end() {
    let b = bands(
        &[
            (0, text("docked")),
            (50, text("cleaning")),
            (80, text("docked")),
        ],
        0,
        100,
    );
    assert_eq!(b.len(), 3);
    assert_eq!((b[0].x0, b[0].x1), (0.0, 0.5));
    assert_eq!((b[1].x0, b[1].x1), (0.5, 0.8));
    assert_eq!((b[2].x0, b[2].x1), (0.8, 1.0));
    assert_eq!((b[1].start, b[1].end), (50, 80));
}

#[test]
fn a_value_keeps_its_colour_and_new_ones_take_the_next() {
    let b = bands(
        &[
            (0, text("docked")),
            (50, text("cleaning")),
            (80, text("docked")),
        ],
        0,
        100,
    );
    assert_eq!(b[0].color, Some(0));
    assert_eq!(b[1].color, Some(1));
    assert_eq!(b[2].color, Some(0));
}

#[test]
fn colours_wrap_after_the_palette() {
    let changes: Vec<_> = (0..7)
        .map(|i| (i * 10, Some(Scalar::Number(i as f64))))
        .collect();
    let b = bands(&changes, 0, 70);
    assert_eq!(b[5].color, Some(0));
    assert_eq!(b[6].color, Some(1));
}

#[test]
fn unknown_is_a_gap_without_colour_and_takes_no_palette_slot() {
    let b = bands(&[(0, None), (50, text("on"))], 0, 100);
    assert_eq!(b[0].color, None);
    assert_eq!(b[1].color, Some(0));
}

#[test]
fn a_change_before_the_window_starts_at_its_edge() {
    let b = bands(&[(-500, text("a")), (40, text("b"))], 0, 100);
    assert_eq!((b[0].start, b[0].x0, b[0].x1), (0, 0.0, 0.4));
}

#[test]
fn changes_at_or_after_the_end_and_empty_stretches_are_dropped() {
    let b = bands(&[(0, text("a")), (0, text("b")), (100, text("c"))], 0, 100);
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].value, text("b"));
    assert!(bands(&[], 0, 100).is_empty());
}

#[test]
fn numbers_read_whole_or_with_one_decimal() {
    assert_eq!(number_text(Lang::En, 52.0, Some("%")), "52\u{A0}%");
    assert_eq!(number_text(Lang::Fr, 21.5, Some("°C")), "21,5\u{A0}°C");
    assert_eq!(number_text(Lang::En, 3.0, None), "3");
}

#[test]
fn values_are_worded_by_kind_for_the_headline_only() {
    let v = text("cleaning");
    assert_eq!(
        scalar_text(Lang::Fr, Kind::Vacuum, None, None, v.as_ref()),
        "Nettoyage"
    );
    assert_eq!(
        scalar_text(Lang::En, Kind::Vacuum, None, None, v.as_ref()),
        "Cleaning"
    );
    assert_eq!(
        scalar_text(
            Lang::En,
            Kind::Vacuum,
            Some("fan_speed"),
            None,
            text("turbo").as_ref()
        ),
        "Turbo"
    );
    assert_eq!(scalar_text(Lang::Fr, Kind::Vacuum, None, None, None), DASH);
}

#[test]
fn booleans_use_the_on_off_words() {
    let on = Some(Scalar::Bool(true));
    assert_eq!(
        scalar_text(Lang::Fr, Kind::Light, None, None, on.as_ref()),
        on_off(Lang::Fr, true, true)
    );
    assert_eq!(
        scalar_text(Lang::Fr, Kind::Switch, None, None, on.as_ref()),
        on_off(Lang::Fr, false, true)
    );
}

#[test]
fn labels_reuse_the_field_names_and_word_the_headline() {
    assert_eq!(
        track_label(Lang::Fr, Kind::Vacuum, Some("battery")),
        "Batterie"
    );
    assert_eq!(
        track_label(Lang::En, Kind::Vacuum, Some("fan_speed")),
        "Fan speed"
    );
    assert_eq!(track_label(Lang::Fr, Kind::Vacuum, None), "Activité");
    assert_eq!(track_label(Lang::En, Kind::Vacuum, None), "Activity");
    assert_eq!(track_label(Lang::Fr, Kind::Climate, None), "État");
    assert_eq!(
        track_label(Lang::En, Kind::Generic, Some("water_level")),
        "Water level"
    );
}

#[test]
fn adjacent_equal_values_merge() {
    let p = periods(
        &[
            (0, text("docked")),
            (10, text("docked")),
            (50, text("cleaning")),
            (60, text("docked")),
        ],
        0,
        100,
    );
    assert_eq!(p.len(), 3);
    assert_eq!((p[0].start, p[0].end), (0, 50));
}

#[test]
fn the_longest_value_is_the_uncoloured_baseline() {
    // docked 0-90 (90 s) vs cleaning 90-100 (10 s)
    let p = periods(&[(0, text("docked")), (90, text("cleaning"))], 0, 100);
    assert_eq!(p[0].color, None);
    assert_eq!(
        p[1].color,
        Some(0),
        "first non-baseline value takes palette slot 0"
    );
    assert_eq!(baseline(&p), text("docked"));
}

#[test]
fn a_single_value_has_no_baseline_and_is_coloured() {
    let p = periods(&[(0, text("turbo"))], 0, 100);
    assert_eq!(baseline(&p), None);
    assert_eq!(p[0].color, Some(0));
}

#[test]
fn baseline_ties_go_to_the_first_seen_and_unknown_never_counts() {
    let p = periods(&[(0, None), (60, text("a")), (80, text("b"))], 0, 100);
    // null 60 s ignored; a 20 s, b 20 s -> a
    assert_eq!(baseline(&p), text("a"));
    assert_eq!(p[0].color, None);
}

#[test]
fn legend_lists_each_value_once_in_order() {
    let p = periods(
        &[
            (0, text("docked")),
            (40, text("cleaning")),
            (45, text("docked")),
            (90, text("error")),
        ],
        0,
        100,
    );
    assert_eq!(
        legend(&p),
        vec![
            (Scalar::Text("docked".into()), None),
            (Scalar::Text("cleaning".into()), Some(0)),
            (Scalar::Text("error".into()), Some(1)),
        ]
    );
}

#[test]
fn all_unknown_has_no_periods_or_legend() {
    let p = periods(&[(0, None)], 0, 100);
    assert!(p.iter().all(|b| b.color.is_none()));
    assert!(legend(&p).is_empty());
}

#[test]
fn value_at_reads_the_change_in_force() {
    let c = [(0, text("a")), (50, text("b"))];
    assert_eq!(value_at(&c, 49), text("a"));
    assert_eq!(value_at(&c, 50), text("b"));
    assert_eq!(value_at(&c, -1), None);
}

#[test]
fn a_short_run_names_a_long_bucket() {
    // Docked for a day, with a 9 minute cleaning inside the 1 h bucket 3600..7200.
    let c = [
        (0, text("docked")),
        (4000, text("cleaning")),
        (4540, text("docked")),
    ];
    let p = periods(&c, 0, 86_400);
    assert_eq!(word_in(&c, &p, 3600, 7200), text("cleaning"));
}

#[test]
fn with_no_coloured_value_the_word_in_force_names_the_span() {
    let c = [(0, text("docked")), (4000, text("cleaning"))];
    let p = periods(&c, 0, 86_400);
    // Before the cleaning: only the baseline is present.
    assert_eq!(word_in(&c, &p, 0, 3600), text("docked"));
    // Nothing coloured at all.
    let one = [(0, text("docked"))];
    assert_eq!(
        word_in(&one, &periods(&one, 0, 100), 10, 20),
        text("docked")
    );
}

#[test]
fn the_longest_coloured_value_wins() {
    let c = [
        (0, text("docked")),
        (100, text("cleaning")),
        (200, text("returning")),
        (500, text("docked")),
        (10_000, text("docked")),
    ];
    let p = periods(&c, 0, 20_000);
    assert_eq!(word_in(&c, &p, 0, 1000), text("returning"));
}

#[test]
fn a_widened_hover_finds_a_sliver_over_the_baseline() {
    let c = [
        (0, text("docked")),
        (500, text("cleaning")),
        (503, text("docked")),
    ];
    let p = periods(&c, 0, 1000);
    let at = |f: f64, tol: f64| band_near(&p, f, tol).and_then(|b| b.value.clone());
    assert_eq!(at(0.51, 0.0), text("docked"));
    assert_eq!(at(0.51, 0.01), text("cleaning"));
    assert_eq!(at(0.2, 0.01), text("docked"));
}
