//! Which tab shows an entity, as the app's `Placement.kt` and `SensorGroup.kt` decide it: Home
//! leaves out what Energy or Security already shows, unless the entity is pinned.

use super::{Kind, UiOverride};

/// The security class of a binary sensor. The declaration order is the summary order: the
/// danger groups first, then intrusion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SensorGroup {
    Smoke,
    Co,
    Gas,
    Leak,
    Sos,
    Opening,
    Motion,
    Vibration,
    Tamper,
}

impl SensorGroup {
    pub const ALL: [SensorGroup; 9] = [
        SensorGroup::Smoke,
        SensorGroup::Co,
        SensorGroup::Gas,
        SensorGroup::Leak,
        SensorGroup::Sos,
        SensorGroup::Opening,
        SensorGroup::Motion,
        SensorGroup::Vibration,
        SensorGroup::Tamper,
    ];

    /// `None`: not a security class, so not on the Security tab.
    pub fn of(device_class: Option<&str>) -> Option<SensorGroup> {
        Some(match device_class? {
            "door" | "window" | "garage_door" | "opening" => SensorGroup::Opening,
            "motion" | "occupancy" | "presence" => SensorGroup::Motion,
            "smoke" => SensorGroup::Smoke,
            "carbon_monoxide" => SensorGroup::Co,
            "gas" => SensorGroup::Gas,
            "moisture" => SensorGroup::Leak,
            "safety" => SensorGroup::Sos,
            "vibration" => SensorGroup::Vibration,
            "tamper" => SensorGroup::Tamper,
            _ => return None,
        })
    }

    /// Rose accent: a danger to people or the house. The others are intrusion (amber).
    pub fn danger(self) -> bool {
        matches!(
            self,
            SensorGroup::Smoke
                | SensorGroup::Co
                | SensorGroup::Gas
                | SensorGroup::Leak
                | SensorGroup::Sos
        )
    }

    /// The word of the summary line ("1 smoke · 2 open · 1 motion").
    pub fn word(self) -> &'static str {
        match self {
            SensorGroup::Smoke => "smoke",
            SensorGroup::Co => "CO",
            SensorGroup::Gas => "gas",
            SensorGroup::Leak => "leak",
            SensorGroup::Sos => "SOS",
            SensorGroup::Opening => "open",
            SensorGroup::Motion => "motion",
            SensorGroup::Vibration => "vibration",
            SensorGroup::Tamper => "tamper",
        }
    }
}

/// The Security tab's sensor sections, in screen order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SensorSection {
    Openings,
    Motion,
    Fire,
    Water,
    Other,
}

impl SensorSection {
    pub const ALL: [SensorSection; 5] = [
        SensorSection::Openings,
        SensorSection::Motion,
        SensorSection::Fire,
        SensorSection::Water,
        SensorSection::Other,
    ];

    pub fn of(group: SensorGroup) -> SensorSection {
        match group {
            SensorGroup::Smoke | SensorGroup::Co | SensorGroup::Gas => SensorSection::Fire,
            SensorGroup::Leak => SensorSection::Water,
            SensorGroup::Opening => SensorSection::Openings,
            SensorGroup::Motion => SensorSection::Motion,
            SensorGroup::Sos | SensorGroup::Vibration | SensorGroup::Tamper => SensorSection::Other,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            SensorSection::Openings => "Openings",
            SensorSection::Motion => "Motion",
            SensorSection::Fire => "Fire",
            SensorSection::Water => "Water",
            SensorSection::Other => "Other",
        }
    }
}

/// Which tabs show an entity.
pub struct Placement;

impl Placement {
    /// Shown on Security by default: alarm panels and binary sensors of a security class.
    pub fn security_kind(kind: Kind, device_class: Option<&str>) -> bool {
        kind == Kind::AlarmPanel
            || (kind == Kind::BinarySensor && SensorGroup::of(device_class).is_some())
    }

    /// A kind the user can opt into Security as an action.
    pub fn action_kind(kind: Kind) -> bool {
        matches!(kind, Kind::Switch | Kind::Button)
    }

    pub fn on_security(kind: Kind, device_class: Option<&str>, ui: &UiOverride) -> bool {
        (!ui.security_hidden && Self::security_kind(kind, device_class))
            || (ui.security_action && Self::action_kind(kind))
    }

    /// Energy (it has a role) or Security already shows it.
    pub fn elsewhere(kind: Kind, device_class: Option<&str>, ui: &UiOverride) -> bool {
        ui.energy_role.is_some() || Self::on_security(kind, device_class, ui)
    }

    pub fn on_home(kind: Kind, device_class: Option<&str>, ui: &UiOverride) -> bool {
        !ui.hidden && (ui.home_pinned || !Self::elsewhere(kind, device_class, ui))
    }
}

#[cfg(test)]
mod tests {
    use super::{Placement, SensorGroup, SensorSection};
    use crate::model::{Kind, UiOverride};

