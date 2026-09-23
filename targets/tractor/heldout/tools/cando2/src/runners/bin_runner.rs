// © 2026 Massachusetts Institute of Technology
// MIT License

use crate::{
    bench::{BenchError, BenchInfo},
    cli::{self, BinOptions},
    log,
    runner::RunAllTestResult,
    runners::runner::{self, parse_exit_status, CommonOutput, Runner, TestOutcome},
    CandoError, CatastrophicError, HarnessContext, LogLevel, RunnableState, TestVector,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    collections::HashMap,
    fmt::Debug,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    time::Duration,
};
use wait_timeout::ChildExt;

/// Main entrypoint for running a binary test
///
/// We return all of the results to make testing easier
pub fn conduct_bin(raw_args: &[&str]) -> Result<RunAllTestResult<(), ()>, CandoError> {
    let opts = cli::parse_args(&raw_args)?;
    log::set_log_level(opts.log_level);

    let context = HarnessContext::discover(opts.test_root_dir)?;
    log!(LogLevel::VERBOSE, "Using context: {:?}", context);

    match opts.subcommand {
        cli::TopLevelSubcommand::Lib(_) => Err(CandoError::usage_err(
            "Running cando like this should only be used for binary tests",
        )),
        cli::TopLevelSubcommand::Bin(bopts) => {
            let bin_runner = BinRunner::new(bopts, context.clone(), opts.rust)?;
            let res = runner::run_all_tests::<(), ()>(
                &context.test_vector_dir,
                opts.vectors,
                bin_runner,
                opts.write_json,
                false,
                opts.timeout,
                opts.bench,
            )?;
            res.output_report(opts.output)?;
            Ok(res)
        }
    }
}

pub struct BinRunner {
    bin_path: PathBuf,
}

impl BinRunner {
    /// Creates instance of `BinRunner`
    ///
    /// Fails if the given binary path doesn't exist
    pub fn new(
        bin_opts: BinOptions,
        context: HarnessContext,
        run_rust: bool,
    ) -> Result<Self, CandoError> {
        let bin_path = get_bin_path(&bin_opts.name, run_rust, &context)?;
        Ok(Self { bin_path })
    }
}

/// Parses the exit status from the binary runner to determine if a failure occurred
fn check_rc<
    S: RunnableState<E> + Clone + PartialEq + PartialEq<E>,
    E: DeserializeOwned + Serialize + Clone + Debug,
>(
    exit_status: ExitStatus,
    expected_rc: Option<i32>,
    common_out: CommonOutput,
) -> Result<TestOutcome<S, E>, CatastrophicError> {
    // If we get an error from parsing the exit status turn it into an Ok variable and
    // propgate because checking the return code doesn't make sense
    let got_rc = match parse_exit_status::<S, E>(exit_status, common_out.clone()) {
        Ok(code) => code,
        Err(res) => return Ok(res),
    };

    // Now just check if rc is correct then pass stdout/err to caller
    // We just need to check if the rc is right then pass stdout/err to the caller
    let expected_rc = expected_rc.unwrap_or(0);

    if got_rc != expected_rc {
        let diff = String::from(format!(
            "Return code mismatch. Expected: {}, Got: {}",
            expected_rc, got_rc,
        ));
        Ok(TestOutcome::VectorComparisonFailed {
            diff,
            common_out: common_out,
        })
    } else {
        Ok(TestOutcome::Pass(common_out))
    }
}

