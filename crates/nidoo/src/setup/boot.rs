//! Deciding what the hub is at start-up: setup mode, or the whole hub with its settings read
//! from the database (or the cached copy of it while the database is down).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use nidoo_api::Location;
use tokio::task::JoinHandle;

use crate::config::Config;
use crate::history::db::Db;
use crate::history::writer::{INITIAL_BACKOFF, MAX_BACKOFF};
use crate::setup::import::{cwd_toml, import_toml, toml_candidate};
use crate::setup::restart::schedule_restart;
use crate::setup::sections::{Sections, assemble};
use crate::setup::store::{load_stored, read_cache, save_sections, write_cache};
use crate::setup::{HubFile, Mode, RestartHook, SetupCtx};
use crate::weather::open_meteo::{self, Endpoints, FetchError};
use crate::weather::poller::{GEOCODE_BACKOFF_MAX, GEOCODE_BACKOFF_START};

/// How long the first start waits for the database before it falls back to the cached copy.
const DB_WAIT: Duration = Duration::from_secs(5);
/// The pause before the hub restarts into a location it just found.
const RESTART_DELAY: Duration = Duration::from_secs(1);
/// The line that tells the user the old file is spent. Its wording is part of the contract.
pub const IMPORTED_LINE: &str =
    "oikos.toml imported into the database; it is no longer read and can be deleted";

pub enum Boot {
    Setup(Arc<SetupCtx>),
    Run {
        cfg: Box<Config>,
        ctx: Arc<SetupCtx>,
    },
}

/// Reads `hub.json` (importing `oikos.toml` the first time) and decides how the hub starts.
/// A `hub.json` that cannot be read or parsed is an error naming the file: starting in setup
/// mode would look as if the house had been wiped.
pub async fn resolve(data_dir: &Path, config_flag: Option<&Path>) -> anyhow::Result<Boot> {
    let mut imported: Option<Sections> = None;
    let file = match HubFile::load(data_dir)? {
        Some(file) => file,
        None => match toml_candidate(data_dir, config_flag) {
            Some(path) => {
                tracing::info!(path = %path.display(), "importing the existing config");
                let (file, sections) = import_toml(&path)
                    .with_context(|| format!("cannot import {}", path.display()))?;
                // The cache first, hub.json last: hub.json is the commit marker. A crash or a
                // full disk in between leaves no hub.json, so the next boot imports again.
                write_cache(data_dir, &sections).context("cannot write the settings cache")?;
                file.save(data_dir).with_context(|| {
                    format!("cannot write {}", HubFile::path(data_dir).display())
                })?;
                imported = Some(sections);
                file
            }
            None => HubFile::default(),
        },
    };
    let file = file
        .with_bootstrap_env()
        .map_err(|e| anyhow::anyhow!("invalid OIKOS_* override: {e}"))?;

    let restart = RestartHook::Exec;
    let Some(url) = file.database_url.clone().filter(|_| file.is_complete()) else {
        if imported.is_none()
            && let Some(path) = cwd_toml()
        {
            tracing::warn!(
                path = %path.display(),
                "setup mode: this oikos.toml is not read; the hub reads hub.json in its data \
                 folder, and imports an oikos.toml only when there is no hub.json yet"
            );
        }
        let sections = imported
            .or_else(|| read_cache(data_dir))
            .unwrap_or_default();
        let ctx = SetupCtx::new(
            data_dir.to_path_buf(),
            Mode::Setup,
            file,
            None,
            sections,
            restart,
        );
        return Ok(Boot::Setup(Arc::new(ctx)));
    };

    let db = match Db::connect_lazy(&url) {
        Ok(db) => Some(db),
        Err(error) => {
            tracing::warn!(%error, "settings: the database address is not usable");
            None
        }
    };
    // Only settings this boot imported, or the cache of an earlier import, are worth copying
    // into the table. With neither there is nothing to import: the table (or the cache) rules.
    let to_copy = if file.settings_imported {
        None
    } else {
        imported.or_else(|| read_cache(data_dir))
    };
    let mark_imported = !file.settings_imported && to_copy.is_none();
    let sections = match &to_copy {
        Some(sections) => sections.clone(),
        None => load_or_cached(db.as_ref(), data_dir).await,
    };

    let cfg = assemble(&file, &sections);
    let ctx = Arc::new(SetupCtx::new(
        data_dir.to_path_buf(),
        Mode::Run,
        file,
        db,
        sections,
        restart,
    ));
    if to_copy.is_some() {
        spawn_import_copy(Arc::clone(&ctx));
    } else if mark_imported
        && let Err(error) = ctx.update_file(|f| f.settings_imported = true).await
    {
        tracing::warn!(%error, "settings: cannot record that there was nothing to import");
    }
    Ok(Boot::Run {
        cfg: Box::new(cfg),
        ctx,
    })
}

