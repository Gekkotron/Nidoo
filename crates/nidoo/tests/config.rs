#![allow(clippy::result_large_err)] // figment::Jail closures return figment::Error

use figment::Jail;
use nidoo::config::Config;

#[test]
fn defaults_match_spec() {
    Jail::expect_with(|_jail| {
        let cfg = Config::load(None)?;
        assert_eq!(cfg.mqtt.port, 1883);
        assert_eq!(cfg.mqtt.client_id, "nidoo");
        assert!(!cfg.mqtt.send_ha_birth);
        assert_eq!(cfg.mqtt.ha_status_topic, "homeassistant/status");
        assert_eq!(cfg.topics.discovery_prefixes, vec!["nidoo".to_string()]);
        assert_eq!(cfg.topics.ui_prefix, "nidoo/ui");
        assert_eq!(cfg.topics.z2m_base, "zigbee2mqtt");
        assert_eq!(cfg.topics.automation_prefix, "nidoo/automation");
        assert_eq!(cfg.automation.runs_kept, 20);
        assert_eq!(cfg.http.bind, "0.0.0.0:8081");
        assert_eq!(cfg.commands.confirm_timeout_ms, 5000);
        assert_eq!(cfg.commands.subscription_debounce_ms, 500);
        assert_eq!(cfg.log.level, "info");
        assert!(!cfg.log.json);
        assert_eq!(cfg.push.prefix, "nidoo/push");
        assert_eq!(cfg.push.credentials, None);
        assert_eq!(cfg.frigate.url, None);
        assert_eq!(cfg.frigate.user, None);
        assert_eq!(cfg.frigate.password, None);
        assert_eq!(cfg.frigate.connect_timeout_ms, 5000);
        assert!(!cfg.frigate.live);
        Ok(())
    });
}

#[test]
fn frigate_live_parses() {
    Jail::expect_with(|jail| {
        jail.create_file(
            "oikos.toml",
            "[frigate]\nurl = \"http://f:8971\"\nlive = true\n",
        )?;
        let cfg = Config::load(None)?;
        assert!(cfg.frigate.live);
        Ok(())
    });
}

#[test]
fn env_overrides_file() {
    Jail::expect_with(|jail| {
        jail.create_file("oikos.toml", "[mqtt]\nhost = \"a\"\nport = 1884\n")?;
        let cfg = Config::load(None)?;
        assert_eq!(cfg.mqtt.host, "a");
        assert_eq!(cfg.mqtt.port, 1884);
        jail.set_env("OIKOS_MQTT__HOST", "b");
        let cfg = Config::load(None)?;
        assert_eq!(cfg.mqtt.host, "b");
        assert_eq!(cfg.mqtt.port, 1884);
        Ok(())
    });
}

#[test]
fn env_list_and_explicit_path() {
    Jail::expect_with(|jail| {
        jail.create_file("custom.toml", "[http]\nbind = \"127.0.0.1:9\"\n")?;
        jail.set_env(
            "OIKOS_TOPICS__DISCOVERY_PREFIXES",
            r#"["nidoo","homeassistant"]"#,
        );
        let cfg = Config::load(Some(std::path::Path::new("custom.toml")))?;
        assert_eq!(cfg.http.bind, "127.0.0.1:9");
        assert_eq!(
            cfg.topics.discovery_prefixes,
            vec!["nidoo".to_string(), "homeassistant".to_string()]
        );
        Ok(())
    });
}

#[test]
fn debug_redacts_password() {
    let mut cfg = Config::default();
    cfg.mqtt.password = Some("s3cret-pw".into());
    let dbg = format!("{:?}", cfg.mqtt);
    assert!(!dbg.contains("s3cret-pw"));
    assert!(dbg.contains("***"));
    cfg.mqtt.password = None;
    assert!(format!("{:?}", cfg.mqtt).contains("password: None"));
}

