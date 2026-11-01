//! Sun position, sunrise/sunset/civil-twilight times and the "is it dark" test, from the NOAA
//! solar-position formulae. Pure computation over `chrono` values (no clock, no I/O), so the hub
//! engine and the web canvas share it.
//!
//! `sun_altitude` is the single evaluator: `sun_events` root-finds crossings of it and
//! `is_dark_at` compares it, so a trigger and an operand can never disagree about the boundary.

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};

/// Civil twilight: the sun's centre 6 degrees below the horizon (refraction included).
pub const CIVIL_ALTITUDE_DEG: f64 = -6.0;
/// Standard apparent sunrise/sunset threshold (atmospheric refraction and the solar radius
/// included).
pub const APPARENT_ALTITUDE_DEG: f64 = -0.833;

/// Sun events of one UTC day window; `None` where the sun never crosses the threshold
/// (polar night, midnight sun).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SunEvents {
    pub sunrise: Option<DateTime<Utc>>,
    pub sunset: Option<DateTime<Utc>>,
    pub dawn: Option<DateTime<Utc>>,
    pub dusk: Option<DateTime<Utc>>,
}

/// Geometric center altitude in degrees at `at`, for a position in degrees (north and east
/// positive); compare against the refraction-inclusive thresholds `-0.833` (apparent
/// sunrise/sunset, [`APPARENT_ALTITUDE_DEG`]) and `-6` (civil twilight, [`CIVIL_ALTITUDE_DEG`]).
pub fn sun_altitude(lat: f64, lon: f64, at: DateTime<Utc>) -> f64 {
    let jd = at.timestamp() as f64 / 86_400.0 + 2_440_587.5;
    let t = (jd - 2_451_545.0) / 36_525.0;

    let l0 = (280.46646 + t * (36_000.769_83 + t * 0.000_303_2)).rem_euclid(360.0);
    let m = 357.52911 + t * (35_999.050_29 - 0.000_153_7 * t);
    let m_rad = m.to_radians();
    let e = 0.016_708_634 - t * (0.000_042_037 + 0.000_000_126_7 * t);
    let c = m_rad.sin() * (1.914_602 - t * (0.004_817 + 0.000_014 * t))
        + (2.0 * m_rad).sin() * (0.019_993 - 0.000_101 * t)
        + (3.0 * m_rad).sin() * 0.000_289;
    let true_long = l0 + c;
    let omega = 125.04 - 1_934.136 * t;
    let app_long = true_long - 0.005_69 - 0.004_78 * omega.to_radians().sin();
    let mean_obliq =
        23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.000_59 - t * 0.001_813))) / 60.0) / 60.0;
    let obliq = (mean_obliq + 0.002_56 * omega.to_radians().cos()).to_radians();
    let decl = (obliq.sin() * app_long.to_radians().sin()).asin();

    let y = (obliq / 2.0).tan().powi(2);
    let l0_rad = l0.to_radians();
    let eq_time_min = 4.0
        * (y * (2.0 * l0_rad).sin() - 2.0 * e * m_rad.sin()
            + 4.0 * e * y * m_rad.sin() * (2.0 * l0_rad).cos()
            - 0.5 * y * y * (4.0 * l0_rad).sin()
            - 1.25 * e * e * (2.0 * m_rad).sin())
        .to_degrees();

    let utc_minutes = at.timestamp().rem_euclid(86_400) as f64 / 60.0;
    let true_solar_min = (utc_minutes + eq_time_min + 4.0 * lon).rem_euclid(1_440.0);
    let hour_angle = (true_solar_min / 4.0 - 180.0).to_radians();

    let lat_rad = lat.to_radians();
    let sin_alt = lat_rad.sin() * decl.sin() + lat_rad.cos() * decl.cos() * hour_angle.cos();
    sin_alt.clamp(-1.0, 1.0).asin().to_degrees()
}

/// Sunrise, sunset, dawn and dusk found as the first upward and downward crossings of
/// `sun_altitude` between 00:00 UTC of `date` and 00:00 UTC of the next day.
/// Rising events return the first second at or above the threshold, setting events the first
/// second at or below it, so `is_dark_at` agrees with `dusk` and `dawn` to the second.
pub fn sun_events(lat: f64, lon: f64, date: NaiveDate) -> SunEvents {
    let start = Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).expect("midnight is valid"));
    let (sunrise, sunset) = crossings(lat, lon, start, APPARENT_ALTITUDE_DEG);
    let (dawn, dusk) = crossings(lat, lon, start, CIVIL_ALTITUDE_DEG);
    SunEvents {
        sunrise,
        sunset,
        dawn,
        dusk,
    }
}

/// Whether the sun's centre is at or below civil twilight at `at`.
pub fn is_dark_at(lat: f64, lon: f64, at: DateTime<Utc>) -> bool {
    sun_altitude(lat, lon, at) <= CIVIL_ALTITUDE_DEG
}

/// Whether it is dark at `at` with civil dusk moved by `offset_minutes` and civil dawn by the
/// opposite: a negative offset starts the dark earlier in the evening and ends it later in the
/// morning, a positive one shortens it at both ends. 0 is `is_dark_at`.
pub fn is_dark_with_offset(lat: f64, lon: f64, at: DateTime<Utc>, offset_minutes: i32) -> bool {
    let shift = Duration::minutes(i64::from(offset_minutes.unsigned_abs()));
    let (sooner, later) = (
        is_dark_at(lat, lon, at + shift),
        is_dark_at(lat, lon, at - shift),
    );
    if offset_minutes < 0 {
        sooner || later
    } else {
        sooner && later
    }
}

