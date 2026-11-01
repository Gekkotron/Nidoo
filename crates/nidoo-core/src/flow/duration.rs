//! Parsing of the delay node's `for` text (`90s`, `5m`, `1h30m`).

use std::time::Duration;

/// Why a duration text was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DurationError {
    /// The text is empty.
    #[error("duration is empty")]
    Empty,
    /// A number is not followed by a unit (`5`).
    #[error("duration number is missing a unit (use s, m or h)")]
    MissingUnit,
    /// A unit letter other than `s`, `m` or `h`.
    #[error("unknown duration unit `{0}` (use s, m or h)")]
    BadUnit(char),
    /// A unit with no number before it, or a number too large.
    #[error("duration has a missing or invalid number")]
    BadNumber,
    /// The total is zero.
    #[error("duration must be longer than zero")]
    Zero,
}

/// Parses whole seconds, minutes and hours, alone or combined (`1h30m`).
///
/// Every number needs a unit, so `5` is refused rather than guessed; the total must not be zero.
pub fn parse_duration(text: &str) -> Result<Duration, DurationError> {
    if text.is_empty() {
        return Err(DurationError::Empty);
    }
    let mut total: u64 = 0;
    let mut digits = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let factor: u64 = match c {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            other => return Err(DurationError::BadUnit(other)),
        };
        let number: u64 = digits.parse().map_err(|_| DurationError::BadNumber)?;
        digits.clear();
        total = number
            .checked_mul(factor)
            .and_then(|part| total.checked_add(part))
            .ok_or(DurationError::BadNumber)?;
    }
    if !digits.is_empty() {
        return Err(DurationError::MissingUnit);
    }
    if total == 0 {
        return Err(DurationError::Zero);
    }
    Ok(Duration::from_secs(total))
}

/// The wait of a delay: `low` without `high`, otherwise a whole number of seconds in
/// `low..=high` chosen by `roll`, a uniformly random number the caller draws.
pub fn pick_duration(low: Duration, high: Option<Duration>, roll: u64) -> Duration {
    let Some(high) = high.filter(|high| *high > low) else {
        return low;
    };
    let span = high.as_secs() - low.as_secs();
    low + Duration::from_secs(roll % (span + 1))
}

/// Writes whole seconds in the same shape [`parse_duration`] reads (`1h12m5s`, `5m`).
pub fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let mut text = String::new();
    for (n, unit) in [(h, 'h'), (m, 'm'), (s, 's')] {
        if n > 0 {
            text.push_str(&format!("{n}{unit}"));
        }
    }
    if text.is_empty() {
        text.push_str("0s");
    }
    text
}

/// The shortest `for` of `trigger.silent` and `per` of `limit`.
pub const MIN_WINDOW: Duration = Duration::from_secs(60);
/// The longest `for` of `trigger.silent` and `per` of `limit`.
pub const MAX_WINDOW: Duration = Duration::from_secs(168 * 3600);

/// Parses a `trigger.silent` `for` or a `limit` `per`: a duration from `1m` to `168h`.
pub fn window(text: &str) -> Result<Duration, String> {
    let d = parse_duration(text).map_err(|e| e.to_string())?;
    if !(MIN_WINDOW..=MAX_WINDOW).contains(&d) {
        return Err(format!("`{text}` is outside 1m to 168h"));
    }
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_units_and_combinations() {
        assert_eq!(parse_duration("90s"), Ok(Duration::from_secs(90)));
        assert_eq!(parse_duration("5m"), Ok(Duration::from_secs(300)));
        assert_eq!(parse_duration("1h30m"), Ok(Duration::from_secs(5400)));
        assert_eq!(parse_duration("2h"), Ok(Duration::from_secs(7200)));
    }

    #[test]
    fn rejects_missing_unit_zero_and_garbage() {
        assert_eq!(parse_duration("5"), Err(DurationError::MissingUnit));
        assert_eq!(parse_duration("0m"), Err(DurationError::Zero));
        assert_eq!(parse_duration(""), Err(DurationError::Empty));
        assert_eq!(parse_duration("5x"), Err(DurationError::BadUnit('x')));
        assert_eq!(parse_duration("m"), Err(DurationError::BadNumber));
        assert_eq!(
            parse_duration("99999999999999999999s"),
            Err(DurationError::BadNumber)
        );
    }

    #[test]
    fn picks_the_lower_bound_without_an_upper_one() {
        let low = Duration::from_secs(60);
        assert_eq!(pick_duration(low, None, 12345), low);
    }

    #[test]
    fn picks_within_the_range_whole_seconds() {
        let low = Duration::from_secs(60);
        let high = Duration::from_secs(120);
        assert_eq!(pick_duration(low, Some(high), 0), low);
        assert_eq!(pick_duration(low, Some(high), 60), high);
        assert_eq!(pick_duration(low, Some(high), 61), low);
        for roll in [1, 7, 59, u64::MAX, 987_654_321] {
            let got = pick_duration(low, Some(high), roll);
            assert!(low <= got && got <= high, "{got:?}");
        }
        assert_eq!(pick_duration(low, Some(low), u64::MAX), low);
    }

    #[test]
    fn formats_like_the_input() {
        assert_eq!(format_duration(Duration::from_secs(300)), "5m");
        assert_eq!(format_duration(Duration::from_secs(5400)), "1h30m");
        assert_eq!(format_duration(Duration::from_secs(4325)), "1h12m5s");
        assert_eq!(format_duration(Duration::from_secs(7200)), "2h");
        assert_eq!(format_duration(Duration::from_secs(0)), "0s");
    }
}
