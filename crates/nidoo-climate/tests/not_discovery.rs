use nidoo_climate::topics::{ZoneDoc, house_topic, zone_topic};

#[test]
fn no_climate_topic_classifies_as_discovery() {
    use nidoo_core::discovery::{TopicClass, classify};
    let prefixes = vec!["nidoo".to_owned()];
    let mut topics = vec![house_topic("nidoo/climate")];
    for doc in [
        ZoneDoc::Settings,
        ZoneDoc::Proposal,
        ZoneDoc::Override,
        ZoneDoc::Decision,
    ] {
        topics.push(zone_topic("nidoo/climate", "living", doc));
    }
    for t in topics {
        assert_eq!(classify(&t, &prefixes, &[]), TopicClass::Other, "{t}");
    }
}
