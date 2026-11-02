//! Restarting the hub in place after the setup changed something only a boot can pick up.

use std::os::unix::process::CommandExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::setup::SetupCtx;

#[derive(Clone)]
pub enum RestartHook {
    /// `exec` this program again with the same arguments.
    Exec,
    /// Count the requests (tests).
    Record(Arc<AtomicUsize>),
}

/// Restarts the hub after `after`. Only the first call does anything: a second one (a double
/// click, a lookup finishing next to a save) finds `restarting` already set.
pub fn schedule_restart(ctx: &Arc<SetupCtx>, after: Duration) {
    if ctx.restarting.swap(true, Ordering::SeqCst) {
        return;
    }
    match &ctx.restart {
        RestartHook::Record(count) => {
            count.fetch_add(1, Ordering::SeqCst);
        }
        RestartHook::Exec => {
            let ctx = Arc::clone(ctx);
            tokio::spawn(async move {
                tokio::time::sleep(after).await;
                // Never exec in the middle of a write: wait for the settings lock (a
                // `PUT /settings`), then the file lock (`update_file`), in the order every
                // writer takes them, and keep both until the process image is replaced.
                let _sections = ctx.sections.lock().await;
                let _file = ctx.file.lock().await;
                before_exec(&ctx);
                exec_self();
            });
        }
    }
}

/// What a restart writes before the image is replaced: the clean-stop marker, so the next
/// boot counts a restart the hub asked for as clean, not as a crash.
fn before_exec(ctx: &SetupCtx) {
    if let Err(error) =
        crate::system::heartbeat::write_clean_stop(&ctx.data_dir, crate::clock::now())
    {
        tracing::warn!(%error, "could not write the clean-stop marker");
    }
}

fn exec_self() -> ! {
    tracing::info!("restarting");
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            tracing::error!(%error, "cannot restart: unknown executable");
            std::process::exit(1);
        }
    };
    let error = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .exec();
    tracing::error!(%error, "cannot restart");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::sections::Sections;
    use crate::setup::{HubFile, Mode};

    #[test]
    fn only_the_first_request_counts() {
        let count = Arc::new(AtomicUsize::new(0));
        let ctx = Arc::new(SetupCtx::new(
            "unused".into(),
            Mode::Run,
            HubFile::default(),
            None,
            Sections::default(),
            RestartHook::Record(Arc::clone(&count)),
        ));
        schedule_restart(&ctx, Duration::ZERO);
        schedule_restart(&ctx, Duration::ZERO);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_restart_leaves_the_clean_stop_marker() {
        use crate::system::heartbeat::{self, Heartbeat, Previous};
        let dir = tempfile::tempdir().unwrap();
        heartbeat::write_heartbeat(dir.path(), 10).unwrap();
        let ctx = SetupCtx::new(
            dir.path().to_path_buf(),
            Mode::Run,
            HubFile::default(),
            None,
            Sections::default(),
            RestartHook::Exec,
        );
        before_exec(&ctx);
        let Previous::Found(Heartbeat {
            heartbeat,
            clean_stop,
        }) = heartbeat::read(dir.path())
        else {
            panic!("no heartbeat file");
        };
        assert_eq!(heartbeat, 10);
        assert!(clean_stop.is_some(), "the next boot would read a crash");
    }
}