#[test]
fn numeric_looking_env_strings_stay_strings() {
    Jail::expect_with(|jail| {
        jail.set_env("OIKOS_MQTT__PASSWORD", "123456");
        jail.set_env("OIKOS_MQTT__CLIENT_ID", "1");
        jail.set_env("OIKOS_MQTT__USER", "007");
        jail.set_env("OIKOS_MQTT__HOST", "192.168.1.20");
        jail.set_env("OIKOS_MQTT__HA_STATUS_TOPIC", "42");
        jail.set_env("OIKOS_TOPICS__UI_PREFIX", "7");
        jail.set_env("OIKOS_TOPICS__Z2M_BASE", "0123");
        jail.set_env("OIKOS_HTTP__BIND", "8080");
        jail.set_env("OIKOS_LOG__LEVEL", "5");
        jail.set_env("OIKOS_PUSH__PREFIX", "1");
        jail.set_env("OIKOS_FRIGATE__PASSWORD", "0042");
        let cfg = Config::load(None)?;
        assert_eq!(cfg.mqtt.ha_status_topic, "42");
        assert_eq!(cfg.topics.ui_prefix, "7");
        assert_eq!(cfg.topics.z2m_base, "0123");
        assert_eq!(cfg.http.bind, "8080");
        assert_eq!(cfg.log.level, "5");
        assert_eq!(cfg.push.prefix, "1");
        assert_eq!(cfg.frigate.password.as_deref(), Some("0042"));
        assert_eq!(cfg.mqtt.password.as_deref(), Some("123456"));
        assert_eq!(cfg.mqtt.client_id, "1");
        assert_eq!(cfg.mqtt.user.as_deref(), Some("007"));
        assert_eq!(cfg.mqtt.host, "192.168.1.20");
        Ok(())
    });
}

#[test]
fn env_layer_excludes_string_keys() {
    Jail::expect_with(|jail| {
        for prefix in ["OIKOS_", "NIDOO_"] {
            for key in nidoo::config::STRING_ENV_KEYS {
                jail.set_env(
                    format!("{prefix}{}", key.replace('.', "__").to_uppercase()),
                    "123",
                );
            }
            jail.set_env(format!("{prefix}MQTT__PORT"), "1999");
        }
        for provider in [Config::legacy_env_provider(), Config::env_provider()] {
            let env = figment::Figment::from(provider);
            for key in nidoo::config::STRING_ENV_KEYS {
                assert!(
                    env.find_value(key).is_err(),
                    "{key} leaked into the Env layer"
                );
            }
            assert!(env.find_value("mqtt.port").is_ok());
        }
        Ok(())
    });
}

#[test]
fn debug_redacts_frigate_password() {
    Jail::expect_with(|jail| {
        jail.create_file(
            "oikos.toml",
            "[frigate]\nurl = \"http://f:8971\"\nuser = \"u\"\npassword = \"hunter2\"\n",
        )?;
        let cfg = Config::load(None)?;
        let text = format!("{cfg:?}");
        assert!(text.contains("http://f:8971"));
        assert!(!text.contains("hunter2"), "{text}");
        Ok(())
    });
}

#[test]
fn env_overrides_automation_keys() {
    Jail::expect_with(|jail| {
        jail.set_env("OIKOS_TOPICS__AUTOMATION_PREFIX", "home/flows");
        jail.set_env("OIKOS_AUTOMATION__RUNS_KEPT", "5");
        let cfg = Config::load(None)?;
        assert_eq!(cfg.topics.automation_prefix, "home/flows");
        assert_eq!(cfg.automation.runs_kept, 5);
        Ok(())
    });
}

#[test]
fn history_defaults_off() {
    Jail::expect_with(|_jail| {
        let cfg = Config::load(None)?;
        assert!(!cfg.history.enabled());
        assert_eq!(cfg.history.url, "");
        assert_eq!(cfg.history.buffer_rows, 100_000);
        Ok(())
    });
}

#[test]
fn history_url_from_env_stays_a_string() {
    Jail::expect_with(|jail| {
        jail.set_env(
            "OIKOS_HISTORY__URL",
            "postgres://nidoo:s3cret@192.168.1.20:5432/nidoo",
        );
        let cfg = Config::load(None)?;
        assert_eq!(
            cfg.history.url,
            "postgres://nidoo:s3cret@192.168.1.20:5432/nidoo"
        );
        assert!(cfg.history.enabled());
        Ok(())
    });
}

