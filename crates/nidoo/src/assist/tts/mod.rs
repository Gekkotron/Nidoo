//! Text-to-speech: the Phonon model, fetched from Hugging Face and run on the hub's CPU.

pub mod download;
pub mod engine;
pub mod hf;
#[cfg(feature = "tts")]
pub mod phonon;
pub mod source;
pub mod store;
pub mod tone;

/// The Hugging Face site every model comes from.
pub const HF_BASE: &str = "https://huggingface.co";

/// Limit the CPU threads the speech engine may use, unless the user already did.
///
/// When `RAYON_NUM_THREADS` is unset, this caps the engine at half the logical
/// cores (at least one); when it is set, that value is the user's override and
/// is left alone. Without the `tts` feature it does nothing.
///
/// This sets an environment variable internally: call it before any other
/// thread exists (first thing in `main`).
pub fn limit_threads() {
    #[cfg(feature = "tts")]
    if std::env::var_os("RAYON_NUM_THREADS").is_none() {
        let cores = std::thread::available_parallelism().map_or(1, usize::from);
        xn::set_num_threads((cores / 2).max(1));
    }
}

/// Why a speech operation failed.
#[derive(Debug, thiserror::Error)]
pub enum TtsError {
    /// The model, voice or repository does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The request is not acceptable as given.
    #[error("{0}")]
    Invalid(String),
    /// The request clashes with the current state.
    #[error("{0}")]
    Conflict(String),
    /// Another speech job is running.
    #[error("speech is busy, try again in a moment")]
    Busy,
    /// Hugging Face (or the network) failed.
    #[error("Hugging Face failed: {0}")]
    Upstream(String),
    /// Hugging Face refused access.
    #[error(
        "Hugging Face refused the download: accept the model's terms on its page, or check the token"
    )]
    Denied,
    /// The disk cannot hold the download.
    #[error("not enough disk space: need {need} bytes, {free} free")]
    NoSpace {
        /// Bytes the download needs.
        need: u64,
        /// Bytes free.
        free: u64,
    },
    /// A file is bigger than the allowed size.
    #[error("too large: {0}")]
    TooLarge(String),
    /// A disk operation failed.
    #[error("disk error: {0}")]
    Io(String),
}
