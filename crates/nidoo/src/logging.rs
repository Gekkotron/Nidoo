use tracing_subscriber::EnvFilter;

use crate::config::LogConfig;

/// The filter for `level` (a `[log] level`): an invalid level falls back to `info`. `rmcp` is
/// held at `info`, because its `debug` line for a received request prints the tool arguments,
/// which can carry a secret (a `set_settings` value).
fn filter_for_level(level: &str) -> EnvFilter {
    let base = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));
    base.add_directive("rmcp=info".parse().expect("a valid directive"))
}

/// Installs the global subscriber. `RUST_LOG` wins over `cfg.level` (the operator's explicit
/// choice, used as given). Output goes to stderr.
pub fn init(cfg: &LogConfig) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| filter_for_level(&cfg.level));
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr);
    if cfg.json {
        let _ = builder.json().try_init();
    } else {
        let _ = builder.pretty().try_init();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::SubscriberExt;

    struct Seen(Arc<Mutex<Vec<String>>>);

    impl<S: tracing::Subscriber> Layer<S> for Seen {
        fn on_event(&self, e: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
            self.0
                .lock()
                .unwrap()
                .push(e.metadata().target().to_owned());
        }
    }

    /// The targets of the `debug` events (one per target) that get through `filter`.
    fn debug_targets(filter: EnvFilter) -> Vec<String> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let subscriber =
            tracing_subscriber::registry().with(Seen(Arc::clone(&seen)).with_filter(filter));
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(target: "rmcp", "received request");
            tracing::debug!(target: "rmcp::service", "received request");
            tracing::debug!(target: "nidoo", "own");
        });
        let all = seen.lock().unwrap();
        all.clone()
    }

    #[test]
    fn debug_level_keeps_rmcp_at_info() {
        assert_eq!(debug_targets(filter_for_level("debug")), ["nidoo"]);
    }

    #[test]
    fn an_invalid_level_falls_back_to_info() {
        assert!(debug_targets(filter_for_level("%%%")).is_empty());
    }

    #[test]
    fn an_explicit_filter_is_left_alone() {
        // `RUST_LOG` is used as given, so an operator can still ask for rmcp debug.
        assert_eq!(debug_targets(EnvFilter::new("debug")).len(), 3);
    }
}
