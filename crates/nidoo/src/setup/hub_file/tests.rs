#![allow(clippy::result_large_err)] // figment::Jail closures return figment::Error

use super::*;

#[test]
fn restore_flag_round_trips_and_old_files_load() {
    let old: HubFile = serde_json::from_str(r#"{"finished":true}"#).unwrap();
    assert_eq!(old.restore, None);
    // Written before the checked target was recorded.
    let old: HubFile = serde_json::from_str(r#"{"restore":{"attempted":true}}"#).unwrap();
    assert_eq!(
        old.restore,
        Some(RestoreState {
            attempted: true,
            target: None
        })
    );
    let file = HubFile {
        restore: Some(RestoreState {
            attempted: true,
            target: RestoreTarget::of(&full()),
        }),
        ..HubFile::default()
    };
    let tmp = tempfile::tempdir().unwrap();
    file.save(tmp.path()).unwrap();
    assert_eq!(HubFile::load(tmp.path()).unwrap().unwrap(), file);
}

fn full() -> HubFile {
    HubFile {
        mqtt: Some(MqttConfig {
            host: "192.0.2.1".into(),
            user: Some("u".into()),
            password: Some("s3cret".into()),
            ..MqttConfig::default()
        }),
        z2m_base: Some("zigbee2mqtt".into()),
        database_url: Some("postgres://u:dbpass@h/db".into()),
        location: Some(Location {
            city: "Paris".into(),
            country: Some("France".into()),
            latitude: 48.85,
            longitude: 2.35,
            timezone: "Europe/Paris".into(),
        }),
        pending_city: Some(("Lyon".into(), None)),
        step: SetupStep::Devices,
        finished: true,
        ..HubFile::default()
    }
}

#[test]
fn missing_file_is_none() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(HubFile::load(tmp.path()).unwrap().is_none());
}

#[test]
fn save_then_load_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("nested");
    let f = full();
    f.save(&dir).unwrap();
    assert_eq!(HubFile::load(&dir).unwrap().unwrap(), f);
    assert!(!dir.join("hub.json.tmp").exists());
}

#[cfg(unix)]
#[test]
fn saved_file_is_0600() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    full().save(tmp.path()).unwrap();
    let meta = std::fs::metadata(tmp.path().join("hub.json")).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
}

#[test]
fn invalid_json_is_an_error_naming_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("hub.json"), "{\"mqtt\":").unwrap();
    let err = HubFile::load(tmp.path()).unwrap_err().to_string();
    assert!(err.contains("hub.json"), "{err}");
}

#[test]
fn complete_needs_mqtt_and_database() {
    let mut f = HubFile {
        mqtt: Some(MqttConfig::default()),
        ..HubFile::default()
    };
    assert!(!f.is_complete());
    f.database_url = Some("postgres://h/db".into());
    assert!(f.is_complete());
    f.mqtt = None;
    assert!(!f.is_complete());
}

#[test]
fn apply_fields_keeps_the_machine_fields() {
    let source = HubFile {
        z2m_base: Some("z2m-source".into()),
        timezone: Some("Europe/Berlin".into()),
        http: HttpConfig {
            bind: "0.0.0.0:9000".into(),
        },
        log: LogConfig {
            level: "debug".into(),
            json: true,
        },
        ..full()
    };
    let fields = source.fields();
    assert_eq!(fields.z2m_base.as_deref(), Some("z2m-source"));
    assert_eq!(fields.location, source.location);

    let mut target = HubFile {
        mqtt: Some(MqttConfig {
            host: "192.0.2.9".into(),
            ..MqttConfig::default()
        }),
        database_url: Some("postgres://u:other@h/target".into()),
        step: SetupStep::Database,
        finished: false,
        pending_city: Some(("Lyon".into(), None)),
        ..HubFile::default()
    };
    let machine = target.clone();
    target.apply_fields(fields);

    assert_eq!(target.mqtt, machine.mqtt);
    assert_eq!(target.database_url, machine.database_url);
    assert_eq!(target.step, machine.step);
    assert_eq!(target.finished, machine.finished);
    assert_eq!(target.location, source.location);
    assert_eq!(target.timezone, source.timezone);
    assert_eq!(target.http, source.http);
    assert_eq!(target.log, source.log);
    assert_eq!(target.z2m_base, source.z2m_base);
    // The restored place wins over a city still waiting to be geocoded.
    assert_eq!(target.pending_city, None);
    assert_eq!(target.fields(), source.fields());
}

