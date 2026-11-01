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
                exec_self();
            });
        }
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
}
