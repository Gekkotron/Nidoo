//! The `settings` table (one JSON row per section) and `settings-cache.json`, the copy the
//! hub boots from when the database is down.

use std::path::Path;

use serde_json::Value;

use crate::history::db::Db;
use crate::setup::fs::write_private_atomic;
use crate::setup::sections::Sections;

pub const CACHE_FILE: &str = "settings-cache.json";

const UPSERT: &str = "INSERT INTO settings (key, value, updated_at) VALUES ($1, $2, now()) \
     ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()";

/// The stored sections; a missing row is that section's defaults.
pub async fn load_sections(db: &Db) -> Result<Sections, sqlx::Error> {
    Ok(load_stored(db).await?.unwrap_or_default())
}

/// Like [`load_sections`], but `None` when the table has no row at all (a new database).
pub async fn load_stored(db: &Db) -> Result<Option<Sections>, sqlx::Error> {
    let rows: Vec<(String, Value)> = sqlx::query_as("SELECT key, value FROM settings")
        .fetch_all(db.pool())
        .await?;
    Ok((!rows.is_empty()).then(|| Sections::from_rows(rows)))
}

/// Upserts every section row in one transaction.
pub async fn save_sections(db: &Db, s: &Sections) -> Result<(), sqlx::Error> {
    let mut tx = db.pool().begin().await?;
    for (key, value) in s.to_rows() {
        sqlx::query(UPSERT)
            .bind(key)
            .bind(value)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}

/// Upserts the one row `key` names, from `s`. An unknown key writes nothing.
pub async fn save_section(db: &Db, key: &str, s: &Sections) -> Result<(), sqlx::Error> {
    let Some((key, value)) = s.to_rows().into_iter().find(|(k, _)| *k == key) else {
        return Ok(());
    };
    sqlx::query(UPSERT)
        .bind(key)
        .bind(value)
        .execute(db.pool())
        .await?;
    Ok(())
}

/// `None` when the cache is missing or unparsable. The warning names the file, never its
/// content (it holds secrets).
pub fn read_cache(dir: &Path) -> Option<Sections> {
    let path = dir.join(CACHE_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!("cannot read {}: {}", path.display(), e.kind());
            return None;
        }
    };
    match serde_json::from_str(&text) {
        Ok(s) => Some(s),
        Err(_) => {
            tracing::warn!("{} is not valid; ignoring it", path.display());
            None
        }
    }
}

/// Atomic, mode 0600.
pub fn write_cache(dir: &Path, s: &Sections) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(s).map_err(std::io::Error::other)?;
    write_private_atomic(&dir.join(CACHE_FILE), &json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Sections {
        let mut s = Sections::default();
        s.solcast.api_key = Some("solcast-key".into());
        s.frigate.password = Some("frigate-pass".into());
        s.weather.enabled = true;
        s
    }

    #[test]
    fn cache_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("nested");
        let s = sample();
        write_cache(&dir, &s).unwrap();
        assert_eq!(read_cache(&dir), Some(s));
        assert!(!dir.join("settings-cache.json.tmp").exists());
    }

    #[test]
    fn missing_cache_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(read_cache(tmp.path()), None);
    }

    #[cfg(unix)]
    #[test]
    fn cache_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        write_cache(tmp.path(), &sample()).unwrap();
        let meta = std::fs::metadata(tmp.path().join(CACHE_FILE)).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn corrupt_cache_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(CACHE_FILE), "{\"solcast\": s3cret").unwrap();
        assert_eq!(read_cache(tmp.path()), None);
    }
}
