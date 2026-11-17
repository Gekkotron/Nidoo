//! Which covers are window coverings (shutters, blinds...): the ones "open the shutters" and
//! the Covers category move. A garage door, gate or entrance door never is.

use super::UiOverride;

/// `device_class` values of a window covering. Any other class (garage, gate, door, damper)
/// is not one.
pub const WINDOW_COVERINGS: [&str; 6] =
    ["shutter", "blind", "curtain", "shade", "awning", "window"];

/// The `showAs` mark that makes a cover with no `device_class` count as a shutter.
pub const SHUTTER_MARK: &str = "shutter";

/// Whether a cover is a window covering: its `device_class` says so, or, with no class, it
/// carries the [`SHUTTER_MARK`]. An unclassed cover without the mark could be a garage door,
/// so it is left out. A class always wins over the mark.
pub fn is_window_covering(device_class: Option<&str>, ui: &UiOverride) -> bool {
    match device_class {
        Some(class) => WINDOW_COVERINGS.contains(&class),
        None => ui.show_as.as_deref() == Some(SHUTTER_MARK),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked(show_as: Option<&str>) -> UiOverride {
        UiOverride {
            show_as: show_as.map(str::to_owned),
            ..UiOverride::default()
        }
    }

    #[test]
    fn window_classes_are_window_coverings() {
        for class in WINDOW_COVERINGS {
            assert!(is_window_covering(Some(class), &marked(None)), "{class}");
        }
    }

    #[test]
    fn other_classes_are_not_even_when_marked() {
        for class in ["garage", "gate", "door", "damper"] {
            assert!(
                !is_window_covering(Some(class), &marked(Some(SHUTTER_MARK))),
                "{class}"
            );
        }
    }

    #[test]
    fn unclassed_cover_needs_the_mark() {
        assert!(!is_window_covering(None, &marked(None)));
        assert!(!is_window_covering(None, &marked(Some("plug"))));
        assert!(is_window_covering(None, &marked(Some(SHUTTER_MARK))));
    }
}