#[test]
fn history_debug_masks_password() {
    Jail::expect_with(|jail| {
        jail.create_file(
            "oikos.toml",
            "[history]\nurl = \"postgres://nidoo:s3cret@192.168.1.20:5432/nidoo\"\n",
        )?;
        let cfg = Config::load(None)?;
        let text = format!("{:?}", cfg.history);
        assert!(text.contains("***"), "{text}");
        assert!(!text.contains("s3cret"), "{text}");
        Ok(())
    });
}

#[test]
fn flows_timezone_is_read() {
    Jail::expect_with(|jail| {
        assert_eq!(Config::load(None)?.flows.timezone, None);
        jail.create_file("oikos.toml", "[flows]\ntimezone = \"Europe/Paris\"\n")?;
        assert_eq!(
            Config::load(None)?.flows.timezone.as_deref(),
            Some("Europe/Paris")
        );
        jail.set_env("OIKOS_FLOWS__TIMEZONE", "UTC");
        assert_eq!(Config::load(None)?.flows.timezone.as_deref(), Some("UTC"));
        Ok(())
    });
}

#[test]
fn push_defaults_are_new_topics() {
    Jail::expect_with(|_jail| {
        let push = Config::load(None)?.push;
        assert_eq!(push.prefix, "nidoo/push");
        assert_eq!(push.credentials, None);
        assert_eq!(push.retry_interval, 60);
        assert_eq!(push.max_retries, 10);
        assert_eq!(push.history_size, 100);
        assert_eq!(push.history_days, 7);
        Ok(())
    });
}

#[test]
fn push_keys_are_read() {
    Jail::expect_with(|jail| {
        jail.create_file(
            "oikos.toml",
            "[push]\nprefix = \"home/push\"\n\
             credentials = \"/srv/keys/fcm.json\"\nretry_interval = 30\n",
        )?;
        let push = Config::load(None)?.push;
        assert_eq!(push.prefix, "home/push");
        assert_eq!(
            push.credentials.as_deref(),
            Some(std::path::Path::new("/srv/keys/fcm.json"))
        );
        assert_eq!(push.retry_interval, 30);
        jail.set_env("OIKOS_PUSH__CREDENTIALS", "/tmp/key.json");
        assert_eq!(
            Config::load(None)?.push.credentials.as_deref(),
            Some(std::path::Path::new("/tmp/key.json"))
        );
        Ok(())
    });
}

#[test]
fn push_key_in_secrets_next_to_the_config_is_picked_up() {
    Jail::expect_with(|jail| {
        assert_eq!(Config::load(None)?.push.credentials, None);
        jail.create_dir("secrets")?;
        jail.create_file("secrets/firebase.json", "{}")?;
        assert_eq!(
            Config::load(None)?.push.credentials.as_deref(),
            Some(std::path::Path::new("secrets/firebase.json"))
        );
        jail.create_dir("etc")?;
        jail.create_file("etc/hub.toml", "")?;
        assert_eq!(
            Config::load(Some(std::path::Path::new("etc/hub.toml")))?
                .push
                .credentials,
            None,
            "the key is looked up next to the config file in use"
        );
        jail.create_dir("etc/secrets")?;
        jail.create_file("etc/secrets/firebase.json", "{}")?;
        assert_eq!(
            Config::load(Some(std::path::Path::new("etc/hub.toml")))?
                .push
                .credentials
                .as_deref(),
            Some(std::path::Path::new("etc/secrets/firebase.json"))
        );
        Ok(())
    });
}

