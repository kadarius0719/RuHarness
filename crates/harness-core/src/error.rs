//! Typed errors for the core crate (§10.2: thiserror at crate boundaries).

use std::path::PathBuf;

/// Errors produced by harness-core operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Filesystem I/O failure, with the path involved.
    #[error("io error at {path}: {source}")]
    Io {
        /// Path being read or written.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A ledger file failed to parse.
    #[error("parse error in {path}: {message}")]
    Parse {
        /// File that failed to parse.
        path: PathBuf,
        /// Human-readable description.
        message: String,
    },
    /// A ledger file carries a schema version newer than this harness supports.
    #[error("{path} has schema_version {found}, but this harness supports up to {supported}; upgrade the harness")]
    SchemaTooNew {
        /// File with the newer schema.
        path: PathBuf,
        /// Version found in the file.
        found: u64,
        /// Maximum version this build understands.
        supported: u64,
    },
    /// The plan violates a structural invariant (missing unit, cycle, …).
    #[error("invalid plan: {0}")]
    InvalidPlan(String),
    /// A referenced unit does not exist in the plan.
    #[error("unknown unit `{0}`")]
    UnknownUnit(String),
    /// Any other invariant violation, described in prose.
    #[error("{0}")]
    Invariant(String),
    /// A recorded attempt, re-judged by HEAD on its RECORDED replies, did not
    /// reproduce its record (docs/REPLAY-DESIGN.md): a finding about the
    /// harness or the judge — its evidence is intact (an integrity failure
    /// is an [`Error::Invariant`] instead).
    #[error(
        "attempt {attempt} does not reproduce from its traces — the recorded evidence was left \
         untouched; {} difference(s): {}",
        .differences.len(),
        .differences.join("; ")
    )]
    Diverged {
        /// The attempt id.
        attempt: String,
        /// The strict-tier differences, in order.
        differences: Vec<String>,
        /// The outcome the re-judged trajectory reached (a supersession of a
        /// green record needs `loosening` unless this is not green).
        replayed_outcome: String,
    },
}

impl Error {
    /// Convenience constructor for [`Error::Io`].
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    /// True when this is an I/O error for a file that does not exist —
    /// callers use it to distinguish "not created yet" from real failures
    /// that must propagate (e.g. [`Error::SchemaTooNew`]).
    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            Error::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound
        )
    }

    /// Convenience constructor for [`Error::Parse`].
    pub fn parse(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Error::Parse {
            path: path.into(),
            message: message.into(),
        }
    }
}
