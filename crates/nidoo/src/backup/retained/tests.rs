use super::*;

fn layout() -> Layout {
    Layout::from_config(&crate::config::Config::default())
}

/// One concrete topic per family under the default layout.
const EXAMPLES: &[(&str, Family)] = &[
    ("nidoo/ui/light_kitchen", Family::UiOverride),
    ("nidoo/ui/_areas", Family::Areas),
    ("nidoo/ui/_home", Family::Home),
    ("nidoo/ui/_types", Family::Types),
    ("nidoo/ui/dashboards/main", Family::Dashboard),
    ("nidoo/ui/ambiances/movie", Family::Ambiance),
    ("nidoo/automation/night", Family::Flow),
    ("nidoo/flow_labels", Family::FlowLabels),
    ("nidoo/switch/virtual/guest/config", Family::VirtualConfig),
    ("nidoo/number/virtual/level/config", Family::VirtualConfig),
    ("nidoo/virtual/guest", Family::VirtualState),
    (
        "nidoo/sensor/weather/temperature/config",
        Family::WeatherAny,
    ),
    ("nidoo/weather/temperature", Family::WeatherAny),
    ("nidoo/weather/availability", Family::WeatherAny),
    ("nidoo/solar/cache", Family::SolarCache),
    ("nidoo/sensor/solar/today/config", Family::SolarOther),
    ("nidoo/solar/today", Family::SolarOther),
    ("nidoo/solar/availability", Family::SolarOther),
    ("nidoo/group/lights/definition", Family::GroupDefinition),
    ("nidoo/group/_migrated", Family::GroupMigrated),
    ("nidoo/light/group/lights/config", Family::GroupOther),
    ("nidoo/group/lights", Family::GroupOther),
    ("nidoo/group/lights/avail", Family::GroupOther),
    ("nidoo/network/router/definition", Family::NetworkDefinition),
    (
        "nidoo/binary_sensor/network/router/config",
        Family::NetworkOther,
    ),
    ("nidoo/network/router", Family::NetworkOther),
    ("nidoo/network/router/avail", Family::NetworkOther),
    ("nidoo/push/tokens/abc", Family::PushTokens),
    ("nidoo/push/history", Family::PushHistory),
    ("nidoo/automation/morning/fired/at", Family::FlowFired),
    ("nidoo/climate/house", Family::ClimateHouse),
    (
        "nidoo/climate/zone/living/settings",
        Family::ClimateZoneSettings,
    ),
    (
        "nidoo/climate/zone/living/proposal",
        Family::ClimateZoneRuntime,
    ),
    (
        "nidoo/climate/zone/living/override",
        Family::ClimateZoneRuntime,
    ),
    (
        "nidoo/climate/zone/living/decision",
        Family::ClimateZoneRuntime,
    ),
    ("nidoo/climate/zone/living/eco", Family::ClimateZoneRuntime),
    ("nidoo/alarm/settings", Family::AlarmSettings),
    ("nidoo/alarm/state", Family::AlarmRuntime),
    ("nidoo/alarm/panel", Family::AlarmRuntime),
    ("nidoo/alarm/panel/attributes", Family::AlarmRuntime),
    (
        "nidoo/alarm_control_panel/alarm/house/config",
        Family::AlarmDiscovery,
    ),
];

#[test]
fn classify_examples() {
    let l = layout();
    for (topic, family) in EXAMPLES {
        assert_eq!(classify(topic, &l), Some(*family), "{topic}");
    }
    for family in Family::ALL {
        assert!(
            EXAMPLES.iter().any(|(_, f)| f == family),
            "no example for {family:?}"
        );
    }
    for topic in [
        "nidoo/sensor/kitchen_temp/config",
        "nidoo/climate/zone/living/hold/set",
        "nidoo/alarm/panel/set",
        "nidoo/push/responses",
        "nidoo/push/notification/frigate",
        "nidoo/ui/dashboards",
        "nidoo/ui/_other",
        "nidoo/ui/a/b",
        "zigbee2mqtt/bridge/devices",
        "homeassistant/switch/virtual/guest/config",
        "nidoo-sound/s1/volume",
    ] {
        assert_eq!(classify(topic, &l), None, "{topic}");
    }
}

