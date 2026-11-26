use super::*;
use nidoo_api::Location;
use serde_json::json;

fn owned(rows: Vec<(&'static str, Value)>) -> Vec<(String, Value)> {
    rows.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
}

fn sample() -> Sections {
    let mut s = Sections::default();
    s.weather.enabled = true;
    s.weather.refresh_minutes = 30;
    s.solcast.enabled = true;
    s.solcast.sites = vec!["site-1".into()];
    s.solcast.api_key = Some("sk-secret".into());
    s.frigate.url = Some("http://192.0.2.5:8971".into());
    s.push.service_account = Some("{\"k\":1}".into());
    s.push.max_retries = 3;
    s.climate.enabled = true;
    s.advanced.history_buffer_rows = 5;
    s.advanced.topics.z2m_base = "z2m-x".into();
    s.system.notify = true;
    s.backup.password = Some("backup-pass".into());
    s.backup.ftp.host = "nas.local".into();
    s
}

fn location() -> Location {
    Location {
        city: "Lyon".into(),
        country: Some("FR".into()),
        latitude: 45.76,
        longitude: 4.84,
        timezone: "Europe/Paris".into(),
    }
}

fn file() -> HubFile {
    HubFile {
        database_url: Some("postgres://u:pw@db.example.com/nidoo".into()),
        ..HubFile::default()
    }
}

#[test]
fn timezone_without_location_reaches_flows_and_location_wins() {
    let mut f = file();
    f.timezone = Some("America/Toronto".into());
    let cfg = assemble(&f, &Sections::default());
    assert_eq!(cfg.flows.timezone.as_deref(), Some("America/Toronto"));
    assert_eq!(cfg.flows.latitude, None);
    f.location = Some(location());
    let cfg = assemble(&f, &Sections::default());
    assert_eq!(cfg.flows.timezone.as_deref(), Some("Europe/Paris"));
}

#[test]
fn rows_round_trip() {
    let s = sample();
    assert_eq!(Sections::from_rows(owned(s.to_rows())), s);
}

#[test]
fn row_keys_match_section_keys() {
    let keys: Vec<_> = Sections::default().to_rows().iter().map(|r| r.0).collect();
    assert_eq!(keys, SECTION_KEYS);
}

#[test]
fn sections_rows_include_backup() {
    assert_eq!(SECTION_KEYS.last(), Some(&"backup"));
    let rows = owned(sample().to_rows());
    assert!(rows.iter().any(|(k, _)| k == "backup"));
    let back = Sections::from_rows(rows);
    assert_eq!(back.backup.ftp.host, "nas.local");
    assert_eq!(back.backup.password.as_deref(), Some("backup-pass"));
}

#[test]
fn system_section_defaults_off_and_round_trips() {
    // A database or cache from before this section has no "system" row: off.
    let rows = vec![("weather".to_owned(), json!({"enabled": true}))];
    let got = Sections::from_rows(rows);
    assert!(!got.system.notify);
    assert!(!assemble(&file(), &got).system.notify);
    let mut on = Sections::default();
    on.system.notify = true;
    let back = Sections::from_rows(owned(on.to_rows()));
    assert!(back.system.notify);
    assert!(assemble(&file(), &back).system.notify);
    // An old cache file without the key still parses.
    let old: Sections = serde_json::from_value(json!({"weather": {"enabled": true}})).unwrap();
    assert!(!old.system.notify);
}

#[test]
fn supervisor_settings_default_on_and_round_trip() {
    use nidoo_api::{SupervisorSensitivity, SupervisorSettings};
    // A row from before the supervisor existed: the supervisor is on with its defaults.
    let rows = vec![("system".to_owned(), json!({"notify": true}))];
    let got = Sections::from_rows(rows);
    assert!(got.system.notify);
    let sup = &got.system.supervisor;
    assert!(sup.enabled && sup.notify && sup.notify_recovered);
    assert_eq!(sup.sensitivity, SupervisorSensitivity::Normal);
    assert!(sup.ignored.is_empty());
    assert_eq!(*sup, SupervisorSettings::default());

    let mut changed = Sections::default();
    changed.system.supervisor = SupervisorSettings {
        enabled: true,
        notify: false,
        notify_recovered: false,
        sensitivity: SupervisorSensitivity::High,
        ignored: vec!["sensor.battery".into()],
    };
    let back = Sections::from_rows(owned(changed.to_rows()));
    assert_eq!(back.system.supervisor, changed.system.supervisor);
    assert_eq!(
        assemble(&file(), &back).system.supervisor,
        changed.system.supervisor
    );
    let row = changed
        .to_rows()
        .into_iter()
        .find(|(k, _)| *k == "system")
        .unwrap()
        .1;
    assert_eq!(row["supervisor"]["sensitivity"], "high");
}

#[test]
fn missing_rows_are_defaults() {
    assert_eq!(Sections::from_rows(vec![]), Sections::default());
}

#[test]
fn unknown_row_is_ignored() {
    let rows = vec![("nope".to_owned(), json!({"a": 1}))];
    assert_eq!(Sections::from_rows(rows), Sections::default());
}

#[test]
fn bad_row_falls_back_to_that_section_only() {
    let s = sample();
    let rows = vec![
        ("weather".to_owned(), json!(5)),
        (
            "solcast".to_owned(),
            serde_json::to_value(&s.solcast).unwrap(),
        ),
    ];
    let got = Sections::from_rows(rows);
    assert_eq!(got.weather, WeatherSection::default());
    assert_eq!(got.solcast, s.solcast);
}

#[test]
fn assemble_maps_location_and_bootstrap() {
    let mut f = file();
    f.location = Some(location());
    let cfg = assemble(&f, &Sections::default());
    assert_eq!(cfg.flows.timezone.as_deref(), Some("Europe/Paris"));
    assert_eq!(
        (cfg.flows.latitude, cfg.flows.longitude),
        (Some(45.76), Some(4.84))
    );
    assert_eq!(cfg.weather.city, "Lyon");
    assert_eq!(cfg.weather.country.as_deref(), Some("FR"));
    assert_eq!(
        (cfg.weather.latitude, cfg.weather.longitude),
        (Some(45.76), Some(4.84))
    );
    assert_eq!(cfg.history.url, f.database_url.unwrap());
    assert_eq!(cfg.topics.z2m_base, "zigbee2mqtt");
    assert!(cfg.push.credentials.is_none());
    assert!(cfg.solcast.api_key_file.is_none());
}

#[test]
fn assemble_maps_sections() {
    let s = sample();
    let cfg = assemble(&file(), &s);
    assert!(cfg.weather.enabled);
    assert_eq!(cfg.weather.refresh_minutes, 30);
    assert_eq!(cfg.solcast.api_key.as_deref(), Some("sk-secret"));
    assert_eq!(cfg.push.service_account.as_deref(), Some("{\"k\":1}"));
    assert_eq!(cfg.push.max_retries, 3);
    assert_eq!(cfg.history.buffer_rows, 5);
    assert_eq!(cfg.topics.z2m_base, "z2m-x");
    assert!(cfg.climate.enabled);
    assert_eq!(cfg.frigate, s.frigate);
}

#[test]
fn assemble_z2m_base_from_file_wins() {
    let mut f = file();
    f.z2m_base = Some("z2m".into());
    let cfg = assemble(&f, &sample());
    assert_eq!(cfg.topics.z2m_base, "z2m");
}

#[test]
fn solcast_api_key_beats_the_file() {
    let cfg = SolcastConfig {
        api_key: Some(" k ".into()),
        api_key_file: Some("/nonexistent/solcast.key".into()),
        ..SolcastConfig::default()
    };
    assert_eq!(cfg.read_key(), Ok("k".to_owned()));
}

#[test]
fn debug_hides_service_account_and_api_key() {
    let mut s = sample();
    s.push.service_account = Some("PRIVATE-KEY-JSON".into());
    s.solcast.api_key = Some("SOLCAST-KEY".into());
    s.frigate.password = Some("FRIGATE-PW".into());
    let mut f = file();
    f.location = Some(location());
    let cfg = assemble(&f, &s);
    for text in [format!("{cfg:?}"), format!("{s:?}")] {
        for secret in ["PRIVATE-KEY-JSON", "SOLCAST-KEY", "FRIGATE-PW"] {
            assert!(!text.contains(secret), "{secret} leaked in {text}");
        }
    }
}