/// The settings table within [`DB_WAIT`] (refreshing the cache, or refilling an empty table
/// from it, see [`from_table`]), else the cached copy, else the defaults. A database that
/// answers badly is told apart from one that does not answer.
async fn load_or_cached(db: Option<&Db>, data_dir: &Path) -> Sections {
    if let Some(db) = db {
        match tokio::time::timeout(DB_WAIT, load_from_db(db)).await {
            Ok(Ok(stored)) => {
                let (sections, refill) = from_table(stored, read_cache(data_dir));
                if refill {
                    tracing::warn!(
                        "settings: the database holds no settings; restoring them from the cached copy"
                    );
                    // The import copy's save step. A failure leaves the table empty and the
                    // cache as it is, so the next boot tries again.
                    match tokio::time::timeout(DB_WAIT, save_sections(db, &sections)).await {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => {
                            tracing::warn!(%error, "settings: cannot restore the database settings")
                        }
                        Err(_) => tracing::warn!(
                            "settings: cannot restore the database settings (no answer)"
                        ),
                    }
                } else if let Err(error) = write_cache(data_dir, &sections) {
                    tracing::warn!(%error, "settings: cannot refresh the cached copy");
                }
                return sections;
            }
            Ok(Err(error)) => {
                tracing::warn!(%error, "settings: database unreachable, starting from the cached copy");
            }
            Err(_) => {
                tracing::warn!(
                    "settings: database unreachable (no answer in {}s), starting from the cached copy",
                    DB_WAIT.as_secs()
                );
            }
        }
    }
    read_cache(data_dir).unwrap_or_else(|| {
        tracing::warn!("settings: no cached copy either; starting from the defaults");
        Sections::default()
    })
}

/// The settings a boot takes once the database answered, and whether they must be written
/// back into the table. A table with rows rules (a missing row is that section's defaults). An
/// empty table is a new database: the cached copy is taken and put back into it, so pointing
/// the hub at a new database never wipes the settings. With no cache either: the defaults.
fn from_table(stored: Option<Sections>, cached: Option<Sections>) -> (Sections, bool) {
    match (stored, cached) {
        (Some(stored), _) => (stored, false),
        (None, Some(cached)) => (cached, true),
        (None, None) => (Sections::default(), false),
    }
}

async fn load_from_db(db: &Db) -> anyhow::Result<Option<Sections>> {
    db.migrate().await?;
    Ok(load_stored(db).await?)
}

