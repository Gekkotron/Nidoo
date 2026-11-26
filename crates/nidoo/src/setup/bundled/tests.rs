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