#[test]
fn classify_follows_a_custom_layout() {
    let mut cfg = crate::config::Config::default();
    cfg.topics.ui_prefix = "home/ui/".into();
    cfg.topics.discovery_prefixes = vec!["ha".into(), "other".into()];
    cfg.climate.prefix = "home/climate".into();
    let l = Layout::from_config(&cfg);
    assert_eq!(classify("home/ui/light_a", &l), Some(Family::UiOverride));
    assert_eq!(
        classify("ha/switch/virtual/a/config", &l),
        Some(Family::VirtualConfig)
    );
    assert_eq!(classify("other/switch/virtual/a/config", &l), None);
    assert_eq!(classify("nidoo/ui/light_a", &l), None);
    assert_eq!(
        classify("home/climate/house", &l),
        Some(Family::ClimateHouse)
    );
}

#[test]
fn filters_catch_every_backed_up_example() {
    let l = layout();
    let filters = filters(&l);
    let mut seen = filters.clone();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), filters.len(), "filters are deduped");
    for (topic, family) in EXAMPLES.iter().filter(|(_, f)| f.backed_up()) {
        assert!(
            filters.iter().any(|f| matches(f, topic)),
            "{topic} ({family:?}) matches no filter"
        );
    }
    for family in Family::ALL {
        assert_eq!(
            family.filter(&l).is_some(),
            family.backed_up(),
            "{family:?}"
        );
    }
}

#[test]
fn wildcard_matching() {
    assert!(matches("a/+/c", "a/b/c"));
    assert!(!matches("a/+/c", "a/b/c/d"));
    assert!(matches("a/#", "a/b/c"));
    assert!(matches("a/#", "a"));
    assert!(!matches("a/b", "a/b/c"));
}

/// The first cells of the table rows, the *Retained* and *Backed up* cells beside them.
fn doc_rows() -> Vec<(String, String, String)> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/reference/mqtt-topics.md"
    );
    let text = std::fs::read_to_string(path).expect("read mqtt-topics.md");
    let mut rows = Vec::new();
    for line in text.lines().filter(|l| l.starts_with("| ")) {
        let cells = split_row(line);
        if cells.len() < 2 || cells[0] == "Topic" || cells[0].starts_with("---") {
            continue;
        }
        assert_eq!(cells.len(), 6, "a row needs six cells: {line}");
        rows.push((
            cells[0].replace('`', ""),
            cells[2].clone(),
            cells[5].clone(),
        ));
    }
    rows
}

/// Splits on unescaped `|`; `\|` becomes `|`.
fn split_row(line: &str) -> Vec<String> {
    let mut cells = vec![String::new()];
    let mut chars = line.trim().trim_start_matches('|').chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                chars.next();
                cells.last_mut().unwrap().push('|');
            }
            '|' => cells.push(String::new()),
            c => cells.last_mut().unwrap().push(c),
        }
    }
    cells.pop();
    cells.into_iter().map(|c| c.trim().to_owned()).collect()
}

#[test]
fn doc_column_matches_the_code() {
    let rows = doc_rows();
    assert!(rows.len() > 30, "the table was not read");
    for (topic, retained, backed_up) in &rows {
        let owned =
            retained == "yes" || retained == "yes (state)" || retained.starts_with("yes for");
        if !owned {
            assert_eq!(backed_up, "—", "{topic}: not a retained topic of ours");
            continue;
        }
        let family = Family::ALL
            .iter()
            .find(|f| f.doc_topic() == topic)
            .unwrap_or_else(|| panic!("no Family for the row `{topic}`"));
        let expected = if family.backed_up() { "yes" } else { "no" };
        assert_eq!(backed_up, expected, "{topic}");
    }
    for family in Family::ALL {
        assert!(
            rows.iter().any(|(t, _, _)| t == family.doc_topic()),
            "{family:?} has no row `{}`",
            family.doc_topic()
        );
    }
}

#[tokio::test]
async fn mem_broker_collects_and_clears() {
    let broker = MemBroker::default();
    broker
        .publish_retained(vec![
            ("nidoo/automation/a".into(), Bytes::from_static(b"1")),
            ("nidoo/automation/b".into(), Bytes::from_static(b"2")),
            ("nidoo/other/c".into(), Bytes::from_static(b"3")),
        ])
        .await
        .unwrap();
    let got = broker
        .collect(vec!["nidoo/automation/+".into()], QUIET)
        .await
        .unwrap();
    assert_eq!(got.len(), 2);
    assert!(
        got.iter()
            .any(|(t, p)| t == "nidoo/automation/a" && p == "1")
    );
    broker
        .publish_retained(vec![("nidoo/automation/a".into(), Bytes::new())])
        .await
        .unwrap();
    let got = broker
        .collect(vec!["nidoo/automation/+".into()], QUIET)
        .await
        .unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, "nidoo/automation/b");
}
