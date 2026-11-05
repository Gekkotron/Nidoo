//! Start at login: a LaunchAgent in `~/Library/LaunchAgents` that opens this
//! executable (inside the installed `.app`) when the owner logs in.

use std::path::PathBuf;

use anyhow::Context;

const LABEL: &str = "io.github.gekkotron.nidoo-satellite";

fn plist_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist")),
    )
}

pub fn enabled() -> bool {
    plist_path().is_some_and(|p| p.exists())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn plist(exe: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#,
        xml_escape(exe)
    )
}

pub fn set(on: bool) -> anyhow::Result<()> {
    let path = plist_path().context("no HOME")?;
    if on {
        let exe = std::env::current_exe().context("finding this executable")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, plist(&exe.to_string_lossy()))
            .with_context(|| format!("writing {}", path.display()))?;
    } else {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(e).with_context(|| format!("removing {}", path.display()));
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_escapes_the_path() {
        let p = plist("/Users/a&b/Applications/Nidoo Satellite.app/Contents/MacOS/x");
        assert!(p.contains("<string>/Users/a&amp;b/Applications/Nidoo Satellite.app"));
        assert!(p.contains("<string>io.github.gekkotron.nidoo-satellite</string>"));
    }
}
