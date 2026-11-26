use super::*;

#[test]
fn y_scale_adds_ten_percent_headroom() {
    let p = vec![(0, 20.0, 20.0, 20.0), (1, 30.0, 30.0, 30.0)];
    let s = y_scale(&p);
    assert!((s.min - 19.0).abs() < 1e-6);
    assert!((s.max - 31.0).abs() < 1e-6);
}

#[test]
fn y_scale_covers_the_min_max_band() {
    let p = vec![(0, 20.0, 10.0, 25.0), (1, 22.0, 21.0, 30.0)];
    let s = y_scale(&p);
    assert!((s.min - 8.0).abs() < 1e-6);
    assert!((s.max - 32.0).abs() < 1e-6);
}

#[test]
fn y_scale_falls_back_to_unit_range_on_empty() {
    let s = y_scale(&[]);
    assert_eq!((s.min, s.max), (0.0, 1.0));
}

#[test]
fn y_scale_expands_a_flat_series() {
    let p = vec![(0, 5.0, 5.0, 5.0), (1, 5.0, 5.0, 5.0)];
    let s = y_scale(&p);
    assert!((s.min - 4.5).abs() < 1e-6);
    assert!((s.max - 5.5).abs() < 1e-6);
}

#[test]
fn step_path_produces_horizontal_then_vertical_segments() {
    let p = vec![(0, 0.0, 0.0, 0.0), (10, 1.0, 1.0, 1.0)];
    let s = YScale { min: 0.0, max: 1.0 };
    let out = step_path(&p, s, 0, 10, 100.0, 50.0);
    assert!(out.d.contains("M0,50"));
    assert!(out.d.contains("H") && out.d.contains("V"));
}

#[test]
fn step_path_holds_a_single_point_until_the_window_end() {
    let p = vec![(50, 0.5, 0.0, 1.0)];
    let s = YScale { min: 0.0, max: 1.0 };
    let out = step_path(&p, s, 0, 100, 100.0, 50.0);
    assert_eq!(out.d, "M50,25H100");
    assert_eq!(out.band_d, "M50,0H100V50H50Z");
}

#[test]
fn step_path_places_short_history_at_the_right_of_the_window() {
    let p = vec![(80, 0.0, 0.0, 0.0), (90, 1.0, 1.0, 1.0)];
    let s = YScale { min: 0.0, max: 1.0 };
    let out = step_path(&p, s, 0, 100, 100.0, 50.0);
    assert_eq!(out.d, "M80,50H90V0H100");
}

#[test]
fn step_path_clamps_a_bucket_starting_before_the_window() {
    let p = vec![(-20, 0.0, 0.0, 0.0)];
    let s = YScale { min: 0.0, max: 1.0 };
    let out = step_path(&p, s, 0, 100, 100.0, 50.0);
    assert_eq!(out.d, "M0,50H100");
}

#[test]
fn step_path_band_steps_back_along_min_without_diagonals() {
    let p = vec![(0, 0.5, 0.0, 1.0), (50, 0.5, 0.25, 0.75)];
    let s = YScale { min: 0.0, max: 1.0 };
    let out = step_path(&p, s, 0, 100, 100.0, 100.0);
    assert!(!out.band_d.contains('L'));
    assert_eq!(out.band_d, "M0,0H50V25H100V75H50V100H0Z");
}

#[test]
fn step_path_empty_input_is_empty() {
    let out = step_path(&[], YScale { min: 0.0, max: 1.0 }, 0, 100, 100.0, 50.0);
    assert_eq!(out.d, "");
    assert_eq!(out.band_d, "");
}

#[test]
fn nice_scale_picks_round_ticks_covering_the_data() {
    let n = nice_scale(YScale {
        min: 19.0,
        max: 31.0,
    });
    assert_eq!(n.ticks, vec![15.0, 20.0, 25.0, 30.0, 35.0]);
    assert_eq!((n.scale.min, n.scale.max), (15.0, 35.0));
    assert_eq!(n.step, 5.0);
}

#[test]
fn nice_scale_hugs_the_data_with_even_steps() {
    let n = nice_scale(YScale {
        min: 6.5,
        max: 12.0,
    });
    assert_eq!(n.ticks, vec![6.0, 8.0, 10.0, 12.0]);
}