/// First upward and first downward crossing of `threshold` in the 24 hours from `start`.
fn crossings(
    lat: f64,
    lon: f64,
    start: DateTime<Utc>,
    threshold: f64,
) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
    let mut rising = None;
    let mut setting = None;
    let mut prev_at = start;
    let mut prev_alt = sun_altitude(lat, lon, prev_at);
    for minute in 1..=1_440 {
        let at = start + Duration::minutes(minute);
        let alt = sun_altitude(lat, lon, at);
        if rising.is_none() && prev_alt < threshold && alt >= threshold {
            rising = Some(bisect(lat, lon, prev_at, at, threshold, true));
        }
        if setting.is_none() && prev_alt > threshold && alt <= threshold {
            setting = Some(bisect(lat, lon, prev_at, at, threshold, false));
        }
        if rising.is_some() && setting.is_some() {
            break;
        }
        prev_at = at;
        prev_alt = alt;
    }
    (rising, setting)
}

/// Narrows a straddling pair to one second and returns the later end, which sits on the far
/// side of the threshold: at or above it when `upward`, at or below it otherwise.
fn bisect(
    lat: f64,
    lon: f64,
    mut lo: DateTime<Utc>,
    mut hi: DateTime<Utc>,
    threshold: f64,
    upward: bool,
) -> DateTime<Utc> {
    while (hi - lo).num_seconds() > 1 {
        let mid = lo + Duration::seconds((hi - lo).num_seconds() / 2);
        let beyond = if upward {
            sun_altitude(lat, lon, mid) >= threshold
        } else {
            sun_altitude(lat, lon, mid) <= threshold
        };
        if beyond {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, FixedOffset, NaiveDate, TimeZone};

    #[test]
    fn paris_sunrise_sunset_within_sixty_seconds_of_published_values() {
        // Paris, 2026-01-01: NOAA sunrise ~08:44 CET, sunset ~17:04:30 CET (UTC+1 all winter).
        let paris_tz = FixedOffset::east_opt(3600).unwrap();
        let e = sun_events(
            48.8566,
            2.3522,
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        );
        let sunrise = e.sunrise.unwrap().with_timezone(&paris_tz);
        let sunset = e.sunset.unwrap().with_timezone(&paris_tz);
        let want_sunrise = paris_tz.with_ymd_and_hms(2026, 1, 1, 8, 44, 0).unwrap();
        let want_sunset = paris_tz.with_ymd_and_hms(2026, 1, 1, 17, 4, 30).unwrap();
        assert!((sunrise - want_sunrise).num_seconds().abs() <= 60);
        assert!((sunset - want_sunset).num_seconds().abs() <= 60);
    }

    #[test]
    fn lyon_summer_solstice_has_four_events_in_order() {
        let e = sun_events(45.75, 4.85, NaiveDate::from_ymd_opt(2026, 6, 21).unwrap());
        assert!(e.dawn.unwrap() < e.sunrise.unwrap());
        assert!(e.sunset.unwrap() < e.dusk.unwrap());
    }

    #[test]
    fn is_dark_boundary_matches_dusk_and_dawn() {
        let (lat, lon) = (48.8566, 2.3522);
        let e = sun_events(lat, lon, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
        let dusk = e.dusk.unwrap();
        assert!(is_dark_at(lat, lon, dusk));
        assert!(!is_dark_at(lat, lon, dusk - Duration::seconds(60)));
        let dawn = e.dawn.unwrap();
        assert!(is_dark_at(lat, lon, dawn - Duration::seconds(60)));
        assert!(!is_dark_at(lat, lon, dawn + Duration::seconds(60)));
    }

    #[test]
    fn is_dark_offset_moves_dusk_and_dawn_in_opposite_directions() {
        let (lat, lon) = (48.8566, 2.3522);
        let e = sun_events(lat, lon, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
        let (dusk, dawn) = (e.dusk.unwrap(), e.dawn.unwrap());
        let minute = Duration::seconds(60);
        assert!(is_dark_with_offset(
            lat,
            lon,
            dusk - Duration::minutes(30),
            -30
        ));
        assert!(!is_dark_with_offset(
            lat,
            lon,
            dusk - Duration::minutes(30) - minute,
            -30
        ));
        assert!(is_dark_with_offset(
            lat,
            lon,
            dawn + Duration::minutes(30) - minute,
            -30
        ));
        assert!(!is_dark_with_offset(
            lat,
            lon,
            dawn + Duration::minutes(30) + minute,
            -30
        ));
        assert!(!is_dark_with_offset(
            lat,
            lon,
            dusk + Duration::minutes(30) - minute,
            30
        ));
        assert!(is_dark_with_offset(
            lat,
            lon,
            dusk + Duration::minutes(30),
            30
        ));
        assert!(is_dark_with_offset(
            lat,
            lon,
            dawn - Duration::minutes(30) - minute,
            30
        ));
        assert!(!is_dark_with_offset(
            lat,
            lon,
            dawn - Duration::minutes(30) + minute,
            30
        ));
        assert_eq!(
            is_dark_with_offset(lat, lon, dusk, 0),
            is_dark_at(lat, lon, dusk)
        );
    }

    #[test]
    fn polar_night_leaves_events_none_without_panicking() {
        let e = sun_events(78.22, 15.65, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
        assert_eq!(e.sunrise, None);
        assert_eq!(e.sunset, None);
    }
}
