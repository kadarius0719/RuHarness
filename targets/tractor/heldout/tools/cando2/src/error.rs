// © 2026 Massachusetts Institute of Technology
// MIT License

use std::{
    backtrace::{Backtrace, BacktraceStatus},
    error::Error,
    fmt,
    io::{self, Write},
};

/// Return code for Cando
pub enum ReturnCode {
    /// Test vectors that were run were successfull
    Success = 0,
    /// Test vectors that were run failed for any reason (e.g., comparison failed, panicked,
    /// segfaulted, timeout)
    VectorRunFailure = 1,
    /// Test vectors not run because binary/library was not generated properly
    ArtifactNotFound = 2,
    /// Test vectors not run because library symbol was not found
    SymbolNotFound = 3,
    /// CLI usage error
    Usage = 4,
    /// Internal failure from cando
    CandoFailure = 5,
    /// Benchmarking failed because the benchmarked code exited too early
    BenchFailure = 6,
}

/// General error returned from main to represent what happened during the run of all specified
/// test vectors
#[derive(Debug)]
pub enum CandoError {
    AnyVectorFailed,
    AnyBenchFailed,
    ArtifactNotFound(String),
    SymbolNotFound(String),
    CatastrophicError(CatastrophicError),
}

impl fmt::Display for CandoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CandoError::AnyVectorFailed => {
                write!(f, "At least one of the test vectors run failed")
            }
            CandoError::AnyBenchFailed => write!(f, "At least one benchmark run failed"),
            CandoError::ArtifactNotFound(s) => {
                write!(f, "Couldn't find artifact with given name: {s}")
            }
            CandoError::SymbolNotFound(s) => {
                if s.starts_with("Couldn't find symbol/function with given name:") {
                    write!(f, "{s}")
                } else {
                    write!(f, "Couldn't find symbol/function with given name: {s}")
                }
            }
            CandoError::CatastrophicError(e) => write!(f, "{}", e),
        }
    }
}

impl CandoError {
    /// Constructs a `CatastrophicError::Usage` as a `CandoError`
    pub fn usage_err(s: &str) -> Self {
        let e = CatastrophicError::Usage(s.into());
        Self::CatastrophicError(e)
    }

    /// Prints the `CandoError` to stderr then exits the process with the correct rc
    pub fn exit(self) -> ! {
        let mut stderr = io::stderr().lock();
        let _ = writeln!(stderr, "{}", self);
        let _ = stderr.flush();
        std::process::exit(self.exit_code());
    }

    /// Converts `CandoError` to an integer `ReturnCode`
    pub fn exit_code(&self) -> i32 {
        let rc = match self {
            Self::AnyVectorFailed => ReturnCode::VectorRunFailure,
            Self::AnyBenchFailed => ReturnCode::BenchFailure,
            Self::ArtifactNotFound(_) => ReturnCode::ArtifactNotFound,
            Self::SymbolNotFound(_) => ReturnCode::SymbolNotFound,
            Self::CatastrophicError(CatastrophicError::Usage(_)) => ReturnCode::Usage,
            Self::CatastrophicError(CatastrophicError::InternalFailure {
                err: _,
                backtrace: _,
            }) => ReturnCode::CandoFailure,
        };
        rc as i32
    }

    /// Creates a `CandoError` from an arbitrary string `s`
    pub fn str_to_err(s: &str) -> Self {
        Self::CatastrophicError(CatastrophicError::str_to_err(s))
    }
}

impl<E> From<E> for CandoError
where
    E: Error + Send + Sync + 'static,
{
    fn from(value: E) -> Self {
        CandoError::CatastrophicError(value.into())
    }
}

impl From<CatastrophicError> for CandoError {
    fn from(value: CatastrophicError) -> Self {
        Self::CatastrophicError(value)
    }
}

/// A catastrophic error that can't be recovered from
#[derive(Debug)]
pub enum CatastrophicError {
    /// Error from CLI usage
    Usage(Box<dyn Error + Send + Sync>),
    /// Some other unknown failure. Stores the error and a backtrace
    InternalFailure {
        err: Box<dyn Error + Send + Sync>,
        backtrace: Option<Backtrace>,
    },
}

impl CatastrophicError {
    /// Converts `s` to `InternalFailure` error with backtrace
    pub fn str_to_err(s: &str) -> Self {
        Self::InternalFailure {
            err: s.into(),
            backtrace: Some(Backtrace::capture()),
        }
    }
}

impl fmt::Display for CatastrophicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(s) => write!(f, "{s}"),
            Self::InternalFailure { err, backtrace } => {
                if let Some(bt) = backtrace {
                    if bt.status() == BacktraceStatus::Disabled {
                        write!(
                            f,
                            "Internal failure: {}. To enable backtrace set `RUST_BACKTRACE=1`",
                            err
                        )
                    } else {
                        write!(f, "Internal failure: {}\n{}", err, bt)
                    }
                } else {
                    write!(f, "Internal failure: {}\nCouldn't get Backtrace", err)
                }
            }
        }
    }
}

impl<E> From<E> for CatastrophicError
where
    E: Error + Send + Sync + 'static,
{
    fn from(value: E) -> Self {
        CatastrophicError::InternalFailure {
            err: Box::new(value),
            backtrace: Some(Backtrace::capture()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_bench_failed_has_bench_exit_code() {
        assert_eq!(
            CandoError::AnyBenchFailed.exit_code(),
            ReturnCode::BenchFailure as i32
        );
    }
}