#[test]
fn nice_scale_keeps_three_to_six_ticks_whatever_the_magnitude() {
    for (lo, hi) in [
        (0.0, 1.0),
        (0.3, 0.7),
        (-3.0, 8.0),
        (120.0, 4_700.0),
        (4.5, 5.5),
    ] {
        let n = nice_scale(YScale { min: lo, max: hi });
        assert!(
            (3..=6).contains(&n.ticks.len()),
            "{lo}..{hi}: {:?}",
            n.ticks
        );
        assert!(n.scale.min <= lo && n.scale.max >= hi, "{lo}..{hi}");
        assert!(n.ticks.windows(2).all(|w| w[0] < w[1]));
    }
}

#[test]
fn nice_scale_ticks_carry_no_float_noise() {
    let n = nice_scale(YScale {
        min: 0.05,
        max: 0.65,
    });
    assert_eq!(n.ticks, vec![0.0, 0.2, 0.4, 0.6, 0.8]);
}

#[test]
fn nice_scale_falls_back_on_a_degenerate_extent() {
    let n = nice_scale(YScale { min: 3.0, max: 3.0 });
    assert_eq!((n.scale.min, n.scale.max), (0.0, 1.0));
}

#[test]
fn step_decimals_follow_the_step() {
    assert_eq!(step_decimals(10.0), 0);
    assert_eq!(step_decimals(1.0), 0);
    assert_eq!(step_decimals(0.5), 1);
    assert_eq!(step_decimals(0.2), 1);
    assert_eq!(step_decimals(0.05), 2);
    assert_eq!(value_decimals(50.0), 0);
    assert_eq!(value_decimals(5.0), 1);
    assert_eq!(value_decimals(0.05), 2);
}

fn numbers(s: &str) -> Vec<f64> {
    s.replace(['M', 'L', 'C', 'Z'], " ")
        .split_whitespace()
        .map(|n| n.parse().unwrap())
        .collect()
}

#[test]
fn smooth_path_goes_through_every_point_and_on_to_the_window_end() {
    let p = vec![
        (20, 0.0, 0.0, 0.0),
        (50, 1.0, 1.0, 1.0),
        (70, 0.5, 0.5, 0.5),
    ];
    let s = YScale { min: 0.0, max: 1.0 };
    let out = smooth_path(&p, s, 0, 100, 100.0, 50.0);
    assert!(out.line.starts_with("M20.0 50.0"), "{}", out.line);
    assert!(out.line.contains("50.0 0.0"), "{}", out.line);
    assert!(out.line.contains("70.0 25.0"), "{}", out.line);
    assert!(out.line.ends_with("100.0 25.0"), "{}", out.line);
    assert!(!out.line.contains('H') && !out.line.contains('V'));
}

#[test]
fn smooth_path_never_leaves_the_data_range() {
    let p = vec![
        (0, 40.0, 40.0, 40.0),
        (10, 40.0, 40.0, 40.0),
        (20, 5.0, 5.0, 5.0),
        (30, 5.0, 5.0, 5.0),
        (40, 20.0, 20.0, 20.0),
    ];
    let s = YScale {
        min: 0.0,
        max: 40.0,
    };
    let out = smooth_path(&p, s, 0, 40, 40.0, 40.0);
    // y grows downwards: the value 40 is y 0, the value 5 is y 35. No control point leaves 0..=35.
    for y in numbers(&out.line).chunks(2).map(|c| c[1]) {
        assert!((-0.05..=35.05).contains(&y), "{y}: {}", out.line);
    }
}

#[test]
fn smooth_path_area_closes_down_to_the_bottom_edge() {
    let p = vec![(0, 1.0, 1.0, 1.0), (50, 0.0, 0.0, 0.0)];
    let s = YScale { min: 0.0, max: 1.0 };
    let out = smooth_path(&p, s, 0, 100, 100.0, 50.0);
    assert!(out.area.starts_with(&out.line));
    assert!(
        out.area.ends_with("L100.0 50.0 L0.0 50.0 Z"),
        "{}",
        out.area
    );
}

#[test]
fn smooth_path_band_is_one_closed_shape_without_stray_segments() {
    let p = vec![
        (0, 0.5, 0.25, 0.75),
        (30, 0.5, 0.5, 0.5),
        (60, 0.6, 0.4, 0.9),
    ];
    let s = YScale { min: 0.0, max: 1.0 };
    let out = smooth_path(&p, s, 0, 100, 100.0, 100.0);
    assert_eq!(out.band.matches('M').count(), 1, "{}", out.band);
    assert_eq!(out.band.matches('Z').count(), 1, "{}", out.band);
    assert!(!out.band.contains('H') && !out.band.contains('V'));
    assert!(out.band.starts_with("M0.0 25.0"), "{}", out.band);
    // It comes back along the lower edge to the start, and stays inside the plot.
    assert!(
        out.band.trim_end_matches(" Z").ends_with("0.0 75.0"),
        "{}",
        out.band
    );
    assert!(
        numbers(&out.band)
            .iter()
            .all(|v| (-0.05..=100.05).contains(v))
    );
}

