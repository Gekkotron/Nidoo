//! Deciding what the hub is at start-up: setup mode, or the whole hub with its settings read
//! from the database (or the cached copy of it while the database is down).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use nidoo_api::Location;
use tokio::task::JoinHandle;

use crate::config::Config;
use crate::history::db::Db;
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

pub enum Boot {
    Setup(Arc<SetupCtx>),
    /// A complete file with a restore pending: the database is open and migrated, and only the
    /// setup and restore screens are served.
    Restore(Arc<SetupCtx>),
    Run {
        cfg: Box<Config>,
        ctx: Arc<SetupCtx>,
    },
}

/// Reads `hub.json` and decides how the hub starts. A `hub.json` that cannot be read or parsed
/// is an error naming the file: starting in setup mode would look as if the house had been
/// wiped.
pub async fn resolve(data_dir: &Path) -> anyhow::Result<Boot> {
    let file = HubFile::load(data_dir)?
        .unwrap_or_default()
        .with_bootstrap_env()
        .map_err(|e| anyhow::anyhow!("invalid NIDOO_* override: {e}"))?;

    let restart = RestartHook::Exec;
    let Some(url) = file.database_url.clone().filter(|_| file.is_complete()) else {
        let sections = read_cache(data_dir).unwrap_or_default();
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
    if file.restore.is_some() {
        if let Some(db) = &db {
            match tokio::time::timeout(DB_WAIT, db.migrate()).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::warn!(%error, "restore: cannot migrate the database"),
                Err(_) => tracing::warn!("restore: cannot migrate the database (no answer)"),
            }
        }
        let sections = read_cache(data_dir).unwrap_or_default();
        let ctx = SetupCtx::new(
            data_dir.to_path_buf(),
            Mode::Restore,
            file,
            db,
            sections,
            restart,
        );
        return Ok(Boot::Restore(Arc::new(ctx)));
    }
    let sections = load_or_cached(db.as_ref(), data_dir).await;
    let cfg = assemble(&file, &sections);
    let ctx = Arc::new(SetupCtx::new(
        data_dir.to_path_buf(),
        Mode::Run,
        file,
        db,
        sections,
        restart,
    ));
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
                    // A failure leaves the table empty and the
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
mod tests;
