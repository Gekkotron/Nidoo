//! MQTT topics of the alarm.
//!
//! Layout under the configured prefix: `<p>/settings` and `<p>/state` (retained documents),
//! `<p>/panel`, `<p>/panel/attributes` (published by the hub for the panel entity) and
//! `<p>/panel/set` (panel commands). No topic ends in `/config`, so none is read as a Home Assistant
//! discovery message.

/// The topics the hub subscribes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlarmTopic {
    Settings,
    Runtime,
    PanelCommand,
}

fn base(p: &str) -> &str {
    p.trim_end_matches('/')
}

/// Topic of the settings document.
pub fn settings_topic(p: &str) -> String {
    format!("{}/settings", base(p))
}

/// Topic of the retained runtime state.
pub fn runtime_topic(p: &str) -> String {
    format!("{}/state", base(p))
}

/// Topic of the panel entity state.
pub fn panel_state_topic(p: &str) -> String {
    format!("{}/panel", base(p))
}

/// Topic panel commands are sent to.
pub fn panel_command_topic(p: &str) -> String {
    format!("{}/panel/set", base(p))
}

/// Topic of the panel entity attributes.
pub fn panel_attributes_topic(p: &str) -> String {
    format!("{}/panel/attributes", base(p))
}

/// The subscription filters: settings, runtime and panel command. Never `<p>/#`, which would also
/// match the topics the hub publishes itself.
pub fn filters(p: &str) -> [String; 3] {
    [settings_topic(p), runtime_topic(p), panel_command_topic(p)]
}

/// Which alarm topic `topic` is, if any. The panel state and attributes topics are published by
/// the hub and ingested by the registry, so they parse to `None`.
pub fn parse(p: &str, topic: &str) -> Option<AlarmTopic> {
    if topic == settings_topic(p) {
        Some(AlarmTopic::Settings)
    } else if topic == runtime_topic(p) {
        Some(AlarmTopic::Runtime)
    } else if topic == panel_command_topic(p) {
        Some(AlarmTopic::PanelCommand)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topics_layout() {
        let p = "oikos/alarm/";
        assert_eq!(settings_topic(p), "oikos/alarm/settings");
        assert_eq!(runtime_topic(p), "oikos/alarm/state");
        assert_eq!(panel_state_topic(p), "oikos/alarm/panel");
        assert_eq!(panel_command_topic(p), "oikos/alarm/panel/set");
        assert_eq!(panel_attributes_topic(p), "oikos/alarm/panel/attributes");
        let [s, r, c] = filters(p);
        assert_eq!(parse(p, &s), Some(AlarmTopic::Settings));
        assert_eq!(parse(p, &r), Some(AlarmTopic::Runtime));
        assert_eq!(parse(p, &c), Some(AlarmTopic::PanelCommand));
        assert_eq!(parse(p, "oikos/alarm/panel"), None);
        assert_eq!(parse(p, "oikos/alarm/panel/attributes"), None);
    }
}
