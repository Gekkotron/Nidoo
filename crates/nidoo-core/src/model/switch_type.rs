//! What a switch entity really drives: the override's `showAs`, read the same way everywhere.

use super::{Kind, UiOverride};

/// The type a switch entity is shown as. Light changes the effective kind; the others stay
/// [`Kind::Switch`] and only change the card, icon and wording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchType {
    Switch,
    Light,
    Door,
    Siren,
    Chime,
    Plug,
}

impl SwitchType {
    /// Every type, in the order the selector shows them.
    pub const ALL: [SwitchType; 6] = [
        SwitchType::Switch,
        SwitchType::Light,
        SwitchType::Door,
        SwitchType::Siren,
        SwitchType::Chime,
        SwitchType::Plug,
    ];

    /// The type of an entity, `None` unless its protocol kind is a switch. A value this hub does
    /// not know reads as a plain switch.
    pub fn of(protocol_kind: Kind, ui: &UiOverride) -> Option<SwitchType> {
        if protocol_kind != Kind::Switch {
            return None;
        }
        let value = ui.show_as.as_deref();
        Some(
            Self::ALL
                .into_iter()
                .find(|t| t.key().is_some() && t.key() == value)
                .unwrap_or(SwitchType::Switch),
        )
    }

    /// The `showAs` value; `None` for a plain switch.
    pub fn key(self) -> Option<&'static str> {
        match self {
            SwitchType::Switch => None,
            SwitchType::Light => Some("light"),
            SwitchType::Door => Some("door"),
            SwitchType::Siren => Some("siren"),
            SwitchType::Chime => Some("chime"),
            SwitchType::Plug => Some("plug"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SwitchType::Switch => "Switch",
            SwitchType::Light => "Light",
            SwitchType::Door => "Door opener",
            SwitchType::Siren => "Siren",
            SwitchType::Chime => "Chime",
            SwitchType::Plug => "Plug",
        }
    }

    /// [`Kind::Light`] for a switch of the Light type, else `protocol_kind`.
    pub fn effective_kind(protocol_kind: Kind, ui: &UiOverride) -> Kind {
        match Self::of(protocol_kind, ui) {
            Some(SwitchType::Light) => Kind::Light,
            _ => protocol_kind,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SwitchType;
    use crate::model::{Kind, SHOW_AS, UiOverride};

    fn ui(show_as: Option<&str>) -> UiOverride {
        UiOverride {
            show_as: show_as.map(str::to_owned),
            ..UiOverride::default()
        }
    }

    #[test]
    fn only_switches_have_a_type() {
        assert_eq!(SwitchType::of(Kind::Light, &ui(Some("door"))), None);
        assert_eq!(
            SwitchType::of(Kind::Switch, &ui(None)),
            Some(SwitchType::Switch)
        );
    }

    #[test]
    fn every_value_maps_to_its_type() {
        let types: Vec<_> = SHOW_AS
            .iter()
            .map(|v| SwitchType::of(Kind::Switch, &ui(Some(v))).unwrap())
            .collect();
        assert_eq!(
            types,
            [
                SwitchType::Light,
                SwitchType::Door,
                SwitchType::Siren,
                SwitchType::Chime,
                SwitchType::Plug
            ]
        );
        assert_eq!(
            SwitchType::of(Kind::Switch, &ui(Some("fan"))),
            Some(SwitchType::Switch)
        );
    }

    #[test]
    fn keys_and_labels() {
        assert_eq!(
            SwitchType::ALL.map(SwitchType::key),
            [
                None,
                Some("light"),
                Some("door"),
                Some("siren"),
                Some("chime"),
                Some("plug")
            ]
        );
        assert_eq!(
            SwitchType::ALL.map(SwitchType::label),
            ["Switch", "Light", "Door opener", "Siren", "Chime", "Plug"]
        );
    }

    #[test]
    fn effective_kind_follows_the_light_type_only() {
        assert_eq!(
            SwitchType::effective_kind(Kind::Switch, &ui(Some("light"))),
            Kind::Light
        );
        assert_eq!(
            SwitchType::effective_kind(Kind::Switch, &ui(Some("door"))),
            Kind::Switch
        );
        assert_eq!(
            SwitchType::effective_kind(Kind::Cover, &ui(Some("light"))),
            Kind::Cover
        );
    }
}