#[test]
fn restore_target_is_the_decoded_addresses_without_passwords() {
    let target = RestoreTarget::of(&HubFile {
        database_url: Some("postgres://me%40home:dbpass@DB.local/my%20db".into()),
        ..full()
    })
    .unwrap();
    assert_eq!(
        target,
        RestoreTarget {
            database: DatabaseTarget {
                host: "db.local".into(),
                port: 5432,
                database: "my db".into(),
                user: "me@home".into(),
            },
            broker_host: "192.0.2.1".into(),
            broker_port: 1883,
        }
    );
    let shown = serde_json::to_string(&target).unwrap();
    assert!(
        !shown.contains("dbpass") && !shown.contains("s3cret"),
        "{shown}"
    );
    assert_eq!(RestoreTarget::of(&HubFile::default()), None);
}

/// A retry clears only the database and broker the first attempt found empty.
#[test]
fn a_retry_clears_only_the_checked_target() {
    let file = full();
    let checked = RestoreState {
        attempted: true,
        target: RestoreTarget::of(&file),
    };
    assert!(checked.retry_clears(&file));
    // Another password, or the same URL written differently, is the same database.
    let same = HubFile {
        database_url: Some("postgres://u:changed@H:5432/%64b".into()),
        ..full()
    };
    assert!(checked.retry_clears(&same));

    let mut moved = Vec::new();
    for url in [
        "postgres://u:dbpass@other/db",
        "postgres://u:dbpass@h:5433/db",
        "postgres://u:dbpass@h/other",
        "postgres://v:dbpass@h/db",
        "not a url",
    ] {
        moved.push(HubFile {
            database_url: Some(url.into()),
            ..full()
        });
    }
    for (host, port) in [("192.0.2.2", 1883), ("192.0.2.1", 1884)] {
        let mut f = full();
        let mqtt = f.mqtt.as_mut().unwrap();
        (mqtt.host, mqtt.port) = (host.into(), port);
        moved.push(f);
    }
    for f in &moved {
        assert!(!checked.retry_clears(f), "{f:?}");
    }

    // No attempt yet, or an attempt that recorded nothing (an older file): check again.
    let fresh = RestoreState::default();
    assert!(!fresh.retry_clears(&file));
    let unrecorded = RestoreState {
        attempted: true,
        target: None,
    };
    assert!(!unrecorded.retry_clears(&file));
}

#[test]
fn debug_hides_passwords() {
    let s = format!("{:?}", full());
    assert!(!s.contains("s3cret") && !s.contains("dbpass"), "{s}");
}

#[test]
fn env_overrides_bootstrap_keys_only() {
    figment::Jail::expect_with(|jail| {
        jail.set_env("NIDOO_MQTT__HOST", "192.0.2.5");
        jail.set_env("NIDOO_MQTT__PASSWORD", "123456");
        jail.set_env("NIDOO_MQTT__PORT", "1884");
        jail.set_env("NIDOO_HISTORY__URL", "postgres://u:p@192.0.2.6/nidoo");
        jail.set_env("NIDOO_WEATHER__ENABLED", "true");
        let f = HubFile::default().with_bootstrap_env().unwrap();
        let mqtt = f.mqtt.unwrap();
        assert_eq!(mqtt.host, "192.0.2.5");
        assert_eq!(mqtt.port, 1884);
        assert_eq!(mqtt.password.as_deref(), Some("123456"));
        assert_eq!(
            f.database_url.as_deref(),
            Some("postgres://u:p@192.0.2.6/nidoo")
        );
        Ok(())
    });
}

#[test]
fn no_mqtt_env_leaves_mqtt_unset() {
    figment::Jail::expect_with(|jail| {
        jail.set_env("NIDOO_LOG__LEVEL", "debug");
        let f = HubFile::default().with_bootstrap_env().unwrap();
        assert!(f.mqtt.is_none());
        assert_eq!(f.log.level, "debug");
        Ok(())
    });
}
