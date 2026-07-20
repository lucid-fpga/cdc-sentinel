//! Error type. The analysis is fail-soft on individual files (an unreadable or
//! unparseable file is recorded as a limit, not a crash), so most of the crate
//! returns data; [`Error`] covers the boundaries that genuinely cannot proceed —
//! I/O opening a core directory and CLI argument handling.

use thiserror::Error;

/// Result alias for the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// A failure that stops the analysis of a whole core.
#[derive(Debug, Error)]
pub enum Error {
    /// The core directory could not be walked.
    #[error("cannot read core source at {path}: {source}")]
    Source {
        /// The path that could not be read.
        path: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },

    /// A fixture/expected file could not be parsed as JSON.
    #[error("cannot parse {path} as JSON: {source}")]
    Json {
        /// The offending path.
        path: String,
        /// The underlying serde error.
        source: serde_json::Error,
    },

    /// The CLI was invoked with unusable arguments.
    #[error("usage: {0}")]
    Usage(String),
}
