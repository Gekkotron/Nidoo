//! Imports an existing `oikos.toml` into the new shape: a [`HubFile`] and the [`Sections`].
//!
//! Key files: `Config::load` already defaults `secrets/firebase.json` and `secrets/solcast.key`
//! to the TOML's folder; a path named in the TOML is used as written, which is how the running
//! hub reads it (relative to the process working directory).

use std::path::{Path, PathBuf};

use anyhow::Context;
use nidoo_api::{Location, SetupStep};

use crate::config::Config;
use crate::setup::hub_file::HubFile;
use crate::setup::secret::split_userinfo;
use crate::setup::sections::{
    AdvancedSection, PushSection, Sections, SolcastSection, WeatherSection,
};

/// The TOML to import: the `--config` file when it exists, else `<data_dir>/oikos.toml` when it
/// exists, else (with no `--config`) `oikos.toml` in the working directory, the file a hub
/// started without a flag has always read. `None` when none exists: a missing file never
/// imports.
pub fn toml_candidate(data_dir: &Path, flag: Option<&Path>) -> Option<PathBuf> {
    if let Some(flag) = flag.filter(|p| p.is_file()) {
        return Some(flag.to_path_buf());
    }
    let default = data_dir.join("oikos.toml");
    if default.is_file() {
        return Some(default);
    }
    flag.is_none().then(cwd_toml).flatten()
}

/// `oikos.toml` in the working directory, when it exists.
pub fn cwd_toml() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let path = cwd.join("oikos.toml");
    path.is_file().then_some(path)
}

/// Moves a `user:password@` written into a Frigate address into the user and password
/// fields (a field already set wins), so no read ever shows it and the form can edit it.
fn lift_frigate_login(frigate: &mut crate::config::FrigateConfig) {
    for address in [&mut frigate.url, &mut frigate.media_url] {
        let Some((bare, user, password)) = address.as_deref().and_then(split_userinfo) else {
            continue;
        };
        tracing::info!("frigate: the login written in its address moved to its own fields");
        *address = Some(bare);
        if frigate.user.is_none() {
            frigate.user = user;
        }
        if frigate.password.is_none() {
            frigate.password = password;
        }
    }
}

/// Loads and validates `path` as today's hub does, then maps it into the new files.
pub fn import_toml(path: &Path) -> anyhow::Result<(HubFile, Sections)> {
    let cfg = Config::load(Some(path)).context("cannot load the config")?;
    cfg.validate_all()
        .map_err(|e| anyhow::anyhow!("invalid config: {e}"))?;

    let database_url = (!cfg.history.url.is_empty()).then(|| cfg.history.url.clone());
    let city = cfg.weather.city.clone();
    let country = cfg.weather.country.clone();
    let coords = match (cfg.flows.latitude, cfg.flows.longitude) {
        (Some(lat), Some(lon)) => Some((lat, lon)),
        _ => match (cfg.weather.latitude, cfg.weather.longitude) {
            (Some(lat), Some(lon)) => Some((lat, lon)),
            _ => None,
        },
    };
    let (location, pending_city) = match coords {
        Some((latitude, longitude)) => {
            let timezone = cfg
                .flows
                .timezone
                .clone()
                .or_else(|| iana_time_zone::get_timezone().ok())
                .unwrap_or_else(|| "UTC".to_owned());
            let loc = Location {
                city,
                country,
                latitude,
                longitude,
                timezone,
            };
            (Some(loc), None)
        }
        _ if !city.is_empty() => (None, Some((city, country))),
        _ => (None, None),
    };
    let timezone = if location.is_none() {
        cfg.flows.timezone.clone()
    } else {
        None
    };
    let finished = database_url.is_some();
    let file = HubFile {
        mqtt: Some(cfg.mqtt.clone()),
        z2m_base: Some(cfg.topics.z2m_base.clone()),
        database_url,
        location,
        pending_city,
        timezone,
        http: cfg.http.clone(),
        log: cfg.log.clone(),
        step: if finished {
            SetupStep::Done
        } else {
            SetupStep::Database
        },
        finished,
        settings_imported: false,
    };

    let service_account = cfg.push.service_account.clone().or_else(|| {
        let path = cfg.push.credentials.as_deref()?;
        match std::fs::read_to_string(path) {
            Ok(text) => Some(text),
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "push key file not imported");
                None
            }
        }
    });
    let api_key = cfg.solcast.read_key().ok();
    if api_key.is_none() && cfg.solcast.api_key_file.is_some() {
        tracing::warn!(
            path = ?cfg.solcast.api_key_file,
            "solcast key file not imported (missing, unreadable or empty)"
        );
    }
    let sections = Sections {
        weather: WeatherSection {
            enabled: cfg.weather.enabled,
            refresh_minutes: cfg.weather.refresh_minutes,
        },
        solcast: SolcastSection {
            enabled: cfg.solcast.enabled,
            sites: cfg.solcast.sites.clone(),
            daily_limit: cfg.solcast.daily_limit,
            api_key,
        },
        frigate: {
            let mut frigate = cfg.frigate.clone();
            lift_frigate_login(&mut frigate);
            frigate
        },
        push: PushSection {
            service_account,
            prefix: cfg.push.prefix.clone(),
            legacy_prefix: cfg.push.legacy_prefix.clone(),
            retry_interval: cfg.push.retry_interval,
            max_retries: cfg.push.max_retries,
            history_size: cfg.push.history_size,
            history_days: cfg.push.history_days,
        },
        climate: cfg.climate.clone(),
        advanced: AdvancedSection {
            topics: cfg.topics.clone(),
            commands: cfg.commands.clone(),
            automation: cfg.automation.clone(),
            watchdog: cfg.watchdog.clone(),
            groups: cfg.groups.clone(),
            alarm: cfg.alarm.clone(),
            history_buffer_rows: cfg.history.buffer_rows,
        },
    };
    Ok((file, sections))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::sections::assemble;

    // Lib tests that call `Config::load` must run under `figment::Jail`: it serializes them with
    // the tests that set `OIKOS_*` env vars, which `load` would otherwise read mid-test.
    /// Holds figment's global lock so `OIKOS_*` env tests elsewhere cannot leak in mid-load.
    #[allow(clippy::result_large_err)]
    fn import_locked(p: &Path) -> anyhow::Result<(HubFile, Sections)> {
        let mut out = None;
        figment::Jail::expect_with(|_| {
            out = Some(import_toml(p));
            Ok(())
        });
        out.unwrap()
    }

    #[allow(clippy::result_large_err)]
    fn load_locked(p: &Path) -> Result<Config, figment::Error> {
        let mut out = None;
        figment::Jail::expect_with(|_| {
            out = Some(Config::load(Some(p)));
            Ok(())
        });
        out.unwrap()
    }

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, body).unwrap();
        p
    }

    const FULL: &str = r#"
