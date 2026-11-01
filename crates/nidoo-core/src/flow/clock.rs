//! The `time.between` node's `HH:MM` clock times and daily range.

/// Parses exactly `HH:MM` (two digits each, 24-hour) into the minute of the day, 0..1440.
pub fn parse_clock(s: &str) -> Result<u16, String> {
    let bad = || format!("`{s}` is not a time of day (use HH:MM, 00:00 to 23:59)");
    let b = s.as_bytes();
    if b.len() != 5 || b[2] != b':' {
        return Err(bad());
    }
    let digits = |part: &str| -> Option<u16> {
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())
            .flatten()
    };
    match (digits(&s[..2]), digits(&s[3..])) {
        (Some(h), Some(m)) if h < 24 && m < 60 => Ok(h * 60 + m),
        _ => Err(bad()),
    }
}

/// Whether minute `now` is in `[from, to)`; a `to` before `from` crosses midnight.
pub fn in_time_range(from: u16, to: u16, now: u16) -> bool {
    if from <= to {
        from <= now && now < to
    } else {
        now >= from || now < to
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clock_forms() {
        assert_eq!(parse_clock("07:00"), Ok(420));
        assert_eq!(parse_clock("00:00"), Ok(0));
        assert_eq!(parse_clock("23:59"), Ok(1439));
        for bad in [
            "7:00", "24:00", "07:60", "0700", "", "07:0a", "+7:00", "07:000",
        ] {
            assert!(parse_clock(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn time_range_bounds() {
        assert!(in_time_range(420, 1350, 420));
        assert!(in_time_range(420, 1350, 1349));
        assert!(!in_time_range(420, 1350, 1350));
        assert!(!in_time_range(420, 1350, 419));
        assert!(in_time_range(1320, 360, 0));
        assert!(in_time_range(1320, 360, 1320));
        assert!(in_time_range(1320, 360, 359));
        assert!(!in_time_range(1320, 360, 360));
        assert!(!in_time_range(1320, 360, 720));
    }
}
