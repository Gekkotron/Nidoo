//! The robot vacuum icon's maths: the rows it cleans its room along, where it is and which way it
//! faces, its trail, its glide home and its charging bolt. Positions are in the 24-unit drawing.

/// What the robot tile shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VacuumScene {
    /// Still at the start of the room: idle, paused, in error, unknown or unreachable.
    Idle,
    /// Driving along the rows, leaving a trail.
    Cleaning,
    /// Gliding into its dock.
    Returning,
    /// Charging in its dock.
    Docked,
}

/// The scene for a vacuum's reported state; an unreachable robot stays still.
pub fn vacuum_scene(state: Option<&str>, available: bool) -> VacuumScene {
    if !available {
        return VacuumScene::Idle;
    }
    match state.map(str::to_ascii_lowercase).as_deref() {
        Some("cleaning") => VacuumScene::Cleaning,
        Some("returning") => VacuumScene::Returning,
        Some("docked") => VacuumScene::Docked,
        _ => VacuumScene::Idle,
    }
}

/// Where the robot is and which way its bumper faces (0° is +x, 90° is down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RobotPose {
    pub x: f64,
    pub y: f64,
    pub heading_deg: f64,
}

impl RobotPose {
    /// The SVG transform that puts a robot drawn around the origin, facing +x, at this pose.
    pub fn transform(&self) -> String {
        format!(
            "translate({} {}) rotate({})",
            self.x, self.y, self.heading_deg
        )
    }
}

/// The corners of the rows the robot cleans, in order.
pub const ROWS: [(f64, f64); 8] = [
    (5.0, 5.5),
    (19.0, 5.5),
    (19.0, 10.0),
    (5.0, 10.0),
    (5.0, 14.5),
    (19.0, 14.5),
    (19.0, 19.0),
    (5.0, 19.0),
];
/// The rows' total length.
pub const PATH_LENGTH: f64 = 69.5;
/// The rows as an SVG path, for the trail.
pub const ROWS_D: &str = "M5 5.5H19V10H5V14.5H19V19H5";

/// The robot's pose at `t` (0..1, clamped) of the rows; it faces along the row it is on.
pub fn pose_at(t: f64) -> RobotPose {
    let mut left = t.clamp(0.0, 1.0) * PATH_LENGTH;
    let mut last = ROWS[0];
    let mut heading = 0.0;
    for w in ROWS.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        let len = (x1 - x0).abs() + (y1 - y0).abs();
        heading = (y1 - y0).atan2(x1 - x0).to_degrees();
        if left <= len {
            let f = if len > 0.0 { left / len } else { 0.0 };
            return RobotPose {
                x: x0 + (x1 - x0) * f,
                y: y0 + (y1 - y0) * f,
                heading_deg: heading,
            };
        }
        left -= len;
        last = (x1, y1);
    }
    RobotPose {
        x: last.0,
        y: last.1,
        heading_deg: heading,
    }
}

/// The trail's dash offset at `t`: with a `69.5 69.5` dash, it uncovers the rows driven so far.
pub fn trail_dash_offset(t: f64) -> f64 {
    PATH_LENGTH * (1.0 - t.clamp(0.0, 1.0))
}

/// One pass over all the rows, in ms, for a fan speed: a stronger suction cleans faster.
pub fn row_period_ms(fan_speed: Option<&str>) -> f64 {
    match fan_speed.map(str::to_ascii_lowercase).as_deref() {
        Some("quiet") => 11000.0,
        Some("turbo") => 6000.0,
        Some("max") => 4500.0,
        _ => 8000.0,
    }
}

/// Where the robot starts its glide home.
pub const RETURN_FROM: (f64, f64) = (6.5, 8.5);
/// Where it rests in its dock.
pub const DOCK_AT: (f64, f64) = (19.5, 19.0);
/// The way it faces while heading home and once docked.
pub const RETURN_HEADING: f64 = 39.0;
/// One glide home, then a hold in the dock, before the loop restarts.
pub const RETURN_PERIOD_MS: f64 = 3000.0;
/// The part of the cycle spent gliding; the rest is spent docked.
const RETURN_ARRIVE: f64 = 0.75;