[mqtt]
host = "192.0.2.1"
[history]
url = "postgres://u:pw@db.example.com/oikos"
[flows]
latitude = 45.76
longitude = 4.84
timezone = "Europe/Paris"
[weather]
enabled = true
city = "Lyon"
country = "FR"
"#;

    #[test]
    fn imports_mqtt_history_and_location() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "oikos.toml", FULL);
        let (file, sections) = import_locked(&p).unwrap();
        assert_eq!(file.mqtt.as_ref().unwrap().host, "192.0.2.1");
        assert_eq!(
            file.database_url.as_deref(),
            Some("postgres://u:pw@db.example.com/oikos")
        );
        let loc = file.location.unwrap();
        assert_eq!(
            (
                loc.city.as_str(),
                loc.country.as_deref(),
                loc.timezone.as_str()
            ),
            ("Lyon", Some("FR"), "Europe/Paris")
        );
        assert_eq!((loc.latitude, loc.longitude), (45.76, 4.84));
        assert!(file.pending_city.is_none());
        assert!(file.finished);
        assert_eq!(file.step, SetupStep::Done);
        assert!(!file.settings_imported);
        assert_eq!(file.z2m_base.as_deref(), Some("zigbee2mqtt"));
        assert!(sections.weather.enabled);
    }

    #[test]
    fn no_history_url_means_database_step() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "oikos.toml", "[mqtt]\nhost = \"192.0.2.1\"\n");
        let (file, _) = import_locked(&p).unwrap();
        assert_eq!(file.database_url, None);
        assert!(!file.finished);
        assert_eq!(file.step, SetupStep::Database);
    }

    #[test]
    fn no_coordinates_means_pending_city() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "oikos.toml",
            "[weather]\nenabled = true\ncity = \"Lyon\"\ncountry = \"FR\"\n",
        );
        let (file, _) = import_locked(&p).unwrap();
        assert!(file.location.is_none());
        assert_eq!(file.pending_city, Some(("Lyon".into(), Some("FR".into()))));
    }

    #[test]
    fn a_city_only_weather_import_assembles_a_valid_config() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "oikos.toml",
            "[weather]\nenabled = true\ncity = \"Lyon\"\ncountry = \"FR\"\n",
        );
        let (file, sections) = import_locked(&p).unwrap();
        let cfg = assemble(&file, &sections);
        assert_eq!(cfg.validate_all(), Ok(()));
        // The poller geocodes the city as it does today.
        assert_eq!(cfg.weather.city, "Lyon");
        assert_eq!(cfg.weather.country.as_deref(), Some("FR"));
        assert_eq!((cfg.weather.latitude, cfg.weather.longitude), (None, None));
    }

    #[test]
    fn frigate_userinfo_moves_into_its_own_fields() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "oikos.toml",
            "[frigate]\nurl = \"http://fu:f%40p@192.0.2.5:8971\"\nmedia_url = \"http://mu:mp@192.0.2.5:8971/media\"\n",
        );
        let (_, sections) = import_locked(&p).unwrap();
        let f = &sections.frigate;
        assert_eq!(f.url.as_deref(), Some("http://192.0.2.5:8971"));
        assert_eq!(f.user.as_deref(), Some("fu"));
        assert_eq!(f.password.as_deref(), Some("f@p"));
        assert_eq!(f.media_url.as_deref(), Some("http://192.0.2.5:8971/media"));
    }

    #[test]
    fn frigate_fields_win_over_userinfo() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "oikos.toml",
            "[frigate]\nurl = \"http://x:y@192.0.2.5:8971/\"\nuser = \"fu\"\npassword = \"fp\"\n",
        );
        let (_, sections) = import_locked(&p).unwrap();
        let f = &sections.frigate;
        assert_eq!(f.url.as_deref(), Some("http://192.0.2.5:8971/"));
        assert_eq!(
            (f.user.as_deref(), f.password.as_deref()),
            (Some("fu"), Some("fp"))
        );
    }

    #[test]
    fn empty_city_with_coordinates_gives_empty_name() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "oikos.toml",
            "[flows]\nlatitude = 1.0\nlongitude = 2.0\ntimezone = \"UTC\"\n",
        );
        let (file, _) = import_locked(&p).unwrap();
        let loc = file.location.unwrap();
        assert_eq!(loc.city, "");
        assert_eq!(loc.country, None);
    }

    #[test]
    fn reads_the_key_files() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "oikos.toml", FULL);
        write(dir.path(), "secrets/firebase.json", "{\"k\":1}");
        write(dir.path(), "secrets/solcast.key", "sk-test\n");
        let (_, sections) = import_locked(&p).unwrap();
        assert_eq!(sections.push.service_account.as_deref(), Some("{\"k\":1}"));
        assert_eq!(sections.solcast.api_key.as_deref(), Some("sk-test"));
    }

    #[test]
    fn missing_key_files_leave_the_fields_unset() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "oikos.toml", FULL);
        let (_, sections) = import_locked(&p).unwrap();
        assert_eq!(sections.push.service_account, None);
        assert_eq!(sections.solcast.api_key, None);
    }

    #[test]
    fn invalid_toml_is_refused_with_todays_message() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "oikos.toml",
            "[climate]\nenabled = true\nprefix = \"#\"\n",
        );
        let err = import_locked(&p).unwrap_err().to_string();
        assert!(err.contains("[climate] prefix"), "{err}");
    }

    #[test]
    fn candidate_ignores_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            toml_candidate(dir.path(), Some(&dir.path().join("nope.toml"))),
            None
        );
        assert_eq!(in_cwd(|_| toml_candidate(dir.path(), None)), None);
    }

    #[test]
    fn candidate_prefers_the_flag_then_the_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let data = write(dir.path(), "oikos.toml", "");
        let flag = write(dir.path(), "other.toml", "");
        assert_eq!(toml_candidate(dir.path(), Some(&flag)), Some(flag));
        assert_eq!(toml_candidate(dir.path(), None), Some(data));
    }

    /// Runs `f` with the working directory set to a fresh folder (figment's jail, which also
    /// serializes the tests that change it).
    #[allow(clippy::result_large_err)]
    fn in_cwd<T>(f: impl FnOnce(&Path) -> T) -> T {
        let mut out = None;
        figment::Jail::expect_with(|jail| {
            out = Some(f(jail.directory()));
            Ok(())
        });
        out.unwrap()
    }

    #[test]
    fn candidate_falls_back_to_the_working_directory() {
        let data = tempfile::tempdir().unwrap();
        // Compared inside the jail: its folder is gone once it ends.
        let (got, expected) = in_cwd(|cwd| {
            write(cwd, "oikos.toml", "");
            (
                toml_candidate(data.path(), None).map(|p| p.canonicalize().unwrap()),
                cwd.join("oikos.toml").canonicalize().unwrap(),
            )
        });
        assert_eq!(got, Some(expected));
    }

    #[test]
    fn candidate_order_is_flag_then_data_dir_then_working_directory() {
        let data = tempfile::tempdir().unwrap();
        let in_data = write(data.path(), "oikos.toml", "");
        let flag = write(data.path(), "other.toml", "");
        let (with_flag, with_data, missing_flag) = in_cwd(|cwd| {
            write(cwd, "oikos.toml", "");
            (
                toml_candidate(data.path(), Some(&flag)),
                toml_candidate(data.path(), None),
                // A flag names the file: the working directory is only today's default.
                toml_candidate(
                    tempfile::tempdir().unwrap().path(),
                    Some(&data.path().join("nope.toml")),
                ),
            )
        });
        assert_eq!(with_flag, Some(flag));
        assert_eq!(with_data, Some(in_data));
        assert_eq!(missing_flag, None);
    }

    #[test]
    fn no_file_anywhere_is_none() {
        let data = tempfile::tempdir().unwrap();
        assert_eq!(in_cwd(|_| toml_candidate(data.path(), None)), None);
    }

    #[test]
    fn timezone_only_is_kept_without_a_location() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "oikos.toml",
            "[flows]\ntimezone = \"Europe/Paris\"\n",
        );
        let (file, sections) = import_locked(&p).unwrap();
        assert!(file.location.is_none());
        assert_eq!(file.timezone.as_deref(), Some("Europe/Paris"));
        let cfg = assemble(&file, &sections);
        assert_eq!(cfg.flows.timezone.as_deref(), Some("Europe/Paris"));
    }

    #[test]
    fn weather_only_coordinates_build_the_location() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "oikos.toml",
            "[flows]\ntimezone = \"Europe/Paris\"\n[weather]\ncity = \"Lyon\"\nlatitude = 45.76\nlongitude = 4.84\n",
        );
        let (file, _) = import_locked(&p).unwrap();
        let loc = file.location.unwrap();
        assert_eq!((loc.latitude, loc.longitude), (45.76, 4.84));
        assert_eq!(loc.city, "Lyon");
        assert_eq!(loc.timezone, "Europe/Paris");
        assert!(file.timezone.is_none() && file.pending_city.is_none());
    }

    const EVERYTHING: &str = r#"
