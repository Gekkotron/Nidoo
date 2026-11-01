//! Which members a group command reaches, and what each one is sent (Appendix A.5). Pure:
//! the hub does the publishing and the waiting.

use nidoo_api::{EntityView, GroupSpec};
use nidoo_core::model::{Availability, Command};

/// The ids of the members that are of the group's kind (honouring `showAs`) and not offline.
pub fn targets(spec: &GroupSpec, members: &[EntityView]) -> Vec<String> {
    members
        .iter()
        .filter(|m| m.kind == spec.kind && m.state.availability != Availability::Offline)
        .map(|m| m.id.clone())
        .collect()
}

/// The command to send `member`, or `None` when it cannot take `cmd`. The discovery config
/// offers only what every member supports, so `None` is a member that drifted since.
pub fn translate(_spec: &GroupSpec, cmd: &Command, member: &EntityView) -> Option<Command> {
    let caps = &member.capabilities;
    let supported = match cmd {
        Command::TurnOn {
            brightness,
            color,
            effect,
        } => {
            (brightness.is_none() || caps.brightness)
                && (color.is_none()
                    || ["hs", "xy", "rgb", "rgbw", "rgbww"]
                        .iter()
                        .any(|mode| caps.color_modes.contains(*mode)))
                && effect.as_ref().is_none_or(|e| caps.effects.contains(e))
        }
        Command::SetPosition { .. } => caps.position,
        Command::SetHvacMode { mode } => caps.hvac_modes.contains(mode),
        Command::SetFanMode { mode } => caps.fan_modes.contains(mode),
        Command::SetFanSpeed { speed } => caps.fan_speeds.contains(speed),
        Command::SelectOption { option } => caps.options.contains(option),
        Command::VacuumAction { action } => caps.vacuum_actions.contains(action),
        _ => true,
    };
    supported.then(|| cmd.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};

    fn member(id: &str, kind: Kind, caps: Capabilities, availability: Availability) -> EntityView {
        EntityView {
            id: id.to_owned(),
            device_id: None,
            name: id.to_owned(),
            device_name: None,
            label: None,
            kind,
            component: kind.component().to_string(),
            protocol_kind: kind,
            device_class: None,
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: String::new(),
            part_of: None,
            capabilities: caps,
            ui: UiOverride::default(),
            state: EntityState {
                availability,
                ..Default::default()
            },
        }
    }

    fn spec(kind: Kind) -> GroupSpec {
        GroupSpec {
            kind,
            name: "g".to_owned(),
            members: Vec::new(),
            hide_members: true,
        }
    }

    fn plain(id: &str, kind: Kind) -> EntityView {
        member(id, kind, Capabilities::default(), Availability::Online)
    }

    #[test]
    fn an_offline_member_is_not_targeted() {
        let offline = member(
            "b",
            Kind::Switch,
            Capabilities::default(),
            Availability::Offline,
        );
        let members = [plain("a", Kind::Switch), offline];
        assert_eq!(targets(&spec(Kind::Switch), &members), ["a"]);
    }

    #[test]
    fn a_switch_shown_as_light_is_a_light_group_member() {
        let mut shown = plain("a", Kind::Light);
        shown.protocol_kind = Kind::Switch;
        assert_eq!(targets(&spec(Kind::Light), &[shown]), ["a"]);
    }

    #[test]
    fn a_member_whose_show_as_was_cleared_is_dropped() {
        let drifted = plain("a", Kind::Switch);
        assert!(targets(&spec(Kind::Light), &[drifted]).is_empty());
    }

    #[test]
    fn brightness_needs_a_dimmable_member() {
        let cmd = Command::TurnOn {
            brightness: Some(80),
            color: None,
            effect: None,
        };
        let member = plain("a", Kind::Light);
        assert_eq!(translate(&spec(Kind::Light), &cmd, &member), None);
        let dimmable = member_with(Kind::Light, |c| c.brightness = true);
        assert_eq!(translate(&spec(Kind::Light), &cmd, &dimmable), Some(cmd));
    }

    fn member_with(kind: Kind, set: impl FnOnce(&mut Capabilities)) -> EntityView {
        let mut caps = Capabilities::default();
        set(&mut caps);
        member("a", kind, caps, Availability::Online)
    }

    #[test]
    fn position_needs_a_position_capable_member() {
        let cmd = Command::SetPosition { position: 40 };
        let member = plain("a", Kind::Cover);
        assert_eq!(translate(&spec(Kind::Cover), &cmd, &member), None);
        let capable = member_with(Kind::Cover, |c| c.position = true);
        assert_eq!(translate(&spec(Kind::Cover), &cmd, &capable), Some(cmd));
    }

    #[test]
    fn hvac_mode_needs_the_mode() {
        let cmd = Command::SetHvacMode {
            mode: "heat".to_owned(),
        };
        let member = member_with(Kind::Climate, |c| c.hvac_modes = vec!["off".to_owned()]);
        assert_eq!(translate(&spec(Kind::Climate), &cmd, &member), None);
        let capable = member_with(Kind::Climate, |c| {
            c.hvac_modes = vec!["off".to_owned(), "heat".to_owned()];
        });
        assert_eq!(translate(&spec(Kind::Climate), &cmd, &capable), Some(cmd));
    }

    #[test]
    fn plain_commands_pass_through() {
        let member = plain("a", Kind::Switch);
        assert_eq!(
            translate(&spec(Kind::Switch), &Command::TurnOff, &member),
            Some(Command::TurnOff)
        );
    }
}
