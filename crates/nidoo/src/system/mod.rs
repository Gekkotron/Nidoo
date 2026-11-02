//! System health: machine figures, restart tracking and service status.

pub mod heartbeat;
pub mod restarts;
pub mod sample;

use crate::hub::Hub;

impl Hub {
    /// Record a deliberate stop so the next boot counts it as a clean restart.
    /// No-op without a setup context; errors are logged and swallowed.
    pub fn mark_clean_stop(&self) {
        let Some(ctx) = self.setup.get() else {
            return;
        };
        if let Err(e) = heartbeat::write_clean_stop(&ctx.data_dir, crate::clock::now()) {
            tracing::warn!(error = %e, "could not write the clean-stop marker");
        }
    }
}