#[test]
fn explicit_push_credentials_win_over_the_secrets_folder() {
    Jail::expect_with(|jail| {
        jail.create_dir("secrets")?;
        jail.create_file("secrets/firebase.json", "{}")?;
        jail.create_file(
            "oikos.toml",
            "[push]\ncredentials = \"/srv/keys/fcm.json\"\n",
        )?;
        assert_eq!(
            Config::load(None)?.push.credentials.as_deref(),
            Some(std::path::Path::new("/srv/keys/fcm.json"))
        );
        jail.set_env("OIKOS_PUSH__CREDENTIALS", "/tmp/key.json");
        assert_eq!(
            Config::load(None)?.push.credentials.as_deref(),
            Some(std::path::Path::new("/tmp/key.json"))
        );
        Ok(())
    });
}

#[test]
fn old_push_keys_are_rejected() {
    Jail::expect_with(|jail| {
        jail.create_file("oikos.toml", "[push]\nhistory_topic = \"x\"\n")?;
        let err = Config::load(None).expect_err("history_topic is no longer a key");
        let text = err.to_string();
        assert!(text.contains("history_topic"), "{text}");
        assert!(
            text.contains("credentials"),
            "the error names the new keys: {text}"
        );
        jail.create_file("oikos.toml", "[push]\nresponses_topic = \"x\"\n")?;
        let err = Config::load(None).expect_err("responses_topic is no longer a key");
        assert!(err.to_string().contains("responses_topic"), "{err}");
        Ok(())
    });
}

#[test]
fn legacy_config_still_loads() {
    Jail::expect_with(|_jail| {
        let path = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/legacy-oikos.toml"
        ));
        let cfg = Config::load(Some(path))?;
        assert_eq!(cfg.push.prefix, "nidoo/push");
        assert_eq!(cfg.push.credentials, None);
        Ok(())
    });
}

#[test]
fn frigate_bridge_defaults_and_string_env() {
    Jail::expect_with(|jail| {
        let cfg = Config::load(None)?;
        let f = &cfg.frigate;
        assert!(!f.bridge && f.media_url.is_none());
        assert!(f.labels.is_empty() && f.cameras.is_empty() && f.zones.is_empty());
        assert_eq!((f.cooldown, f.ttl), (0, 43200));
        assert_eq!(
            (f.severity.as_str(), f.topic_prefix.as_str()),
            ("warning", "frigate")
        );
        assert!(f.bridge_settings().is_none());

        jail.set_env("OIKOS_FRIGATE__SEVERITY", "123");
        jail.set_env("OIKOS_FRIGATE__TOPIC_PREFIX", "007");
        jail.set_env("OIKOS_FRIGATE__MEDIA_URL", "http://192.168.1.20:5000/");
        jail.set_env("OIKOS_FRIGATE__BRIDGE", "true");
        let cfg = Config::load(None)?;
        assert_eq!(cfg.frigate.severity, "123");
        assert_eq!(cfg.frigate.events_topic(), "007/events");
        let settings = cfg.frigate.bridge_settings().expect("bridge on");
        assert_eq!(settings.media_url, "http://192.168.1.20:5000");
        Ok(())
    });
}

#[test]
fn watchdog_defaults_and_validation() {
    use nidoo::config::AclSource;
    Jail::expect_with(|jail| {
        let cfg = Config::load(None)?;
        let w = &cfg.watchdog;
        assert!(!w.gateway.enabled && !w.acl.enabled);
        assert_eq!(w.gateway.prefix, "encryption-gateway/health");
        assert_eq!(w.acl.source, AclSource::Topic);
        assert_eq!(w.acl.log_topic, "$SYS/broker/log/#");
        assert_eq!((w.acl.log_file.is_none(), w.acl.cooldown), (true, 300));
        assert_eq!(w.validate(), Ok(()));

        // A file source without a log file is a config error, but only once enabled.
        jail.set_env("OIKOS_WATCHDOG__ACL__SOURCE", "file");
        assert_eq!(Config::load(None)?.watchdog.validate(), Ok(()));
        jail.set_env("OIKOS_WATCHDOG__ACL__ENABLED", "true");
        let cfg = Config::load(None)?;
        assert_eq!(cfg.watchdog.acl.source, AclSource::File);
        assert!(cfg.watchdog.validate().is_err());

        jail.set_env("OIKOS_WATCHDOG__ACL__LOG_FILE", "007");
        let cfg = Config::load(None)?;
        assert_eq!(
            cfg.watchdog.acl.log_file.as_deref(),
            Some(std::path::Path::new("007"))
        );
        assert_eq!(cfg.watchdog.validate(), Ok(()));

        jail.set_env("OIKOS_WATCHDOG__GATEWAY__ENABLED", "true");
        jail.set_env("OIKOS_WATCHDOG__GATEWAY__PREFIX", "/");
        assert!(Config::load(None)?.watchdog.validate().is_err());
        Ok(())
    });
}

