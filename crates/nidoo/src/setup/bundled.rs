//! `bundled.json`: the credentials of the broker and database the compose file ships.

use std::fmt;
use std::path::Path;

use std::io::Write;

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
    /// The phone app's broker login, which the setup QR carries. Missing in a `bundled.json`
    /// written before it existed; the next `init-bundled` adds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_mqtt_user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_mqtt_password: Option<String>,
}

impl Bundled {
    /// The phone app's broker user and password, when both exist.
    pub fn app_login(&self) -> Option<(&str, &str)> {
        Some((
            self.app_mqtt_user.as_deref()?,
            self.app_mqtt_password.as_deref()?,
        ))
    }
}

impl fmt::Debug for Bundled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bundled")
            .field("mqtt_user", &self.mqtt_user)
            .field("mqtt_password", &"***")
            .field("db_user", &self.db_user)
            .field("db_password", &"***")
            .field("db_name", &self.db_name)
            .field("app_mqtt_user", &self.app_mqtt_user)
            .field(
                "app_mqtt_password",
                &self.app_mqtt_password.as_ref().map(|_| "***"),
            )
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
/// The broker ACL shipped next to it: every user keeps full access except the app login.
const MOSQUITTO_ACL: &str = include_str!("../../assets/mosquitto.acl");
/// The line an upgrade appends to a `mosquitto.conf` that names no ACL file.
const ACL_FILE_LINE: &str = "acl_file /mosquitto/config/acl";
/// The phone app's broker user; `mosquitto.acl` names it.
pub const APP_MQTT_USER: &str = "nidoo-app";
/// Fixed user and database names of the bundled stack.
const BUNDLED_NAME: &str = "nidoo";
/// Mosquitto's PBKDF2 iteration count for `$7$` hashes.
const PBKDF2_ROUNDS: u32 = 101;
const SECRET_LEN: usize = 24;
const ALPHABET: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// What `init_bundled` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitOutcome {
    Created,
    Kept,
    /// An install from before the app login: the app login and the ACL were added.
    AppLoginAdded,
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
/// use them, already exist; adds the app login and the ACL to an install from before them.
/// `bundled.json` is written last: a run that stops earlier leaves it as it was, so the next
/// run starts over.
pub fn init_bundled(dir: &Path) -> anyhow::Result<InitOutcome> {
    if dir.join(BUNDLED_FILE).exists() {
        return match load(dir) {
            Some(b) if b.app_login().is_none() => {
                add_app_login(dir, b)?;
                Ok(InitOutcome::AppLoginAdded)
            }
            _ => Ok(InitOutcome::Kept),
        };
    }
    if dir.join(HUB_FILE).exists() {
        return Ok(InitOutcome::HubConfigured);
    }
    if dir.join("timescaledb/PG_VERSION").exists() {
        return Ok(InitOutcome::DatabaseWithoutCredentials);
    }
    let app_password = random_secret();
    let app_line = password_line(APP_MQTT_USER, &app_password)?;
    let bundled = Bundled {
        mqtt_user: BUNDLED_NAME.into(),
        mqtt_password: random_secret(),
        db_user: BUNDLED_NAME.into(),
        db_password: random_secret(),
        db_name: BUNDLED_NAME.into(),
        app_mqtt_user: Some(APP_MQTT_USER.into()),
        app_mqtt_password: Some(app_password),
    };
    let hub_line = password_line(&bundled.mqtt_user, &bundled.mqtt_password)?;

    let config = dir.join("mosquitto/config");
    std::fs::create_dir_all(&config)?;
    let conf = config.join("mosquitto.conf");
    if !conf.exists() {
        std::fs::write(&conf, MOSQUITTO_CONF).context("writing mosquitto.conf")?;
    }
    write_acl(&config)?;
    write_private_atomic(
        &config.join("passwd"),
        format!("{hub_line}\n{app_line}\n").as_bytes(),
    )
    .context("writing the broker password file")?;
    write_private_atomic(
        &dir.join("bundled/db-password"),
        bundled.db_password.as_bytes(),
    )
    .context("writing the database password file")?;
    std::fs::create_dir_all(dir.join("mosquitto/data"))?;
    std::fs::create_dir_all(dir.join("mosquitto/log"))?;
    write_bundled(dir, &bundled)?;
    Ok(InitOutcome::Created)
}