    #[test]
    fn groups_by_device_class() {
        for class in ["door", "window", "garage_door", "opening"] {
            assert_eq!(SensorGroup::of(Some(class)), Some(SensorGroup::Opening));
        }
        for class in ["motion", "occupancy", "presence"] {
            assert_eq!(SensorGroup::of(Some(class)), Some(SensorGroup::Motion));
        }
        assert_eq!(SensorGroup::of(Some("smoke")), Some(SensorGroup::Smoke));
        assert_eq!(
            SensorGroup::of(Some("carbon_monoxide")),
            Some(SensorGroup::Co)
        );
        assert_eq!(SensorGroup::of(Some("gas")), Some(SensorGroup::Gas));
        assert_eq!(SensorGroup::of(Some("moisture")), Some(SensorGroup::Leak));
        assert_eq!(SensorGroup::of(Some("safety")), Some(SensorGroup::Sos));
        assert_eq!(
            SensorGroup::of(Some("vibration")),
            Some(SensorGroup::Vibration)
        );
        assert_eq!(SensorGroup::of(Some("tamper")), Some(SensorGroup::Tamper));
        assert_eq!(SensorGroup::of(None), None);
        assert_eq!(SensorGroup::of(Some("battery")), None);
        assert_eq!(SensorGroup::of(Some("problem")), None);
    }

    #[test]
    fn danger_subset() {
        let danger: Vec<_> = SensorGroup::ALL.iter().filter(|g| g.danger()).collect();
        assert_eq!(
            danger,
            [
                &SensorGroup::Smoke,
                &SensorGroup::Co,
                &SensorGroup::Gas,
                &SensorGroup::Leak,
                &SensorGroup::Sos
            ]
        );
        assert!(!SensorGroup::Opening.danger());
        assert!(!SensorGroup::Tamper.danger());
    }

    #[test]
    fn sections() {
        assert_eq!(SensorSection::of(SensorGroup::Smoke), SensorSection::Fire);
        assert_eq!(SensorSection::of(SensorGroup::Co), SensorSection::Fire);
        assert_eq!(SensorSection::of(SensorGroup::Gas), SensorSection::Fire);
        assert_eq!(SensorSection::of(SensorGroup::Leak), SensorSection::Water);
        assert_eq!(
            SensorSection::of(SensorGroup::Opening),
            SensorSection::Openings
        );
        assert_eq!(
            SensorSection::of(SensorGroup::Motion),
            SensorSection::Motion
        );
        assert_eq!(SensorSection::of(SensorGroup::Sos), SensorSection::Other);
        assert_eq!(
            SensorSection::of(SensorGroup::Vibration),
            SensorSection::Other
        );
        assert_eq!(SensorSection::of(SensorGroup::Tamper), SensorSection::Other);
        assert_eq!(SensorSection::ALL[0].title(), "Openings");
        assert_eq!(SensorSection::ALL[4].title(), "Other");
    }

    #[test]
    fn security_kind_and_actions() {
        assert!(Placement::security_kind(Kind::AlarmPanel, None));
        assert!(Placement::security_kind(Kind::BinarySensor, Some("door")));
        assert!(!Placement::security_kind(
            Kind::BinarySensor,
            Some("battery")
        ));
        assert!(!Placement::security_kind(Kind::BinarySensor, None));
        assert!(!Placement::security_kind(Kind::Switch, None));
        assert!(Placement::action_kind(Kind::Switch));
        assert!(Placement::action_kind(Kind::Button));
        assert!(!Placement::action_kind(Kind::Light));
    }

    #[test]
    fn on_security_rules() {
        let plain = UiOverride::default();
        assert!(Placement::on_security(
            Kind::BinarySensor,
            Some("door"),
            &plain
        ));
        let hidden = UiOverride {
            security_hidden: true,
            ..Default::default()
        };
        assert!(!Placement::on_security(
            Kind::BinarySensor,
            Some("door"),
            &hidden
        ));
        let action = UiOverride {
            security_action: true,
            ..Default::default()
        };
        assert!(Placement::on_security(Kind::Switch, None, &action));
        assert!(Placement::on_security(Kind::Button, None, &action));
        assert!(!Placement::on_security(Kind::Light, None, &action));
        assert!(!Placement::on_security(Kind::Switch, None, &plain));
    }

    #[test]
    fn pinned_stays_home() {
        let grid = UiOverride {
            energy_role: Some("grid".into()),
            ..Default::default()
        };
        assert!(Placement::elsewhere(Kind::Sensor, Some("power"), &grid));
        assert!(!Placement::on_home(Kind::Sensor, Some("power"), &grid));
        let pinned = UiOverride {
            home_pinned: true,
            ..grid.clone()
        };
        assert!(Placement::on_home(Kind::Sensor, Some("power"), &pinned));
        let hidden_pinned = UiOverride {
            hidden: true,
            ..pinned
        };
        assert!(!Placement::on_home(
            Kind::Sensor,
            Some("power"),
            &hidden_pinned
        ));
        // A door sensor leaves Home for Security; hiding it from Security brings it back.
        assert!(!Placement::on_home(
            Kind::BinarySensor,
            Some("door"),
            &UiOverride::default()
        ));
        let off_security = UiOverride {
            security_hidden: true,
            ..Default::default()
        };
        assert!(Placement::on_home(
            Kind::BinarySensor,
            Some("door"),
            &off_security
        ));
        assert!(Placement::on_home(
            Kind::Light,
            None,
            &UiOverride::default()
        ));
    }
}