impl<
        S: RunnableState<E> + Clone + PartialEq + PartialEq<E>,
        E: DeserializeOwned + Serialize + Clone + Debug,
    > Runner<S, E> for BinRunner
{
    fn run(
        &self,
        test_vector: Option<&TestVector<S, E>>,
        timeout: Option<u64>,
        bench_info: BenchInfo,
    ) -> Result<TestOutcome<S, E>, CatastrophicError> {
        match test_vector {
            None => Err(CatastrophicError::str_to_err(
                "Binary tests currently don't support generating test vectors similar to library tests",
            )),
            Some(vector) => {
                let output = fork_and_run(
                    &self.bin_path,
                    &vector.argv,
                    &vector.stdin,
                    &vector.env,
                    timeout,
                    &bench_info,
                )?;

                if output.timed_out {
                    return Ok(TestOutcome::Timeout(output.common_out));
                }
                let bench_info_json = match bench_info.to_json() {
                    Ok(bench_info_json) => bench_info_json,
                    Err(BenchError::BenchFail(message)) => {
                        return Ok(TestOutcome::BenchFail(message));
                    }
                    Err(BenchError::Internal(err)) => return Err(err),
                };
                if let Some(bench_info_json) = bench_info_json {
                    return Ok(TestOutcome::Benchmark(bench_info_json));
                }

                check_rc(output.exit_status, vector.rc, output.common_out)
            }
        }
    }
}

#[derive(Debug)]
struct BinRunOutput {
    exit_status: ExitStatus,
    common_out: CommonOutput,
    timed_out: bool,
}

/// Run `bin` in a subprocess
fn fork_and_run(
    bin: &Path,
    argv: &Option<Vec<String>>,
    stdin: &Option<String>,
    env: &Option<HashMap<String, String>>,
    timeout: Option<u64>,
    bench_info: &BenchInfo,
) -> Result<BinRunOutput, CatastrophicError> {
    let mut cmd = match bench_info.get_bench_wrapper() {
        Some(mut c) => {
            c.arg(bin);
            c
        }
        None => Command::new(bin),
    };

    if let Some(args) = argv {
        cmd.args(args);
    }
    log!(LogLevel::VERBOSE, "Running subprocess: {:?}", cmd);

    let cmd = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(env) = env {
        cmd.envs(env);
    }

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => match bench_info.map_spawn_io_error(&err) {
            Some(BenchError::Internal(err)) => return Err(err),
            Some(BenchError::BenchFail(message)) => {
                return Err(CatastrophicError::str_to_err(&message));
            }
            None => return Err(err.into()),
        },
    };

    // Write to stdin
    if let Some(s) = stdin {
        let mut stdin_handle = child.stdin.take().ok_or_else(|| {
            CatastrophicError::str_to_err("Couldn't get stdin handle for child process")
        })?;
        stdin_handle.write_all(s.as_bytes())?;
    }

    // Kill the child after the set timeout if it isn't done yet
    let timed_out = if let Some(timeout) = timeout {
        match child.wait_timeout(Duration::from_secs(timeout))? {
            Some(_) => false,
            None => {
                child.kill()?;
                true
            }
        }
    } else {
        false
    };

    let output = child.wait_with_output()?;
    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;

    Ok(BinRunOutput {
        exit_status: output.status,
        common_out: CommonOutput { stdout, stderr },
        timed_out,
    })
}

/// Get's the full path to the binary to be run
///
/// # Args
///
/// `name`: name of the binary to test. If absolute then just returns that, otherwise return path
/// of `name` relative to appropriate build directory
/// `run_rust`: if we're looking for the Rust or C build directory
/// `context`: path context
///
/// # Returns
///
/// The full path, or `Err(CandoError::ArtifactNotFound)` if the full path doesn't exit
fn get_bin_path(
    name: &str,
    run_rust: bool,
    context: &HarnessContext,
) -> Result<PathBuf, CandoError> {
    let path = Path::new(name);
    let new_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        context.get_build_dir(run_rust).join(path).to_path_buf()
    };

    if new_path.exists() {
        Ok(new_path)
    } else {
        let path_str = new_path
            .to_str()
            .ok_or_else(|| CandoError::str_to_err("Couldn't convert binary PathBuf to str"))?;
        Err(CandoError::ArtifactNotFound(path_str.to_string()))
    }
}