/// Adds the app login to `passwd` (every other line kept, a stale app line replaced), the ACL
/// file when missing, and an `acl_file` line to a `mosquitto.conf` that names none.
fn add_app_login(dir: &Path, mut bundled: Bundled) -> anyhow::Result<()> {
    let password = random_secret();
    let app_line = password_line(APP_MQTT_USER, &password)?;
    let config = dir.join("mosquitto/config");
    std::fs::create_dir_all(&config)?;
    let passwd_path = config.join("passwd");
    let old = match std::fs::read_to_string(&passwd_path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).context("reading the broker password file"),
    };
    let prefix = format!("{APP_MQTT_USER}:");
    let mut passwd: String = old
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with(&prefix))
        .map(|l| format!("{l}\n"))
        .collect();
    passwd.push_str(&app_line);
    passwd.push('\n');
    write_acl(&config)?;
    write_private_atomic(&passwd_path, passwd.as_bytes())
        .context("writing the broker password file")?;
    let conf = config.join("mosquitto.conf");
    match std::fs::read_to_string(&conf) {
        Ok(text) if has_acl_file(&text) => {}
        Ok(text) => {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&conf)
                .context("opening mosquitto.conf")?;
            let newline = if text.is_empty() || text.ends_with('\n') {
                ""
            } else {
                "\n"
            };
            writeln!(f, "{newline}{ACL_FILE_LINE}").context("writing mosquitto.conf")?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::write(&conf, MOSQUITTO_CONF).context("writing mosquitto.conf")?;
        }
        Err(e) => return Err(e).context("reading mosquitto.conf"),
    }
    bundled.app_mqtt_user = Some(APP_MQTT_USER.into());
    bundled.app_mqtt_password = Some(password);
    write_bundled(dir, &bundled)
}

/// Whether a `mosquitto.conf` already names an ACL file (a commented line does not count).
fn has_acl_file(conf: &str) -> bool {
    conf.lines()
        .any(|l| l.split_whitespace().next() == Some("acl_file"))
}

/// The shipped ACL, unless an ACL file is already there: then it is the user's.
fn write_acl(config: &Path) -> anyhow::Result<()> {
    let path = config.join("acl");
    if path.exists() {
        return Ok(());
    }
    write_private_atomic(&path, MOSQUITTO_ACL.as_bytes()).context("writing the broker ACL")
}

fn password_line(user: &str, password: &str) -> anyhow::Result<String> {
    let mut salt = [0u8; 12];
    getrandom::getrandom(&mut salt).context("reading random bytes")?;
    Ok(mosquitto_password_line(user, password, &salt))
}

