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

/// The two subscription filters of the engine (never `<prefix>/#`).
pub fn filters(prefix: &str) -> [String; 2] {
    let p = prefix.trim_end_matches('/');
    [format!("{p}/house"), format!("{p}/zone/+/+")]
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
        assert_eq!(house_topic("oikos/climate"), "oikos/climate/house");
        assert_eq!(
            zone_topic("oikos/climate/", "living", ZoneDoc::Decision),
            "oikos/climate/zone/living/decision"
        );
        assert_eq!(
            parse("oikos/climate", "oikos/climate/zone/living/override"),
            Some(ClimateTopic::Zone {
                zone: "living".into(),
                doc: ZoneDoc::Override
            })
        );
        assert_eq!(
            parse("oikos/climate", "oikos/climate/house"),
            Some(ClimateTopic::House)
        );
        assert_eq!(
            parse("oikos/climate", "oikos/climate/group/g1/config"),
            None
        );
        assert_eq!(
            parse("oikos/climate", "oikos/climate/zone/living/config"),
            None
        );
        assert_eq!(
            parse("oikos/climate", "oikos/climate/zone/living/decision/x"),
            None
        );
        assert_eq!(parse("oikos/climate", "other/zone/living/decision"), None);
        assert_eq!(
            zone_topic("oikos/climate", "living", ZoneDoc::Eco),
            "oikos/climate/zone/living/eco"
        );
        assert_eq!(
            parse("oikos/climate", "oikos/climate/zone/living/eco"),
            Some(ClimateTopic::Zone {
                zone: "living".into(),
                doc: ZoneDoc::Eco
            })
        );
        assert_eq!(
            filters("oikos/climate"),
            [
                "oikos/climate/house".to_owned(),
                "oikos/climate/zone/+/+".to_owned()
            ]
        );
    }
}