/// The robot's pose at `phase` of its glide home: eased in and out, then held at the dock.
pub fn return_pose(phase: f64) -> RobotPose {
    let s = (phase / RETURN_ARRIVE).clamp(0.0, 1.0);
    let e = s * s * (3.0 - 2.0 * s);
    RobotPose {
        x: RETURN_FROM.0 + (DOCK_AT.0 - RETURN_FROM.0) * e,
        y: RETURN_FROM.1 + (DOCK_AT.1 - RETURN_FROM.1) * e,
        heading_deg: RETURN_HEADING,
    }
}

/// One full breath of the charging bolt.
pub const BOLT_PERIOD_MS: f64 = 3600.0;

/// The bolt's opacity at `phase`: 0.4, up to 0.9 halfway, back to 0.4.
pub fn bolt_opacity(phase: f64) -> f64 {
    0.4 + 0.5 * (1.0 - (2.0 * phase.rem_euclid(1.0) - 1.0).abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_ends_and_corners() {
        assert_eq!(
            pose_at(0.0),
            RobotPose {
                x: 5.0,
                y: 5.5,
                heading_deg: 0.0
            }
        );
        let c = pose_at(14.0 / PATH_LENGTH);
        assert!((c.x - 19.0).abs() < 1e-9 && (c.y - 5.5).abs() < 1e-9);
        assert_eq!(pose_at(16.0 / PATH_LENGTH).heading_deg, 90.0);
        assert_eq!(pose_at(25.0 / PATH_LENGTH).heading_deg, 180.0);
        let end = pose_at(1.0);
        assert!((end.x - 5.0).abs() < 1e-9 && (end.y - 19.0).abs() < 1e-9);
    }

    #[test]
    fn path_clamps() {
        assert_eq!(pose_at(-1.0), pose_at(0.0));
        assert_eq!(pose_at(2.0), pose_at(1.0));
    }

    #[test]
    fn rows_add_up_to_the_length() {
        let len: f64 = ROWS
            .windows(2)
            .map(|w| (w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs())
            .sum();
        assert!((len - PATH_LENGTH).abs() < 1e-9);
    }

    #[test]
    fn trail_grows_with_the_robot() {
        assert_eq!(trail_dash_offset(0.0), PATH_LENGTH);
        assert_eq!(trail_dash_offset(1.0), 0.0);
    }

    #[test]
    fn row_periods_follow_the_fan() {
        assert_eq!(row_period_ms(Some("quiet")), 11000.0);
        assert_eq!(row_period_ms(Some("turbo")), 6000.0);
        assert_eq!(row_period_ms(Some("max")), 4500.0);
        assert_eq!(row_period_ms(Some("standard")), 8000.0);
        assert_eq!(row_period_ms(None), 8000.0);
    }

    #[test]
    fn periods_ignore_case() {
        assert_eq!(row_period_ms(Some("Turbo")), 6000.0);
    }

    #[test]
    fn scenes_follow_the_state() {
        assert_eq!(vacuum_scene(Some("cleaning"), true), VacuumScene::Cleaning);
        assert_eq!(
            vacuum_scene(Some("Returning"), true),
            VacuumScene::Returning
        );
        assert_eq!(vacuum_scene(Some("docked"), true), VacuumScene::Docked);
        for s in ["idle", "paused", "error", "unknown"] {
            assert_eq!(vacuum_scene(Some(s), true), VacuumScene::Idle);
        }
        assert_eq!(vacuum_scene(Some("cleaning"), false), VacuumScene::Idle);
    }

    #[test]
    fn return_glides_then_holds_at_the_dock() {
        assert_eq!(
            return_pose(0.0),
            RobotPose {
                x: 6.5,
                y: 8.5,
                heading_deg: 39.0
            }
        );
        assert_eq!(
            return_pose(0.75),
            RobotPose {
                x: 19.5,
                y: 19.0,
                heading_deg: 39.0
            }
        );
        assert_eq!(return_pose(0.9), return_pose(0.75));
    }

    #[test]
    fn bolt_breathes() {
        assert!((bolt_opacity(0.0) - 0.4).abs() < 1e-9);
        assert!((bolt_opacity(0.5) - 0.9).abs() < 1e-9);
        assert!((bolt_opacity(1.0) - 0.4).abs() < 1e-9);
    }

    #[test]
    fn pose_renders_as_a_transform() {
        let p = RobotPose {
            x: 19.5,
            y: 19.0,
            heading_deg: 39.0,
        };
        assert_eq!(p.transform(), "translate(19.5 19) rotate(39)");
    }
}