#[test]
fn smooth_path_hides_the_band_when_min_equals_max() {
    let p = vec![(0, 0.5, 0.5, 0.5), (50, 0.7, 0.7, 0.7)];
    let out = smooth_path(&p, YScale { min: 0.0, max: 1.0 }, 0, 100, 100.0, 100.0);
    assert_eq!(out.band, "");
    assert!(!out.line.is_empty());
}

#[test]
fn smooth_path_handles_one_point_and_none() {
    let s = YScale { min: 0.0, max: 1.0 };
    let one = smooth_path(&[(50, 0.5, 0.5, 0.5)], s, 0, 100, 100.0, 50.0);
    assert!(one.line.starts_with("M50.0 25.0") && one.line.ends_with("100.0 25.0"));
    let none = smooth_path(&[], s, 0, 100, 100.0, 50.0);
    assert_eq!(
        (none.line.as_str(), none.area.as_str(), none.band.as_str()),
        ("", "", "")
    );
}

#[test]
fn smooth_path_survives_buckets_sharing_the_left_edge() {
    let p = vec![
        (-20, 0.0, 0.0, 0.0),
        (-10, 1.0, 1.0, 1.0),
        (50, 0.5, 0.5, 0.5),
    ];
    let out = smooth_path(&p, YScale { min: 0.0, max: 1.0 }, 0, 100, 100.0, 50.0);
    assert!(
        !out.line.contains("NaN") && !out.line.contains("inf"),
        "{}",
        out.line
    );
    assert!(out.line.starts_with("M0.0 0.0"), "{}", out.line);
}

#[test]
fn nearest_index_finds_the_closest_point_by_x() {
    let xs = [0.1, 0.4, 0.5, 0.9];
    assert_eq!(nearest_index(&xs, 0.0), Some(0));
    assert_eq!(nearest_index(&xs, 0.3), Some(1));
    assert_eq!(nearest_index(&xs, 0.46), Some(2));
    assert_eq!(nearest_index(&xs, 0.75), Some(3));
    assert_eq!(nearest_index(&xs, 5.0), Some(3));
    assert_eq!(nearest_index(&[], 0.5), None);
}

#[test]
fn nearest_index_prefers_the_later_point_on_a_tie_and_on_a_shared_edge() {
    assert_eq!(nearest_index(&[0.25, 0.75], 0.5), Some(1));
    assert_eq!(nearest_index(&[0.0, 0.0, 0.5], 0.0), Some(1));
}

#[test]
fn slot_index_maps_a_fraction_to_a_bar() {
    assert_eq!(slot_index(4, 0.0), Some(0));
    assert_eq!(slot_index(4, 0.26), Some(1));
    assert_eq!(slot_index(4, 1.0), Some(3));
    assert_eq!(slot_index(4, -1.0), Some(0));
    assert_eq!(slot_index(0, 0.5), None);
}

#[test]
fn the_tooltip_flips_left_in_the_right_part_of_the_chart() {
    assert!(!tooltip_flips(0.0));
    assert!(!tooltip_flips(0.6));
    assert!(tooltip_flips(0.61));
    assert!(tooltip_flips(1.0));
}

#[test]
fn daily_bars_lay_out_evenly_and_scale_to_max() {
    let days = vec![(0, 2.0), (86_400, 4.0), (172_800, 1.0)];
    let bars = daily_bars(&days, 300.0, 100.0);
    assert_eq!(bars.len(), 3);
    let w_each = 100.0;
    assert!((bars[0].x - 0.0).abs() < 1e-6);
    assert!((bars[1].x - 100.0).abs() < 1e-6);
    assert!((bars[1].h - 100.0).abs() < 1e-6);
    assert!((bars[0].h - 50.0).abs() < 1e-6);
    assert!((bars[0].w - w_each * 0.8).abs() < w_each * 0.21);
}

#[test]
fn daily_bars_empty_input_is_empty() {
    assert!(daily_bars(&[], 300.0, 100.0).is_empty());
}