fn write_bundled(dir: &Path, bundled: &Bundled) -> anyhow::Result<()> {
    write_private_atomic(
        &dir.join(BUNDLED_FILE),
        &serde_json::to_vec_pretty(bundled)?,
    )
    .context("writing bundled.json")
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
            mosquitto_password_line("nidoo", "correct horse", &salt),
            "nidoo:$7$101$8ByG83gjOOd8TqHW$pYBnuy3hWyF7SI1EMLZE3/NTahEnAorkGhzthwFEyTulx8mn9VkqQdIM/2/6a0mrHH7T5QDY+SgGWTmJr899ZA=="
        );
    }

    #[test]
    fn random_secret_is_24_url_safe_chars() {
        let a = random_secret();
        assert_eq!(a.len(), 24);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(a, random_secret());
    }

    /// Whether `line` is the `passwd` line of `user` with `password`.
    fn line_matches(line: &str, user: &str, password: &str) -> bool {
        let Some(salt) = line.split('$').nth(3) else {
            return false;
        };
        let salt: [u8; 12] = STANDARD.decode(salt).unwrap().try_into().unwrap();
        mosquitto_password_line(user, password, &salt) == line
    }

    /// A bundled install from before the app login: `bundled.json` without it, the hub's
    /// line plus a hand-added Zigbee2MQTT line in `passwd`, the old `mosquitto.conf`.
    fn old_install(dir: &Path) -> String {
        let salt = [7u8; 12];
        let hub_line = mosquitto_password_line("nidoo", "hubpass", &salt);
        let config = dir.join("mosquitto/config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("passwd"),
            format!("{hub_line}\nzigbee2mqtt:$7$101$abc$def\n"),
        )
        .unwrap();
        std::fs::write(
            config.join("mosquitto.conf"),
            "listener 1883\npassword_file /mosquitto/config/passwd",
        )
        .unwrap();
        std::fs::write(
            dir.join(BUNDLED_FILE),
            r#"{"mqtt_user":"nidoo","mqtt_password":"hubpass","db_user":"nidoo","db_password":"dbpass","db_name":"nidoo"}"#,
        )
        .unwrap();
        hub_line
    }

    #[test]
    fn creates_the_app_login_and_the_acl() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        assert_eq!(init_bundled(dir).unwrap(), InitOutcome::Created);
        let b = load(dir).unwrap();
        let (user, password) = b.app_login().unwrap();
        assert_eq!(user, APP_MQTT_USER);
        assert_eq!(password.len(), 24);
        assert_ne!(password, b.mqtt_password);
        let passwd = read(dir, "mosquitto/config/passwd");
        let lines: Vec<&str> = passwd.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(line_matches(lines[0], "nidoo", &b.mqtt_password));
        assert!(line_matches(lines[1], APP_MQTT_USER, password));
        assert_eq!(read(dir, "mosquitto/config/acl"), MOSQUITTO_ACL);
        assert!(has_acl_file(&read(dir, "mosquitto/config/mosquitto.conf")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("mosquitto/config/acl"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn the_shipped_acl_denies_the_app_and_grants_everyone_else() {
        let lines: Vec<&str> = MOSQUITTO_ACL
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
        assert_eq!(
            lines,
            [
                "pattern readwrite #",
                "pattern read $SYS/#",
                "user nidoo-app",
                "topic deny nidoo/push/notification/#",
                "topic deny nidoo/alarm/settings",
                "topic deny nidoo/flow_labels",
                "topic deny $SYS/#",
            ]
        );
    }

    #[test]
    fn upgrade_adds_the_app_login_and_keeps_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let hub_line = old_install(dir);
        assert_eq!(init_bundled(dir).unwrap(), InitOutcome::AppLoginAdded);
        let b = load(dir).unwrap();
        assert_eq!(b.mqtt_password, "hubpass");
        assert_eq!(b.db_password, "dbpass");
        let (user, password) = b.app_login().unwrap();
        let passwd = read(dir, "mosquitto/config/passwd");
        let lines: Vec<&str> = passwd.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], hub_line);
        assert_eq!(lines[1], "zigbee2mqtt:$7$101$abc$def");
        assert!(line_matches(lines[2], user, password));
        assert_eq!(read(dir, "mosquitto/config/acl"), MOSQUITTO_ACL);
        let conf = read(dir, "mosquitto/config/mosquitto.conf");
        assert!(conf.starts_with("listener 1883\npassword_file /mosquitto/config/passwd\n"));
        assert!(has_acl_file(&conf));
        assert_eq!(init_bundled(dir).unwrap(), InitOutcome::Kept);
        assert_eq!(read(dir, "mosquitto/config/passwd"), passwd);
        assert_eq!(read(dir, "mosquitto/config/mosquitto.conf"), conf);
    }

    #[test]
    fn upgrade_keeps_an_own_acl_and_acl_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        old_install(dir);
        let conf = "listener 1883\nacl_file /mosquitto/config/mine\n";
        std::fs::write(dir.join("mosquitto/config/mosquitto.conf"), conf).unwrap();
        std::fs::write(dir.join("mosquitto/config/acl"), "# mine").unwrap();
        assert_eq!(init_bundled(dir).unwrap(), InitOutcome::AppLoginAdded);
        assert_eq!(read(dir, "mosquitto/config/mosquitto.conf"), conf);
        assert_eq!(read(dir, "mosquitto/config/acl"), "# mine");
    }

    #[test]
    fn upgrade_replaces_an_app_line_left_by_a_partial_run() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        old_install(dir);
        let path = dir.join("mosquitto/config/passwd");
        let mut passwd = std::fs::read_to_string(&path).unwrap();
        passwd.push_str("nidoo-app:stale\n");
        std::fs::write(&path, passwd).unwrap();
        assert_eq!(init_bundled(dir).unwrap(), InitOutcome::AppLoginAdded);
        let b = load(dir).unwrap();
        let (user, password) = b.app_login().unwrap();
        let passwd = read(dir, "mosquitto/config/passwd");
        let app: Vec<&str> = passwd
            .lines()
            .filter(|l| l.starts_with("nidoo-app:"))
            .collect();
        assert_eq!(app.len(), 1);
        assert!(line_matches(app[0], user, password));
    }

    #[test]
    fn acl_file_directive_is_found_only_uncommented() {
        assert!(has_acl_file("a\n  acl_file /x\n"));
        assert!(!has_acl_file("# acl_file /x\nacl_filex /y\n"));
        assert!(!has_acl_file(""));
    }

    #[test]
    fn debug_masks_the_app_password() {
        let tmp = tempfile::tempdir().unwrap();
        init_bundled(tmp.path()).unwrap();
        let b = load(tmp.path()).unwrap();
        let (_, password) = b.app_login().unwrap();
        assert!(!format!("{b:?}").contains(password));
    }

    #[test]
    fn creates_everything_in_an_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        assert_eq!(init_bundled(dir).unwrap(), InitOutcome::Created);
        let b = load(dir).unwrap();
        assert_eq!(b.mqtt_user, "nidoo");
        assert_eq!(b.db_user, "nidoo");
        assert_eq!(b.db_name, "nidoo");
        assert_eq!(read(dir, "bundled/db-password"), b.db_password);
        let passwd = read(dir, "mosquitto/config/passwd");
        let line = passwd.lines().next().unwrap();
        assert!(line.starts_with("nidoo:$7$101$"));
        let salt: [u8; 12] = STANDARD
            .decode(line.split('$').nth(3).unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(
            mosquitto_password_line("nidoo", &b.mqtt_password, &salt),
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
        assert_eq!(init_bundled(tmp.path()).unwrap(), InitOutcome::Created);
        let before = std::fs::read(tmp.path().join(BUNDLED_FILE)).unwrap();
        assert_eq!(init_bundled(tmp.path()).unwrap(), InitOutcome::Kept);
        assert_eq!(
            std::fs::read(tmp.path().join(BUNDLED_FILE)).unwrap(),
            before
        );
    }

    #[test]
    fn init_skips_a_configured_hub() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(HUB_FILE), "").unwrap();
        assert_eq!(
            init_bundled(tmp.path()).unwrap(),
            InitOutcome::HubConfigured
        );
        assert!(!tmp.path().join(BUNDLED_FILE).exists());
        assert!(!tmp.path().join("mosquitto").exists());
    }

    #[test]
    fn refuses_when_the_database_exists_without_credentials() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("timescaledb")).unwrap();
        std::fs::write(tmp.path().join("timescaledb/PG_VERSION"), "16\n").unwrap();
        assert_eq!(
            init_bundled(tmp.path()).unwrap(),
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
        assert_eq!(init_bundled(dir).unwrap(), InitOutcome::Created);
        let b = load(dir).unwrap();
        assert_eq!(read(dir, "bundled/db-password"), b.db_password);
        let passwd = read(dir, "mosquitto/config/passwd");
        assert!(line_matches(
            passwd.lines().next().unwrap(),
            "nidoo",
            &b.mqtt_password
        ));
    }

    #[test]
    fn keeps_an_edited_mosquitto_conf() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join("mosquitto/config")).unwrap();
        std::fs::write(dir.join("mosquitto/config/mosquitto.conf"), "# mine").unwrap();
        assert_eq!(init_bundled(dir).unwrap(), InitOutcome::Created);
        assert_eq!(read(dir, "mosquitto/config/mosquitto.conf"), "# mine");
    }
}
