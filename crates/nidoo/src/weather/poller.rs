//! The task that keeps the weather fresh: geocode the configured city once, then fetch the
//! forecast every `refresh_minutes` and hand it to the hub.

use std::sync::Arc;
use std::time::Duration;

use nidoo_api::ConnectionStatus;
use tokio::task::JoinHandle;

use super::is_expired;
use super::open_meteo::{self, Endpoints, FetchError, Place};
use crate::clock;
use crate::hub::Hub;

/// First wait after a failed geocoding; it doubles up to [`GEOCODE_BACKOFF_MAX`].
pub(crate) const GEOCODE_BACKOFF_START: Duration = Duration::from_secs(30);
pub(crate) const GEOCODE_BACKOFF_MAX: Duration = Duration::from_secs(15 * 60);

/// The shortest sleep between two fetches. `[weather] refresh_minutes >= 10` is enforced only
/// by `WeatherConfig::validate` at start; the poller takes whatever the hub holds, so tests
/// can set 0 and run in seconds.
const MIN_SLEEP: Duration = Duration::from_secs(1);

impl Hub {
    /// Starts the weather poller; `None` while `[weather] enabled` is off, in which case
    /// nothing ever leaves the LAN.
    pub fn start_weather(self: &Arc<Self>) -> Option<JoinHandle<()>> {
        self.weather_enabled()
            .then(|| spawn_with(self, Endpoints::default()))
    }
}

/// Spawns [`run`] against `endpoints` (tests point them at a stub). The city, country and
/// refresh period come from the hub's config.
pub fn spawn_with(hub: &Arc<Hub>, endpoints: Endpoints) -> JoinHandle<()> {
    tokio::spawn(run(Arc::clone(hub), endpoints))
}

/// The poller. Publishes the entity configs once the broker is online, resolves the city, then
/// loops until the task is aborted. A failure never ends it, except an unknown city.
pub async fn run(hub: Arc<Hub>, endpoints: Endpoints) {
    let client = match open_meteo::client() {
        Ok(client) => client,
        Err(e) => {
            tracing::error!(error = %e, "weather: cannot build the HTTP client");
            return;
        }
    };
    let mut connection = hub.connection();
    if connection
        .wait_for(|status| matches!(status, ConnectionStatus::Online))
        .await
        .is_err()
    {
        return;
    }
    // Once, from this task only, so it never interleaves with `apply_weather`.
    hub.publish_weather_configs();

    let city = hub.weather_city();
    let place = match hub.weather_coordinates() {
        // The house's location is known: no geocoding request.
        Some((latitude, longitude)) => Place {
            name: city,
            admin: None,
            country: hub.weather_country(),
            latitude,
            longitude,
            timezone: None,
        },
        None => {
            let Some(place) = resolve(&client, &endpoints, &city, hub.weather_country()).await
            else {
                return;
            };
            place
        }
    };

    let mut last_success: Option<u64> = None;
    let mut warned = false;
    loop {
        match open_meteo::forecast(&client, &endpoints, &place).await {
            Ok(forecast) => {
                let now = clock::now();
                hub.apply_weather(open_meteo::build_view(&forecast, &place, now));
                last_success = Some(now);
                warned = false;
            }
            Err(e) => {
                if !warned {
                    tracing::warn!(error = %e, "weather: fetching the forecast failed; keeping the last one");
                    warned = true;
                }
                if last_success.is_some_and(|at| is_expired(at, clock::now())) {
                    hub.set_weather_offline();
                }
            }
        }
        tokio::time::sleep(refresh_delay(
            hub.weather_refresh_minutes(),
            clock::random_roll(),
        ))
        .await;
    }
}

/// Geocodes `city`, retrying any failed request (network, HTTP status or an undecodable body)
/// with a doubling backoff. `None` when the
/// city does not exist: weather then stays unavailable.
async fn resolve(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    city: &str,
    country: Option<String>,
) -> Option<Place> {
    let mut backoff = GEOCODE_BACKOFF_START;
    loop {
        match open_meteo::geocode(client, endpoints, city, country.as_deref()).await {
            Ok(Some(place)) => {
                tracing::info!(
                    name = %place.name,
                    admin = place.admin.as_deref().unwrap_or(""),
                    country = place.country.as_deref().unwrap_or(""),
                    latitude = place.latitude,
                    longitude = place.longitude,
                    "weather: place found"
                );
                return Some(place);
            }
            Ok(None) => {
                tracing::error!(city, "weather: city not found");
                return None;
            }
            Err(e @ (FetchError::Http(_) | FetchError::Decode(_))) => {
                tracing::warn!(error = %e, retry_in = ?backoff, "weather: geocoding failed");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(GEOCODE_BACKOFF_MAX);
            }
        }
    }
}

/// `minutes` minutes, plus or minus 10 %, from a random `roll`; never below [`MIN_SLEEP`].
fn refresh_delay(minutes: u64, roll: u64) -> Duration {
    let base_ms = minutes.saturating_mul(60_000);
    let span = base_ms / 5;
    let delay = base_ms - base_ms / 10 + roll % (span + 1);
    Duration::from_millis(delay).max(MIN_SLEEP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_delay_is_ten_percent_around_the_period() {
        let low = refresh_delay(15, 0);
        let high = refresh_delay(15, u64::MAX);
        assert_eq!(low, Duration::from_secs(15 * 54));
        assert!(high <= Duration::from_secs(15 * 66));
        assert!(high > Duration::from_secs(15 * 60));
    }

    #[test]
    fn refresh_delay_has_a_one_second_floor() {
        assert_eq!(refresh_delay(0, 12345), MIN_SLEEP);
    }
}