/// Copies the sections into the `settings` table once the database answers, then marks the
/// import done in `hub.json`. Retries forever with the history writer's backoff: the house
/// runs from the cached copy meanwhile.
pub fn spawn_import_copy(ctx: Arc<SetupCtx>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let Some(db) = ctx.db.as_ref() else { return };
        let mut backoff = INITIAL_BACKOFF;
        loop {
            // The lock is held from the snapshot to the save (and released before any
            // sleep), so a `PUT /settings` can never be overwritten by a stale copy.
            let copied = async {
                db.migrate().await.map_err(anyhow::Error::from)?;
                let sections = ctx.sections.lock().await;
                save_sections(db, &sections)
                    .await
                    .map_err(anyhow::Error::from)
            }
            .await;
            match copied {
                Ok(()) => break,
                Err(error) => {
                    tracing::warn!(%error, retry_in = ?backoff, "settings: import copy waiting for the database");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
            }
        }
        match ctx.update_file(|f| f.settings_imported = true).await {
            Ok(()) => tracing::info!("{IMPORTED_LINE}"),
            Err(error) => tracing::error!(%error, "settings: cannot record the import in hub.json"),
        }
    })
}

/// Geocodes `pending_city` into `location` (the setup's weather place), then restarts so the
/// hub boots with it. Until then the old fallback place applies. A city that matches nothing
/// is kept (weather still uses it) with a warning, and looked up again at the next boot.
pub fn spawn_location_lookup(ctx: Arc<SetupCtx>, endpoints: Endpoints) -> JoinHandle<()> {
    tokio::spawn(async move {
        let Some((city, country)) = ctx.file.lock().await.pending_city.clone() else {
            return;
        };
        let client = match open_meteo::client() {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!(%error, "settings: cannot look the location up");
                return;
            }
        };
        let mut backoff = GEOCODE_BACKOFF_START;
        let place = loop {
            match open_meteo::geocode_many(&client, &endpoints, &city, country.as_deref(), 1).await
            {
                Ok(places) => break places.into_iter().next(),
                Err(e @ (FetchError::Http(_) | FetchError::Decode(_))) => {
                    tracing::warn!(error = %e, retry_in = ?backoff, "settings: location lookup failed");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(GEOCODE_BACKOFF_MAX);
                }
            }
        };
        let Some(place) = place else {
            // The pending city is what weather falls back on while there is no location, so
            // it stays: the poller geocodes it itself and the next boot looks it up again.
            tracing::warn!(
                city,
                "settings: the imported city was not found; keeping it for the weather poller"
            );
            return;
        };
        let mut applied = false;
        let saved = ctx
            .update_file(|f| {
                // A place the user chose meanwhile (which clears the pending city) wins.
                if f.pending_city.is_none() {
                    return;
                }
                applied = true;
                // An explicit imported time zone wins over the geocoded one.
                let timezone = f
                    .timezone
                    .take()
                    .or(place.timezone.clone())
                    .or_else(|| iana_time_zone::get_timezone().ok())
                    .unwrap_or_else(|| "UTC".to_owned());
                f.location = Some(Location {
                    city: city.clone(),
                    country: country.clone(),
                    latitude: place.latitude,
                    longitude: place.longitude,
                    timezone,
                });
                f.pending_city = None;
            })
            .await;
        match saved {
            Ok(()) if !applied => {
                tracing::info!(
                    city,
                    "settings: a location was set meanwhile; lookup dropped"
                );
            }
            Ok(()) => {
                tracing::info!(city, "settings: location found");
                schedule_restart(&ctx, RESTART_DELAY);
            }
            Err(error) => tracing::error!(%error, "settings: cannot save hub.json"),
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::result_large_err)] // figment::Jail closures return figment::Error

    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::Router;
    use axum::routing::get;
    use tokio::net::TcpListener;

    use super::*;

    /// `resolve` reads `OIKOS_*` and `Config::load` does too, so each call runs inside figment's
    /// lock on a runtime of its own.
    fn resolve_locked(dir: &Path, flag: Option<&Path>) -> anyhow::Result<Boot> {
        let mut out = None;
        figment::Jail::expect_with(|_| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            out = Some(rt.block_on(resolve(dir, flag)));
            Ok(())
        });
        out.unwrap()
    }

    fn write(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    const DOWN: &str = "postgres://u:p@127.0.0.1:1/oikos";

    fn toml_with_history(url: &str) -> String {
        format!(
            "[mqtt]\nhost = \"192.0.2.1\"\n[history]\nurl = \"{url}\"\n[weather]\nenabled = true\ncity = \"Lyon\"\n"
        )
    }

    #[test]
    fn empty_dir_is_setup_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let boot = resolve_locked(tmp.path(), None).unwrap();
        let Boot::Setup(ctx) = boot else {
            panic!("expected setup mode")
        };
        assert_eq!(ctx.mode, Mode::Setup);
        assert!(ctx.db.is_none());
    }

    #[test]
    fn toml_without_history_is_setup_mode_with_mqtt_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let toml = write(tmp.path(), "oikos.toml", "[mqtt]\nhost = \"192.0.2.1\"\n");
        let Boot::Setup(ctx) = resolve_locked(tmp.path(), Some(&toml)).unwrap() else {
            panic!("expected setup mode")
        };
        let file = ctx.file.try_lock().unwrap();
        assert_eq!(file.mqtt.as_ref().unwrap().host, "192.0.2.1");
        assert!(file.database_url.is_none());
        assert!(tmp.path().join("hub.json").exists());
    }

    #[test]
    fn import_with_unreachable_db_boots_run_from_the_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let toml = write(tmp.path(), "oikos.toml", &toml_with_history(DOWN));
        let Boot::Run { cfg, ctx } = resolve_locked(tmp.path(), Some(&toml)).unwrap() else {
            panic!("expected normal mode")
        };
        assert!(cfg.weather.enabled);
        assert_eq!(cfg.mqtt.host, "192.0.2.1");
        assert!(tmp.path().join("hub.json").exists());
        assert!(tmp.path().join("settings-cache.json").exists());
        // The copy has not happened: the flag stays off until the database takes the rows.
        assert!(!ctx.file.try_lock().unwrap().settings_imported);
    }

    #[test]
    fn a_city_only_weather_import_boots_a_config_that_validates() {
        // `toml_with_history` turns weather on with a city and no coordinates (the documented
        // setup): main validates the config before anything starts.
        let tmp = tempfile::tempdir().unwrap();
        let toml = write(tmp.path(), "oikos.toml", &toml_with_history(DOWN));
        let Boot::Run { cfg, ctx } = resolve_locked(tmp.path(), Some(&toml)).unwrap() else {
            panic!("expected normal mode")
        };
        assert_eq!(cfg.validate_all(), Ok(()));
        assert_eq!(cfg.weather.city, "Lyon");
        assert!(ctx.file.try_lock().unwrap().pending_city.is_some());
    }

    #[test]
    fn an_empty_table_boots_from_the_cache_and_refills_it() {
        let mut cached = Sections::default();
        cached.weather.enabled = true;
        let (got, refill) = from_table(None, Some(cached.clone()));
        assert_eq!(got, cached);
        assert!(refill);
        // No cache either: the defaults, nothing to put back.
        let (got, refill) = from_table(None, None);
        assert_eq!(got, Sections::default());
        assert!(!refill);
        // Rows win over the cache, even rows at their defaults.
        let (got, refill) = from_table(Some(Sections::default()), Some(cached));
        assert_eq!(got, Sections::default());
        assert!(!refill);
    }

    #[test]
    fn complete_file_and_db_down_uses_the_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let file = HubFile {
            mqtt: Some(crate::config::MqttConfig::default()),
            database_url: Some(DOWN.into()),
            settings_imported: true,
            finished: true,
            ..HubFile::default()
        };
        file.save(tmp.path()).unwrap();
        let mut cached = Sections::default();
        cached.weather.enabled = true;
        write_cache(tmp.path(), &cached).unwrap();
        let Boot::Run { cfg, ctx } = resolve_locked(tmp.path(), None).unwrap() else {
            panic!("expected normal mode")
        };
        assert!(cfg.weather.enabled);
        assert_eq!(ctx.mode, Mode::Run);
    }

    #[test]
    fn complete_file_db_down_and_no_cache_uses_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let file = HubFile {
            mqtt: Some(crate::config::MqttConfig::default()),
            database_url: Some(DOWN.into()),
            settings_imported: true,
            ..HubFile::default()
        };
        file.save(tmp.path()).unwrap();
        let Boot::Run { cfg, .. } = resolve_locked(tmp.path(), None).unwrap() else {
            panic!("expected normal mode")
        };
        assert_eq!(cfg.weather.enabled, Sections::default().weather.enabled);
    }

    #[test]
    fn corrupt_hub_file_is_an_error_naming_it() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "hub.json", "{\"mqtt\":");
        let err = match resolve_locked(tmp.path(), None) {
            Ok(_) => panic!("a corrupt hub.json must not boot"),
            Err(e) => format!("{e:#}"),
        };
        assert!(err.contains("hub.json"), "{err}");
    }

    #[test]
    fn an_existing_hub_file_is_not_replaced_by_a_toml() {
        let tmp = tempfile::tempdir().unwrap();
        HubFile::default().save(tmp.path()).unwrap();
        let toml = write(tmp.path(), "oikos.toml", &toml_with_history(DOWN));
        let Boot::Setup(ctx) = resolve_locked(tmp.path(), Some(&toml)).unwrap() else {
            panic!("the existing, empty hub.json decides")
        };
        assert!(ctx.file.try_lock().unwrap().mqtt.is_none());
    }

    #[test]
    fn env_alone_completes_the_bootstrap() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = None;
        figment::Jail::expect_with(|jail| {
            jail.set_env("OIKOS_MQTT__HOST", "192.0.2.7");
            jail.set_env("OIKOS_HISTORY__URL", DOWN);
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            out = Some(rt.block_on(resolve(tmp.path(), None)));
            Ok(())
        });
        let Boot::Run { cfg, .. } = out.unwrap().unwrap() else {
            panic!("expected normal mode")
        };
        assert_eq!(cfg.mqtt.host, "192.0.2.7");
    }

    #[test]
    fn env_only_boot_marks_the_flag_without_a_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = None;
        figment::Jail::expect_with(|jail| {
            jail.set_env("OIKOS_MQTT__HOST", "192.0.2.7");
            jail.set_env("OIKOS_HISTORY__URL", DOWN);
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            out = Some(rt.block_on(resolve(tmp.path(), None)));
            Ok(())
        });
        let Boot::Run { ctx, .. } = out.unwrap().unwrap() else {
            panic!("expected normal mode")
        };
        // A copy against a down database could never have set the flag.
        assert!(ctx.file.try_lock().unwrap().settings_imported);
        assert!(
            HubFile::load(tmp.path())
                .unwrap()
                .unwrap()
                .settings_imported
        );
    }

    #[test]
    fn complete_file_without_cache_or_import_marks_the_flag_without_a_copy() {
        let tmp = tempfile::tempdir().unwrap();
        HubFile {
            mqtt: Some(crate::config::MqttConfig::default()),
            database_url: Some(DOWN.into()),
            ..HubFile::default()
        }
        .save(tmp.path())
        .unwrap();
        let Boot::Run { ctx, .. } = resolve_locked(tmp.path(), None).unwrap() else {
            panic!("expected normal mode")
        };
        assert!(ctx.file.try_lock().unwrap().settings_imported);
        assert!(
            HubFile::load(tmp.path())
                .unwrap()
                .unwrap()
                .settings_imported
        );
    }

    #[test]
    fn an_uncopied_cache_still_gets_its_copy() {
        let tmp = tempfile::tempdir().unwrap();
        HubFile {
            mqtt: Some(crate::config::MqttConfig::default()),
            database_url: Some(DOWN.into()),
            ..HubFile::default()
        }
        .save(tmp.path())
        .unwrap();
        let mut cached = Sections::default();
        cached.weather.enabled = true;
        write_cache(tmp.path(), &cached).unwrap();
        let Boot::Run { cfg, ctx } = resolve_locked(tmp.path(), None).unwrap() else {
            panic!("expected normal mode")
        };
        assert!(cfg.weather.enabled);
        // The copy is waiting for the database: the flag stays off.
        assert!(!ctx.file.try_lock().unwrap().settings_imported);
    }

    #[test]
    fn a_failed_cache_write_leaves_no_hub_json() {
        let tmp = tempfile::tempdir().unwrap();
        // A directory where the cache file goes makes its write fail.
        std::fs::create_dir(tmp.path().join("settings-cache.json")).unwrap();
        let toml = write(tmp.path(), "oikos.toml", &toml_with_history(DOWN));
        assert!(resolve_locked(tmp.path(), Some(&toml)).is_err());
        assert!(!tmp.path().join("hub.json").exists());
    }

    #[test]
    fn setup_mode_keeps_the_imported_sections() {
        let tmp = tempfile::tempdir().unwrap();
        let toml = write(
            tmp.path(),
            "oikos.toml",
            "[mqtt]\nhost = \"192.0.2.1\"\n[weather]\nenabled = true\ncity = \"Lyon\"\n",
        );
        let Boot::Setup(ctx) = resolve_locked(tmp.path(), Some(&toml)).unwrap() else {
            panic!("expected setup mode")
        };
        assert!(ctx.sections.try_lock().unwrap().weather.enabled);
    }

    fn ctx_with(file: HubFile, dir: &Path) -> (Arc<SetupCtx>, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let ctx = Arc::new(SetupCtx::new(
            dir.to_path_buf(),
            Mode::Run,
            file,
            None,
            Sections::default(),
            RestartHook::Record(Arc::clone(&count)),
        ));
        (ctx, count)
    }

    async fn geocoder(body: &'static str) -> Endpoints {
        let app = Router::new().route("/geocode", get(move || async move { body }));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Endpoints {
            geocoding: format!("{base}/geocode"),
            forecast: format!("{base}/forecast"),
        }
    }

    const LYON: &str = r#"{"results":[{"name":"Lyon","latitude":45.75,"longitude":4.85,
        "country_code":"FR","admin1":"Auvergne-Rhone-Alpes","timezone":"Europe/Paris"}]}"#;

    async fn wait_for(mut done: impl FnMut() -> bool) {
        for _ in 0..200 {
            if done() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("timed out");
    }

    fn pending(tz: Option<&str>) -> HubFile {
        HubFile {
            pending_city: Some(("Lyon".into(), Some("FR".into()))),
            timezone: tz.map(Into::into),
            ..HubFile::default()
        }
    }

    #[tokio::test]
    async fn pending_city_is_geocoded_then_restarts() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, count) = ctx_with(pending(None), tmp.path());
        let ep = geocoder(LYON).await;
        spawn_location_lookup(Arc::clone(&ctx), ep);
        wait_for(|| count.load(Ordering::SeqCst) == 1).await;
        let file = ctx.file.lock().await.clone();
        let loc = file.location.clone().unwrap();
        assert_eq!(loc.city, "Lyon");
        assert_eq!(loc.country.as_deref(), Some("FR"));
        assert_eq!((loc.latitude, loc.longitude), (45.75, 4.85));
        assert_eq!(loc.timezone, "Europe/Paris");
        assert!(file.pending_city.is_none() && file.timezone.is_none());
        // Saved to disk too.
        assert_eq!(HubFile::load(tmp.path()).unwrap().unwrap(), file);
    }

    #[tokio::test]
    async fn an_imported_timezone_beats_the_geocoded_one() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, count) = ctx_with(pending(Some("America/New_York")), tmp.path());
        let ep = geocoder(LYON).await;
        spawn_location_lookup(Arc::clone(&ctx), ep);
        wait_for(|| count.load(Ordering::SeqCst) == 1).await;
        let file = ctx.file.lock().await.clone();
        assert_eq!(file.location.unwrap().timezone, "America/New_York");
        assert!(file.timezone.is_none());
    }

    #[tokio::test]
    async fn a_result_without_a_timezone_falls_back_to_a_real_zone() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, count) = ctx_with(pending(None), tmp.path());
        let ep =
            geocoder(r#"{"results":[{"name":"Lyon","latitude":45.75,"longitude":4.85}]}"#).await;
        spawn_location_lookup(Arc::clone(&ctx), ep);
        wait_for(|| count.load(Ordering::SeqCst) == 1).await;
        let tz = ctx.file.lock().await.location.clone().unwrap().timezone;
        assert!(tz.parse::<chrono_tz::Tz>().is_ok(), "{tz}");
    }

    #[tokio::test]
    async fn no_match_keeps_the_pending_city_without_restarting() {
        let tmp = tempfile::tempdir().unwrap();
        pending(None).save(tmp.path()).unwrap();
        let (ctx, count) = ctx_with(pending(None), tmp.path());
        let ep = geocoder("{}").await;
        let task = spawn_location_lookup(Arc::clone(&ctx), ep);
        task.await.unwrap();
        let file = ctx.file.lock().await.clone();
        assert!(file.pending_city.is_some() && file.location.is_none());
        assert_eq!(count.load(Ordering::SeqCst), 0);
        let disk = HubFile::load(tmp.path()).unwrap().unwrap();
        assert!(disk.pending_city.is_some() && disk.location.is_none());
        // The kept city is what lets weather boot: the config still validates.
        let mut sections = Sections::default();
        sections.weather.enabled = true;
        assert_eq!(
            crate::setup::sections::assemble(&file, &sections).validate_all(),
            Ok(())
        );
    }

    #[tokio::test]
    async fn a_location_set_during_the_lookup_is_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, count) = ctx_with(pending(None), tmp.path());
        // The geocoder answers only once the user has picked a place.
        let go = Arc::new(tokio::sync::Notify::new());
        let asked = Arc::new(tokio::sync::Notify::new());
        let (wait, ask) = (Arc::clone(&go), Arc::clone(&asked));
        let app = Router::new().route(
            "/geocode",
            get(move || {
                let (wait, ask) = (Arc::clone(&wait), Arc::clone(&ask));
                async move {
                    ask.notify_one();
                    wait.notified().await;
                    LYON
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let ep = Endpoints {
            geocoding: format!("{base}/geocode"),
            forecast: format!("{base}/forecast"),
        };
        let task = spawn_location_lookup(Arc::clone(&ctx), ep);
        // The lookup has read the pending city and is waiting for the answer.
        asked.notified().await;
        let chosen = Location {
            city: "Paris".into(),
            country: Some("FR".into()),
            latitude: 48.85,
            longitude: 2.35,
            timezone: "Europe/Paris".into(),
        };
        let picked = chosen.clone();
        ctx.update_file(move |f| {
            f.location = Some(picked);
            f.pending_city = None;
        })
        .await
        .unwrap();
        go.notify_one();
        task.await.unwrap();
        assert_eq!(ctx.file.lock().await.location, Some(chosen.clone()));
        assert_eq!(
            HubFile::load(tmp.path()).unwrap().unwrap().location,
            Some(chosen)
        );
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "no restart for a stale lookup"
        );
    }

    #[tokio::test]
    async fn concurrent_file_updates_do_not_collide() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, _) = ctx_with(HubFile::default(), tmp.path());
        let mut tasks = Vec::new();
        for i in 0..20 {
            let ctx = Arc::clone(&ctx);
            tasks.push(tokio::spawn(async move {
                ctx.update_file(|f| f.z2m_base = Some(format!("base{i}")))
                    .await
            }));
        }
        for t in tasks {
            t.await.unwrap().unwrap();
        }
        let held = ctx.file.lock().await.clone();
        assert_eq!(HubFile::load(tmp.path()).unwrap().unwrap(), held);
    }
}