#[test]
fn watchdog_rejects_an_unknown_source() {
    Jail::expect_with(|jail| {
        jail.set_env("OIKOS_WATCHDOG__ACL__SOURCE", "stdin");
        assert!(Config::load(None).is_err());
        Ok(())
    });
}

fn enabled_weather(city: &str) -> nidoo::config::WeatherConfig {
    nidoo::config::WeatherConfig {
        enabled: true,
        city: city.to_owned(),
        ..Default::default()
    }
}

#[test]
fn weather_disabled_by_default_and_valid() {
    let cfg = Config::default();
    assert!(!cfg.weather.enabled);
    assert_eq!(cfg.weather.refresh_minutes, 15);
    assert_eq!(cfg.weather.validate(), Ok(()));
}

#[test]
fn weather_enabled_needs_a_city() {
    assert!(enabled_weather("").validate().unwrap_err().contains("city"));
    let blank = enabled_weather("  ").validate().unwrap_err();
    assert!(blank.starts_with("[weather]"), "{blank}");
    assert_eq!(enabled_weather("Paris").validate(), Ok(()));
    // Disabled never complains.
    let off = nidoo::config::WeatherConfig {
        refresh_minutes: 1,
        country: Some("FRA".into()),
        ..Default::default()
    };
    assert_eq!(off.validate(), Ok(()));
}

#[test]
fn weather_country_is_two_letters() {
    for (code, ok) in [("FR", true), ("fr", true), ("FRA", false), ("F1", false)] {
        let mut cfg = enabled_weather("Paris");
        cfg.country = Some(code.to_owned());
        assert_eq!(cfg.validate().is_ok(), ok, "{code}");
    }
}

#[test]
fn weather_refresh_has_a_floor() {
    let mut cfg = enabled_weather("Paris");
    cfg.refresh_minutes = 9;
    assert!(cfg.validate().unwrap_err().starts_with("[weather]"));
    cfg.refresh_minutes = 10;
    assert_eq!(cfg.validate(), Ok(()));
}

#[test]
fn weather_refresh_has_a_ceiling() {
    let mut cfg = enabled_weather("Paris");
    cfg.refresh_minutes = 1441;
    assert!(cfg.validate().unwrap_err().starts_with("[weather]"));
    cfg.refresh_minutes = 1440;
    assert_eq!(cfg.validate(), Ok(()));
}

#[test]
fn weather_from_toml_and_env() {
    Jail::expect_with(|jail| {
        jail.create_file(
            "oikos.toml",
            "[weather]\nenabled = true\ncity = \"Lyon\"\ncountry = \"FR\"\n",
        )?;
        let cfg = Config::load(None)?;
        assert!(cfg.weather.enabled);
        assert_eq!(cfg.weather.city, "Lyon");
        assert_eq!(cfg.weather.country.as_deref(), Some("FR"));
        jail.set_env("OIKOS_WEATHER__CITY", "1234");
        jail.set_env("OIKOS_WEATHER__COUNTRY", "42");
        let cfg = Config::load(None)?;
        assert_eq!(cfg.weather.city, "1234");
        assert_eq!(cfg.weather.country.as_deref(), Some("42"));
        Ok(())
    });
}

mod solcast {
    #![allow(clippy::result_large_err)]
    use super::*;
    use nidoo::config::SolcastConfig;