[mqtt]
host = "192.0.2.1"
port = 1884
user = "u"
password = "pw"
client_id = "cid"
send_ha_birth = false
ha_status_topic = "ha/status"
[topics]
discovery_prefixes = ["oikos", "homeassistant"]
ui_prefix = "x/ui"
z2m_base = "z2m"
automation_prefix = "x/auto"
[http]
bind = "127.0.0.1:9000"
[log]
level = "debug"
json = true
[commands]
confirm_timeout_ms = 1234
subscription_debounce_ms = 77
[automation]
runs_kept = 5
[watchdog.gateway]
enabled = true
prefix = "gw/health"
[watchdog.acl]
enabled = true
source = "file"
log_file = "/var/log/mosquitto.log"
cooldown = 99
[groups]
enabled = true
[alarm]
prefix = "x/alarm"
[history]
url = "postgres://u:pw@db.example.com/oikos"
buffer_rows = 42
[flows]
timezone = "Europe/Paris"
latitude = 45.76
longitude = 4.84
[weather]
enabled = true
refresh_minutes = 30
city = "Lyon"
country = "FR"
[solcast]
enabled = true
sites = ["site-1", "site_2"]
daily_limit = 20
api_key = "sk-inline"
[frigate]
url = "http://192.0.2.5:5000"
user = "fu"
password = "fp"
connect_timeout_ms = 1111
media_url = "http://192.0.2.5:8971"
bridge = true
labels = ["person"]
cameras = ["door"]
zones = ["yard"]
cooldown = 12
severity = "alert"
ttl = 100
topic_prefix = "fr"
live = true
[push]
prefix = "p/push"
legacy_prefix = "p-legacy"
retry_interval = 11
max_retries = 3
history_size = 50
history_days = 2
[climate]
enabled = true
prefix = "x/climate"
"#;

    #[test]
    fn assemble_of_the_import_equals_load() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "oikos.toml", EVERYTHING);
        write(dir.path(), "secrets/firebase.json", "{\"k\":1}");
        let mut expected = load_locked(&p).unwrap();
        let (file, sections) = import_locked(&p).unwrap();
        let got = assemble(&file, &sections);
        // What assemble leaves unset on purpose, and what the import inlines.
        expected.push.credentials = None;
        expected.push.service_account = Some("{\"k\":1}".into());
        expected.solcast.api_key_file = None;
        // The location also feeds the weather coordinates.
        expected.weather.latitude = Some(45.76);
        expected.weather.longitude = Some(4.84);
        assert_eq!(got, expected);
        // Guard the test itself: it really sets non-default values.
        assert_ne!(got.watchdog, Config::default().watchdog);
        assert_ne!(got.frigate, Config::default().frigate);
        assert_ne!(got.push.history_days, Config::default().push.history_days);
    }
}
