//! MQTT topics of the climate documents.
//!
//! Layout under the configured prefix: `<p>/house` and `<p>/zone/<zone>/<doc>`. No topic ends in
//! `/config`, so none of them is ever read as a Home Assistant discovery message.

/// The retained documents kept per zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoneDoc {
    Settings,
    Proposal,
    Override,
    Decision,
    Eco,
}

impl ZoneDoc {
    /// The last topic segment of this document.
    pub fn segment(self) -> &'static str {
        match self {
            ZoneDoc::Settings => "settings",
            ZoneDoc::Proposal => "proposal",
            ZoneDoc::Override => "override",
            ZoneDoc::Decision => "decision",
            ZoneDoc::Eco => "eco",
        }
    }

    fn from_segment(s: &str) -> Option<Self> {
        Some(match s {
            "settings" => ZoneDoc::Settings,
            "proposal" => ZoneDoc::Proposal,
            "override" => ZoneDoc::Override,
            "decision" => ZoneDoc::Decision,
            "eco" => ZoneDoc::Eco,
            _ => return None,
        })
    }
}

/// A parsed climate topic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClimateTopic {
    House,
    Zone { zone: String, doc: ZoneDoc },
}

/// Topic of the house settings document.
pub fn house_topic(prefix: &str) -> String {
    format!("{}/house", prefix.trim_end_matches('/'))
}

/// Topic of one zone document.
pub fn zone_topic(prefix: &str, zone: &str, doc: ZoneDoc) -> String {
    format!(
        "{}/zone/{zone}/{}",
        prefix.trim_end_matches('/'),
        doc.segment()
    )
}

/// The subscription filters of the engine (never `<prefix>/#`): the retained documents and
/// the hold command.
pub fn filters(prefix: &str) -> [String; 3] {
    let p = prefix.trim_end_matches('/');
    [
        format!("{p}/house"),
        format!("{p}/zone/+/+"),
        format!("{p}/zone/+/hold/set"),
    ]
}

/// Topic of a zone's hold command: a JSON override request sets a boost, an empty payload
/// clears it. A command, never retained.
pub fn hold_command_topic(prefix: &str, zone: &str) -> String {
    format!("{}/zone/{zone}/hold/set", prefix.trim_end_matches('/'))
}

/// The zone of a hold command topic under `prefix`; `None` for any other topic.
pub fn parse_hold_command(prefix: &str, topic: &str) -> Option<String> {
    let zone = topic
        .strip_prefix(prefix.trim_end_matches('/'))?
        .strip_prefix("/zone/")?
        .strip_suffix("/hold/set")?;
    (!zone.is_empty() && !zone.contains('/')).then(|| zone.to_owned())
}

/// Parse a topic received under `prefix`; `None` for anything that is not a climate document.
pub fn parse(prefix: &str, topic: &str) -> Option<ClimateTopic> {
    let rest = topic
        .strip_prefix(prefix.trim_end_matches('/'))?
        .strip_prefix('/')?;
    if rest == "house" {
        return Some(ClimateTopic::House);
    }
    let mut it = rest.split('/');
    match (it.next(), it.next(), it.next(), it.next()) {
        (Some("zone"), Some(zone), Some(doc), None) if !zone.is_empty() => {
            Some(ClimateTopic::Zone {
                zone: zone.to_owned(),
                doc: ZoneDoc::from_segment(doc)?,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_and_parses() {
        assert_eq!(house_topic("nidoo/climate"), "nidoo/climate/house");
        assert_eq!(
            zone_topic("nidoo/climate/", "living", ZoneDoc::Decision),
            "nidoo/climate/zone/living/decision"
        );
        assert_eq!(
            parse("nidoo/climate", "nidoo/climate/zone/living/override"),
            Some(ClimateTopic::Zone {
                zone: "living".into(),
                doc: ZoneDoc::Override
            })
        );
        assert_eq!(
            parse("nidoo/climate", "nidoo/climate/house"),
            Some(ClimateTopic::House)
        );
        assert_eq!(
            parse("nidoo/climate", "nidoo/climate/group/g1/config"),
            None
        );
        assert_eq!(
            parse("nidoo/climate", "nidoo/climate/zone/living/config"),
            None
        );
        assert_eq!(
            parse("nidoo/climate", "nidoo/climate/zone/living/decision/x"),
            None
        );
        assert_eq!(parse("nidoo/climate", "other/zone/living/decision"), None);
        assert_eq!(
            zone_topic("nidoo/climate", "living", ZoneDoc::Eco),
            "nidoo/climate/zone/living/eco"
        );
        assert_eq!(
            parse("nidoo/climate", "nidoo/climate/zone/living/eco"),
            Some(ClimateTopic::Zone {
                zone: "living".into(),
                doc: ZoneDoc::Eco
            })
        );
        assert_eq!(
            filters("nidoo/climate"),
            [
                "nidoo/climate/house".to_owned(),
                "nidoo/climate/zone/+/+".to_owned(),
                "nidoo/climate/zone/+/hold/set".to_owned(),
            ]
        );
    }

    #[test]
    fn hold_command_round_trip() {
        let topic = hold_command_topic("nidoo/climate/", "living");
        assert_eq!(topic, "nidoo/climate/zone/living/hold/set");
        assert_eq!(
            parse_hold_command("nidoo/climate", &topic),
            Some("living".to_owned())
        );
        assert_eq!(
            parse_hold_command("nidoo/climate", "nidoo/climate/zone/living/override"),
            None
        );
        assert_eq!(
            parse_hold_command("nidoo/climate", "nidoo/climate/zone//hold/set"),
            None
        );
        assert_eq!(
            parse_hold_command("nidoo/climate", "nidoo/climate/zone/living/hold/set/x"),
            None
        );
        // Never one of the retained documents.
        assert_eq!(parse("nidoo/climate", &topic), None);
    }
}
