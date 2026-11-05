//! The menu-bar glyphs, drawn at startup as template images (macOS keeps only their
//! alpha and colors them for a light or dark menu bar): a microphone, with sound waves
//! while a voice window is open, crossed out when Listening is off, faded when the hub
//! cannot be reached.

use nidoo_satellite::IconState;

/// 22 pt at 2×.
const SIZE: u32 = 44;
/// Samples per pixel side, for smooth edges.
const SUPER: u32 = 4;

fn capsule(x: f32, y: f32) -> bool {
    // A vertical segment (22, 12)–(22, 20) widened by 6.
    let cy = y.clamp(12.0, 20.0);
    (x - 22.0).hypot(y - cy) <= 6.0
}

fn cradle(x: f32, y: f32) -> bool {
    let r = (x - 22.0).hypot(y - 20.0);
    let ring = (10.5..=13.5).contains(&r) && y >= 20.0;
    let arms = (10.5..=13.5).contains(&(x - 22.0).abs()) && (16.0..20.0).contains(&y);
    let stem = (20.5..=23.5).contains(&x) && (33.0..=38.0).contains(&y);
    let base = (15.0..=29.0).contains(&x) && (37.0..=40.0).contains(&y);
    ring || arms || stem || base
}

fn waves(x: f32, y: f32) -> bool {
    let (dx, dy) = (x - 22.0, y - 16.0);
    let r = dx.hypot(dy);
    let side = dx.abs() > 2.0 * dy.abs();
    side && (16.5..=19.0).contains(&r)
}

/// Distance from the slash's centre line, (8, 6) to (36, 40).
fn slash_distance(x: f32, y: f32) -> f32 {
    let (ax, ay, bx, by) = (8.0f32, 6.0f32, 36.0f32, 40.0f32);
    let (vx, vy) = (bx - ax, by - ay);
    let t = (((x - ax) * vx + (y - ay) * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
    (x - (ax + t * vx)).hypot(y - (ay + t * vy))
}

fn covered(state: IconState, x: f32, y: f32) -> bool {
    let mic = capsule(x, y) || cradle(x, y);
    match state {
        IconState::Ready | IconState::Gray => mic,
        IconState::Listening => mic || waves(x, y),
        IconState::MicOff => {
            let d = slash_distance(x, y);
            d <= 1.75 || (mic && d > 4.5)
        }
    }
}

/// RGBA pixels (black, alpha = coverage) and the side length.
pub fn rgba(state: IconState) -> (Vec<u8>, u32) {
    let fade = if state == IconState::Gray { 0.35 } else { 1.0 };
    let mut px = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for py in 0..SIZE {
        for pxl in 0..SIZE {
            let mut hits = 0;
            for sy in 0..SUPER {
                for sx in 0..SUPER {
                    let x = pxl as f32 + (sx as f32 + 0.5) / SUPER as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / SUPER as f32;
                    hits += u32::from(covered(state, x, y));
                }
            }
            let a = hits as f32 / (SUPER * SUPER) as f32 * fade;
            px.extend_from_slice(&[0, 0, 0, (a * 255.0).round() as u8]);
        }
    }
    (px, SIZE)
}

pub fn icon(state: IconState) -> tray_icon::Icon {
    let (px, side) = rgba(state);
    tray_icon::Icon::from_rgba(px, side, side).expect("a 44×44 RGBA icon")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ink(state: IconState) -> u32 {
        rgba(state).0.chunks(4).map(|p| u32::from(p[3])).sum()
    }

    #[test]
    fn the_four_states_look_different() {
        let ready = ink(IconState::Ready);
        assert!(ready > 0);
        assert!(ink(IconState::Listening) > ready, "waves added");
        assert!(ink(IconState::Gray) < ready / 2, "faded");
        assert_ne!(rgba(IconState::MicOff).0, rgba(IconState::Ready).0);
    }
}
