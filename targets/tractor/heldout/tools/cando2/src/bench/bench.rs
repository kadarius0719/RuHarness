// © 2026 Massachusetts Institute of Technology
// MIT License

use {
    crate::{
        bench::{memory, runtime},
        cli::BenchMode,
        CatastrophicError,
    },
    procspawn::SpawnError,
    std::{error::Error, io, process::Command},
    tempfile::NamedTempFile,
};

#[derive(Debug)]
pub enum BenchError {
    Internal(CatastrophicError),
    BenchFail(String),
}

impl BenchError {
    pub fn internal<E>(err: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self::Internal(err.into())
    }

    pub fn internal_str(s: &str) -> Self {
        Self::Internal(CatastrophicError::str_to_err(s))
    }
}

#[derive(Debug)]
enum InternalBenchInfo {
    None,
    Memory,
    Runtime,
}

impl InternalBenchInfo {
    fn tool_name(&self) -> Option<&'static str> {
        match self {
            InternalBenchInfo::None => None,
            InternalBenchInfo::Memory => Some("valgrind"),
            InternalBenchInfo::Runtime => Some("perf"),
        }
    }

    fn flag_name(&self) -> Option<&'static str> {
        match self {
            InternalBenchInfo::None => None,
            InternalBenchInfo::Memory => Some("memory"),
            InternalBenchInfo::Runtime => Some("runtime"),
        }
    }
}

/// Contains information about the current benchmark we're running
#[derive(Debug)]
pub struct BenchInfo {
    /// The benchmark that we're running. If `None` not running any benchmark
    bencher: InternalBenchInfo,
    /// The file that the benchmarking output will be dumped to
    file: NamedTempFile,
}

impl BenchInfo {
    /// Creates a new bencher
    ///
    /// # Args
    ///
    /// `bench_mode`: the mode that benchmarking will be run with. If passed
    ///     `None` no benchmarking will be performed
    pub fn new(bench_mode: Option<BenchMode>) -> Result<Self, CatastrophicError> {
        let bencher = match bench_mode {
            None => InternalBenchInfo::None,
            Some(BenchMode::Memory) => InternalBenchInfo::Memory,
            Some(BenchMode::Runtime) => InternalBenchInfo::Runtime,
        };
        Ok(Self {
            bencher,
            file: NamedTempFile::new()?,
        })
    }

    /// Prepares a command for benchmarking
    ///
    /// # Returns
    ///
    /// A command that can have an executable and more arguments added after it to
    /// run that binary under the given `bench_mode`
    pub fn get_bench_wrapper(&self) -> Option<Command> {
        let out_file = self.file.path();
        match &self.bencher {
            InternalBenchInfo::None => None,
            InternalBenchInfo::Memory => Some(memory::get_wrapper_cmd(out_file)),
            InternalBenchInfo::Runtime => Some(runtime::get_wrapper_cmd(out_file)),
        }
    }

    pub fn map_spawn_io_error(&self, err: &io::Error) -> Option<BenchError> {
        let tool = self.bencher.tool_name()?;
        let flag = self.bencher.flag_name()?;

        let message = if err.kind() == io::ErrorKind::NotFound {
            format!(
                "Benchmarking with `--bench {flag}` requires `{tool}`, but it was not found in `PATH`."
            )
        } else {
            format!("Failed to start `{tool}` for `--bench {flag}`: {err}")
        };

        Some(BenchError::Internal(CatastrophicError::str_to_err(
            &message,
        )))
    }

    pub fn map_spawn_error(&self, err: &SpawnError) -> Option<BenchError> {
        let flag = self.bencher.flag_name()?;

        if err.is_remote_close() {
            return Some(BenchError::BenchFail(format!(
                "Benchmarking with `--bench {flag}` failed because the child process exited before benchmark output could be collected."
            )));
        }

        err.io_error()
            .and_then(|io_err| self.map_spawn_io_error(io_err))
    }

    /// Converts the benchmarking output into JSON. If `BenchInfo` was created with
    /// `bench_mode == None` then this will return Ok(None)
    pub fn to_json(self) -> Result<Option<serde_json::Value>, BenchError> {
        let Self { bencher, file } = self;

        match bencher {
            InternalBenchInfo::None => Ok(None),
            InternalBenchInfo::Memory => {
                let ret = memory::to_json(file)?;
                Ok(Some(
                    serde_json::to_value(ret).map_err(BenchError::internal)?,
                ))
            }
            InternalBenchInfo::Runtime => {
                let ret = runtime::to_json(file).map_err(BenchError::Internal)?;
                Ok(Some(
                    serde_json::to_value(ret).map_err(BenchError::internal)?,
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_valgrind_gets_specific_error() {
        let bench_info = BenchInfo::new(Some(BenchMode::Memory)).unwrap();
        let err = bench_info
            .map_spawn_io_error(&io::Error::new(io::ErrorKind::NotFound, "missing"))
            .unwrap();
        match err {
            BenchError::Internal(err) => assert!(format!("{err}").contains("requires `valgrind`")),
            BenchError::BenchFail(_) => panic!("missing valgrind should be internal"),
        }
    }

    #[test]
    fn missing_perf_gets_specific_error() {
        let bench_info = BenchInfo::new(Some(BenchMode::Runtime)).unwrap();
        let err = bench_info
            .map_spawn_io_error(&io::Error::new(io::ErrorKind::NotFound, "missing"))
            .unwrap();
        match err {
            BenchError::Internal(err) => assert!(format!("{err}").contains("requires `perf`")),
            BenchError::BenchFail(_) => panic!("missing perf should be internal"),
        }
    }

    #[test]
    fn non_benchmark_spawn_errors_are_ignored() {
        let bench_info = BenchInfo::new(None).unwrap();
        assert!(bench_info
            .map_spawn_io_error(&io::Error::new(io::ErrorKind::NotFound, "missing"))
            .is_none());
    }

    #[test]
    fn remote_close_becomes_bench_fail() {
        let bench_info = BenchInfo::new(Some(BenchMode::Runtime)).unwrap();
        let err = SpawnError::from(io::Error::new(
            io::ErrorKind::ConnectionReset,
            "remote closed",
        ));

        match bench_info.map_spawn_error(&err).unwrap() {
            BenchError::BenchFail(message) => {
                assert!(message.contains("child process exited before benchmark output"));
            }
            BenchError::Internal(_) => panic!("remote close should be benchmark failure"),
        }
    }

    #[test]
    fn empty_memory_benchmark_output_becomes_bench_fail() {
        let bench_info = BenchInfo::new(Some(BenchMode::Memory)).unwrap();

        match bench_info.to_json().unwrap_err() {
            BenchError::BenchFail(message) => {
                assert!(message.contains("`--bench memory`"));
                assert!(
                    message.contains("may have exited before benchmark output could be collected")
                );
            }
            BenchError::Internal(_) => panic!("empty massif output should be benchmark failure"),
        }
    }
}
