//! `nidoo init-bundled`: the real binary run against a temporary data dir.

use assert_cmd::Command;

fn init(dir: &std::path::Path) -> assert_cmd::assert::Assert {
    Command::cargo_bin("nidoo")
        .unwrap()
        .arg("--data")
        .arg(dir)
        .arg("init-bundled")
        .env_remove("OIKOS_DATA_DIR")
        .assert()
}

#[test]
fn cli_creates_then_keeps() {
    let tmp = tempfile::tempdir().unwrap();
    let first = init(tmp.path()).success();
    assert!(String::from_utf8_lossy(&first.get_output().stdout).contains("created"));
    let second = init(tmp.path()).success();
    assert!(String::from_utf8_lossy(&second.get_output().stdout).contains("nothing changed"));
}

#[test]
fn cli_fails_on_orphan_database() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("timescaledb")).unwrap();
    std::fs::write(tmp.path().join("timescaledb/PG_VERSION"), "16\n").unwrap();
    let out = init(tmp.path()).code(1);
    let text = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(text.contains("bundled.json is gone"), "{text}");
}
