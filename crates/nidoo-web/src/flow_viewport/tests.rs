use super::*;
use crate::flow_layout::{NODE_H, NODE_W};

#[test]
fn fit_beside_leaves_the_panel_free() {
    let c = Rect {
        x: 0.0,
        y: 0.0,
        w: 1200.0,
        h: 200.0,
    };
    let v = fit_beside(c, 1440.0, 800.0, 384.0);
    let right = v
        .to_screen(Pt {
            x: c.x + c.w,
            y: 0.0,
        })
        .x;
    let left = v.to_screen(Pt { x: c.x, y: 0.0 }).x;
    assert!(right <= 1440.0 - 384.0 + 0.01, "right edge at {right}");
    assert!(left >= 0.0, "left edge at {left}");
    assert_eq!(fit_beside(c, 1440.0, 800.0, 0.0), fit(c, 1440.0, 800.0));
}

#[test]
fn free_area_is_left_of_the_panel() {
    let view = View {
        x: -100.0,
        y: -50.0,
        zoom: 2.0,
    };
    let a = free_area(&view, 1000.0, 600.0, 384.0);
    assert_eq!(
        a,
        Rect {
            x: 50.0,
            y: 25.0,
            w: 616.0 / 2.0,
            h: 300.0
        }
    );
    // Too narrow for a card beside the panel: the whole screen.
    assert_eq!(free_area(&view, 500.0, 600.0, 384.0).w, 250.0);
}

#[test]
fn add_spot_centres_left_of_the_panel() {
    let view = View {
        x: 0.0,
        y: 0.0,
        zoom: 1.0,
    };
    // 1000×600 screen, 384 px panel: the free area is 616 wide.
    let p = add_spot(&view, 1000.0, 600.0, 384.0);
    assert_eq!(
        p,
        Pt {
            x: (616.0 - NODE_W) / 2.0,
            y: (600.0 - NODE_H) / 2.0
        }
    );
}

#[test]
fn add_spot_follows_pan_and_zoom() {
    let view = View {
        x: -100.0,
        y: -50.0,
        zoom: 2.0,
    };
    let screen = Pt {
        x: (616.0 - NODE_W * 2.0) / 2.0,
        y: (600.0 - NODE_H * 2.0) / 2.0,
    };
    assert_eq!(add_spot(&view, 1000.0, 600.0, 384.0), view.to_world(screen));
}

#[test]
fn add_spot_ignores_the_panel_when_too_narrow() {
    let view = View {
        x: 0.0,
        y: 0.0,
        zoom: 1.0,
    };
    assert_eq!(
        add_spot(&view, 500.0, 400.0, 384.0).x,
        (500.0 - NODE_W) / 2.0
    );
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.01
}

fn card(x: f64, y: f64) -> Rect {
    Rect {
        x,
        y,
        w: NODE_W,
        h: NODE_H,
    }
}

#[test]
fn zoom_at_keeps_point_fixed() {
    let v = View {
        x: 30.0,
        y: -20.0,
        zoom: 0.8,
    };
    let s = Pt { x: 400.0, y: 250.0 };
    let before = v.to_world(s);
    let after = v.zoom_at(s, 1.2).to_world(s);
    assert!(close(before.x, after.x) && close(before.y, after.y));
}

#[test]
fn zoom_clamps() {
    let v = View {
        x: 0.0,
        y: 0.0,
        zoom: 1.0,
    };
    let s = Pt { x: 10.0, y: 10.0 };
    assert_eq!(v.zoom_at(s, 100.0).zoom, ZOOM_MAX);
    assert_eq!(v.zoom_at(s, 0.001).zoom, ZOOM_MIN);
}

#[test]
fn fit_centres_content() {
    let c = Rect {
        x: 100.0,
        y: 50.0,
        w: 1000.0,
        h: 600.0,
    };
    let v = fit(c, 1400.0, 800.0);
    assert!((FIT_MIN..=FIT_MAX).contains(&v.zoom));
    let mid = v.to_screen(Pt {
        x: c.x + c.w / 2.0,
        y: c.y + c.h / 2.0,
    });
    assert!(close(mid.x, 700.0) && close(mid.y, 400.0));
}

#[test]
fn fit_single_node_caps_zoom() {
    let v = fit(card(0.0, 0.0), 1400.0, 800.0);
    assert_eq!(v.zoom, FIT_MAX);
}

#[test]
fn fit_far_positions() {
    let c = card(5000.0, 3000.0);
    let v = fit(c, 1400.0, 800.0);
    for p in [
        Pt { x: c.x, y: c.y },
        Pt {
            x: c.x + c.w,
            y: c.y + c.h,
        },
    ] {
        let s = v.to_screen(p);
        assert!((0.0..=1400.0).contains(&s.x) && (0.0..=800.0).contains(&s.y));
    }
}

#[test]
fn fit_frames_a_wide_flow_and_clamps_huge_ones() {
    // About the size of the 23-node LED flow: it must fit a 1440 × 818 canvas whole.
    let wide = Rect {
        x: 40.0,
        y: 40.0,
        w: 3850.0,
        h: 500.0,
    };
    let v = fit(wide, 1440.0, 818.0);
    for p in [
        Pt {
            x: wide.x,
            y: wide.y,
        },
        Pt {
            x: wide.x + wide.w,
            y: wide.y + wide.h,
        },
    ] {
        let s = v.to_screen(p);
        assert!(
            (0.0..=1440.0).contains(&s.x) && (0.0..=818.0).contains(&s.y),
            "{s:?}"
        );
    }
    let huge = Rect {
        x: 0.0,
        y: 0.0,
        w: 50_000.0,
        h: 500.0,
    };
    assert_eq!(fit(huge, 1440.0, 818.0).zoom, FIT_MIN);
}

#[test]
fn minimap_roundtrip() {
    let content = Rect {
        x: 0.0,
        y: 0.0,
        w: 2000.0,
        h: 900.0,
    };
    let view = View {
        x: 0.0,
        y: 0.0,
        zoom: 1.0,
    };
    let vr = visible_world(&view, 1400.0, 800.0);
    let m = minimap(content, vr, 180.0, 120.0);
    let c = Pt {
        x: 1000.0,
        y: 450.0,
    };
    let p = Pt {
        x: (c.x - m.ox) * m.scale,
        y: (c.y - m.oy) * m.scale,
    };
    let back = mini_to_world(&m, p);
    assert!(close(back.x, c.x) && close(back.y, c.y));
    // The union fits the box.
    assert!((2000.0 - m.ox) * m.scale <= 180.01);
}

#[test]
fn centre_on_moves_point_to_middle() {
    let v = View {
        x: 5.0,
        y: 5.0,
        zoom: 0.5,
    };
    let w = Pt { x: 700.0, y: 300.0 };
    let s = centre_on(&v, w, 1000.0, 600.0).to_screen(w);
    assert!(close(s.x, 500.0) && close(s.y, 300.0));
}

#[test]
fn css_transform_format() {
    let v = View {
        x: 10.0,
        y: -5.5,
        zoom: 1.5,
    };
    assert_eq!(v.css_transform(), "translate(10px,-5.5px) scale(1.5)");
}
