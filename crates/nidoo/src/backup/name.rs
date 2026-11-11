//! The file name of a backup: `nidoo-backup-YYYYMMDD-HHMMSS.tar.zst.age`, in UTC.

use chrono::{DateTime, NaiveDateTime, Utc};

const PREFIX: &str = "nidoo-backup-";
const SUFFIX: &str = ".tar.zst.age";
const STAMP: &str = "%Y%m%d-%H%M%S";

/// The name of the backup taken at `at`.
pub fn file_name(at: DateTime<Utc>) -> String {
    format!("{PREFIX}{}{SUFFIX}", at.format(STAMP))
}

/// When the backup called `name` was taken; `None` for anything that is not exactly a backup
/// name (a `.partial` file, a stranger, a malformed stamp).
pub fn parse(name: &str) -> Option<DateTime<Utc>> {
    let stamp = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    let well_formed = stamp.len() == 15
        && stamp.bytes().enumerate().all(|(i, b)| {
            if i == 8 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    if !well_formed {
        return None;
    }
    let naive = NaiveDateTime::parse_from_str(stamp, STAMP).ok()?;
    Some(naive.and_utc())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn name_round_trips() {
        let at = Utc.with_ymd_and_hms(2026, 11, 11, 22, 45, 12).unwrap();
        let name = file_name(at);
        assert_eq!(name, "nidoo-backup-20261111-224512.tar.zst.age");
        assert_eq!(parse(&name), Some(at));
    }

    #[test]
    fn name_rejects() {
        for name in [
            "nidoo-backup-20261111-224512.tar.zst.age.partial",
            "notes.txt",
            "nidoo-backup-2026.tar.zst.age",
            "nidoo-backup-20261311-224512.tar.zst.age",
            "nidoo-backup-20261111-2245123.tar.zst.age",
            "nidoo-backup-+0261111-224512.tar.zst.age",
            "x-nidoo-backup-20261111-224512.tar.zst.age",
        ] {
            assert_eq!(parse(name), None, "{name}");
        }
    }
}