    fn enabled(sites: &[&str], daily_limit: u32, key: Option<&str>) -> SolcastConfig {
        SolcastConfig {
            enabled: true,
            sites: sites.iter().map(|s| (*s).to_owned()).collect(),
            daily_limit,
            api_key_file: key.map(Into::into),
            api_key: None,
        }
    }

    #[test]
    fn solcast_defaults() {
        Jail::expect_with(|jail| {
            let cfg = Config::load(None)?;
            assert!(!cfg.solcast.enabled);
            assert!(cfg.solcast.sites.is_empty());
            assert_eq!(cfg.solcast.daily_limit, 10);
            assert_eq!(
                cfg.solcast.api_key_file.as_deref(),
                Some(std::path::Path::new("secrets/solcast.key"))
            );
            jail.create_dir("etc")?;
            jail.create_file("etc/hub.toml", "")?;
            let cfg = Config::load(Some(std::path::Path::new("etc/hub.toml")))?;
            assert!(
                cfg.solcast
                    .api_key_file
                    .unwrap()
                    .ends_with("etc/secrets/solcast.key")
            );
            Ok(())
        });
    }

    #[test]
    fn solcast_validate_messages() {
        Jail::expect_with(|jail| {
            let err = |c: SolcastConfig| c.validate().unwrap_err();
            assert_eq!(
                err(enabled(&[], 10, None)),
                "[solcast] sites needs at least one rooftop resource id"
            );
            assert_eq!(
                err(enabled(&["a", " "], 10, None)),
                "[solcast] sites has an empty id"
            );
            assert_eq!(
                err(enabled(&["a b"], 10, None)),
                "[solcast] sites has an id with characters other than letters, digits, - and _"
            );
            assert_eq!(
                err(enabled(&["a/../b"], 10, None)),
                "[solcast] sites has an id with characters other than letters, digits, - and _"
            );
            assert_eq!(
                err(enabled(&["a", "b", "c"], 2, None)),
                "[solcast] daily_limit is 2: it must be at least the number of sites (3)"
            );
            assert_eq!(
                err(enabled(&["a"], 10, Some("missing.key"))),
                format!(
                    "[solcast] cannot read the API key file missing.key: {}",
                    std::fs::read_to_string("missing.key").unwrap_err()
                )
            );
            jail.create_file("empty.key", " \n")?;
            assert_eq!(
                err(enabled(&["a"], 10, Some("empty.key"))),
                "[solcast] the API key file empty.key is empty"
            );
            jail.create_file("ok.key", "  k \n")?;
            let ok = enabled(&["abcd-1234_x"], 10, Some("ok.key"));
            assert_eq!(ok.validate(), Ok(()));
            assert_eq!(ok.read_key().unwrap(), "k");
            Ok(())
        });
    }

    #[test]
    fn solcast_disabled_is_always_valid() {
        let cfg = SolcastConfig {
            enabled: false,
            sites: vec![String::new(), "a b".to_owned()],
            daily_limit: 0,
            api_key_file: Some("nope.key".into()),
            api_key: None,
        };
        assert_eq!(cfg.validate(), Ok(()));
    }
}

/// The image sets no env var of its own: a `NIDOO_*` default would beat a user's `OIKOS_*`
/// override in `.env` (NIDOO wins), and an `OIKOS_*` one would log the deprecation warning on
/// every start. The defaults (`0.0.0.0:8081`, `./data` under `WORKDIR /`) already fit the image.
#[test]
fn dockerfile_sets_no_env_that_shadows_a_user_override() {
    let dockerfile =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Dockerfile")).unwrap();
    let runtime = dockerfile
        .split_once("AS runtime")
        .expect("a runtime stage")
        .1;
    for line in runtime.lines().map(str::trim) {
        assert!(
            !(line.starts_with("ENV NIDOO_") || line.starts_with("ENV OIKOS_")),
            "the image sets {line}"
        );
    }
    assert!(
        runtime.lines().any(|l| l.trim() == "WORKDIR /"),
        "the runtime stage must run from / so the default ./data is /data"
    );
    assert_eq!(Config::default().http.bind, "0.0.0.0:8081");
}
