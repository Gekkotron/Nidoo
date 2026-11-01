//! `bundled.json`: the credentials of the broker and database the compose file ships.

use std::fmt;
use std::path::Path;

use anyhow::Context;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use super::HUB_FILE;
use super::fs::write_private_atomic;

pub const BUNDLED_FILE: &str = "bundled.json";

#[derive(Clone, Serialize, Deserialize)]
pub struct Bundled {
    pub mqtt_user: String,
    pub mqtt_password: String,
    pub db_user: String,
    pub db_password: String,
    pub db_name: String,
}

impl fmt::Debug for Bundled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bundled")
            .field("mqtt_user", &self.mqtt_user)
            .field("mqtt_password", &"***")
            .field("db_user", &self.db_user)
            .field("db_password", &"***")
            .field("db_name", &self.db_name)
            .finish()
    }
}

/// `None` when the file is missing or unreadable: the wizard then pre-fills nothing.
pub fn load(dir: &Path) -> Option<Bundled> {
    let text = std::fs::read_to_string(dir.join(BUNDLED_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// The broker configuration shipped to the bundled Mosquitto; written once, then the user's.
const MOSQUITTO_CONF: &str = include_str!("../../assets/mosquitto.conf");
/// Fixed user and database names of the bundled stack.
const BUNDLED_NAME: &str = "oikos";
/// Mosquitto's PBKDF2 iteration count for `$7$` hashes.
const PBKDF2_ROUNDS: u32 = 101;
const SECRET_LEN: usize = 24;
const ALPHABET: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// What `init_bundled` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitOutcome {
    Created,
    Kept,
    HubConfigured,
    DatabaseWithoutCredentials,
}

/// A `passwd` line in Mosquitto's `$7$` format (PBKDF2-HMAC-SHA512, 101 rounds); the salt is
/// the raw 12 bytes, base64-encoded in the line.
pub fn mosquitto_password_line(user: &str, password: &str, salt: &[u8; 12]) -> String {
    let mut hash = [0u8; 64];
    pbkdf2::pbkdf2_hmac::<sha2::Sha512>(password.as_bytes(), salt, PBKDF2_ROUNDS, &mut hash);
    format!(
        "{user}:$7${PBKDF2_ROUNDS}${}${}",
        STANDARD.encode(salt),
        STANDARD.encode(hash)
    )
}

/// 24 random characters from `[A-Za-z0-9]`: safe in a URL.
pub fn random_secret() -> String {
    let mut out = String::with_capacity(SECRET_LEN);
    while out.len() < SECRET_LEN {
        let mut buf = [0u8; 32];
        getrandom::getrandom(&mut buf).expect("the OS random source");
        // 248 is the largest multiple of 62 that fits in a byte: higher values would bias.
        out.extend(
            buf.iter()
                .filter(|b| **b < 248)
                .map(|b| ALPHABET[usize::from(*b) % ALPHABET.len()] as char)
                .take(SECRET_LEN - out.len()),
        );
    }
    out
}

/// Creates the bundled broker and database credentials unless they, or a hub that does not
/// use them, already exist. `legacy_toml` is the `--config` file: a host upgraded by an older
/// `update.sh` still has its `oikos.toml` at the repository root, outside `dir`, and counts as
/// a hub that is already set up. `bundled.json` is written last: a run that stops earlier
/// leaves no `bundled.json`, so the next run starts over.
pub fn init_bundled(dir: &Path, legacy_toml: Option<&Path>) -> anyhow::Result<InitOutcome> {
    if dir.join(BUNDLED_FILE).exists() {
        return Ok(InitOutcome::Kept);
    }
    if dir.join(HUB_FILE).exists()
        || dir.join("oikos.toml").exists()
        || legacy_toml.is_some_and(Path::is_file)
    {
        return Ok(InitOutcome::HubConfigured);
    }
    if dir.join("timescaledb/PG_VERSION").exists() {
        return Ok(InitOutcome::DatabaseWithoutCredentials);
    }
    let bundled = Bundled {
        mqtt_user: BUNDLED_NAME.into(),
        mqtt_password: random_secret(),
        db_user: BUNDLED_NAME.into(),
        db_password: random_secret(),
        db_name: BUNDLED_NAME.into(),
    };
    let mut salt = [0u8; 12];
    getrandom::getrandom(&mut salt).context("reading random bytes")?;
    let line = mosquitto_password_line(&bundled.mqtt_user, &bundled.mqtt_password, &salt);

    let config = dir.join("mosquitto/config");
    std::fs::create_dir_all(&config)?;
    let conf = config.join("mosquitto.conf");
    if !conf.exists() {
        std::fs::write(&conf, MOSQUITTO_CONF).context("writing mosquitto.conf")?;
    }
    write_private_atomic(&config.join("passwd"), format!("{line}\n").as_bytes())
        .context("writing the broker password file")?;
    write_private_atomic(
        &dir.join("bundled/db-password"),
        bundled.db_password.as_bytes(),
    )
    .context("writing the database password file")?;
    std::fs::create_dir_all(dir.join("mosquitto/data"))?;
    std::fs::create_dir_all(dir.join("mosquitto/log"))?;
    write_private_atomic(
        &dir.join(BUNDLED_FILE),
        &serde_json::to_vec_pretty(&bundled)?,
    )
    .context("writing bundled.json")?;
    Ok(InitOutcome::Created)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(dir: &Path, rel: &str) -> String {
        std::fs::read_to_string(dir.join(rel)).unwrap()
    }

    #[test]
    fn password_line_matches_mosquitto_passwd() {
        let salt: [u8; 12] = STANDARD
            .decode("8ByG83gjOOd8TqHW")
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(
            mosquitto_password_line("oikos", "correct horse", &salt),
            "oikos:$7$101$8ByG83gjOOd8TqHW$pYBnuy3hWyF7SI1EMLZE3/NTahEnAorkGhzthwFEyTulx8mn9VkqQdIM/2/6a0mrHH7T5QDY+SgGWTmJr899ZA=="
        );
    }

    #[test]
    fn random_secret_is_24_url_safe_chars() {
        let a = random_secret();
        assert_eq!(a.len(), 24);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(a, random_secret());
    }

    #[test]
    fn creates_everything_in_an_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        assert_eq!(init_bundled(dir, None).unwrap(), InitOutcome::Created);
        let b = load(dir).unwrap();
        assert_eq!(b.mqtt_user, "oikos");
        assert_eq!(b.db_user, "oikos");
        assert_eq!(b.db_name, "oikos");
        assert_eq!(read(dir, "bundled/db-password"), b.db_password);
        let passwd = read(dir, "mosquitto/config/passwd");
        let line = passwd.trim_end();
        assert!(line.starts_with("oikos:$7$101$"));
        let salt: [u8; 12] = STANDARD
            .decode(line.split('$').nth(3).unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(
            mosquitto_password_line("oikos", &b.mqtt_password, &salt),
            line
        );
        assert!(read(dir, "mosquitto/config/mosquitto.conf").contains("allow_anonymous false"));
        assert!(dir.join("mosquitto/data").is_dir());
        assert!(dir.join("mosquitto/log").is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for rel in [
                BUNDLED_FILE,
                "bundled/db-password",
                "mosquitto/config/passwd",
            ] {
                let mode = std::fs::metadata(dir.join(rel))
                    .unwrap()
                    .permissions()
                    .mode();
                assert_eq!(mode & 0o777, 0o600, "{rel}");
            }
        }
    }

    #[test]
    fn second_run_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            init_bundled(tmp.path(), None).unwrap(),
            InitOutcome::Created
        );
        let before = std::fs::read(tmp.path().join(BUNDLED_FILE)).unwrap();
        assert_eq!(init_bundled(tmp.path(), None).unwrap(), InitOutcome::Kept);
        assert_eq!(
            std::fs::read(tmp.path().join(BUNDLED_FILE)).unwrap(),
            before
        );
    }

    #[test]
    fn init_skips_a_configured_hub() {
        for file in [HUB_FILE, "oikos.toml"] {
            let tmp = tempfile::tempdir().unwrap();
            std::fs::write(tmp.path().join(file), "").unwrap();
            assert_eq!(
                init_bundled(tmp.path(), None).unwrap(),
                InitOutcome::HubConfigured
            );
            assert!(!tmp.path().join(BUNDLED_FILE).exists());
            assert!(!tmp.path().join("mosquitto").exists());
        }
    }

    #[test]
    fn init_skips_a_legacy_toml_outside_the_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let toml = tmp.path().join("oikos.toml");
        std::fs::write(&toml, "").unwrap();
        assert_eq!(
            init_bundled(&data, Some(&toml)).unwrap(),
            InitOutcome::HubConfigured
        );
        assert!(!data.join(BUNDLED_FILE).exists());
        assert!(!data.join("mosquitto").exists());
        // A path that does not exist is no hub: the credentials are created.
        assert_eq!(
            init_bundled(&data, Some(&tmp.path().join("nope.toml"))).unwrap(),
            InitOutcome::Created
        );
    }

    #[test]
    fn refuses_when_the_database_exists_without_credentials() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("timescaledb")).unwrap();
        std::fs::write(tmp.path().join("timescaledb/PG_VERSION"), "16\n").unwrap();
        assert_eq!(
            init_bundled(tmp.path(), None).unwrap(),
            InitOutcome::DatabaseWithoutCredentials
        );
        assert!(!tmp.path().join(BUNDLED_FILE).exists());
    }

    #[test]
    fn regenerates_after_a_partial_run() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join("mosquitto/config")).unwrap();
        std::fs::write(dir.join("mosquitto/config/passwd"), "stale").unwrap();
        std::fs::create_dir_all(dir.join("bundled")).unwrap();
        std::fs::write(dir.join("bundled/db-password"), "stale").unwrap();
        assert_eq!(init_bundled(dir, None).unwrap(), InitOutcome::Created);
        let b = load(dir).unwrap();
        assert_eq!(read(dir, "bundled/db-password"), b.db_password);
        let passwd = read(dir, "mosquitto/config/passwd");
        let salt: [u8; 12] = STANDARD
            .decode(passwd.split('$').nth(3).unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(
            mosquitto_password_line("oikos", &b.mqtt_password, &salt),
            passwd.trim_end()
        );
    }

    #[test]
    fn keeps_an_edited_mosquitto_conf() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join("mosquitto/config")).unwrap();
        std::fs::write(dir.join("mosquitto/config/mosquitto.conf"), "# mine").unwrap();
        assert_eq!(init_bundled(dir, None).unwrap(), InitOutcome::Created);
        assert_eq!(read(dir, "mosquitto/config/mosquitto.conf"), "# mine");
    }
}
