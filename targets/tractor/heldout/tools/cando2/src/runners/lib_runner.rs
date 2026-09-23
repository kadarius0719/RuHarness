// © 2026 Massachusetts Institute of Technology
// MIT License

use {
    crate::{
        bench::{BenchError, BenchInfo},
        cli::LibOptions,
        common::HarnessContext,
        error::{CandoError, CatastrophicError, ReturnCode},
        log,
        log::LogLevel,
        runners::runner::{parse_exit_status, CommonOutput, Runner, TestOutcome},
        test_vector::Output,
        RunnableState, TestVector, RUN_RUST, TEST_ROOT_DIR,
    },
    libloading::Library,
    process_fun::sys::Signal::SIGABRT,
    procspawn::SpawnError,
    serde::{de::DeserializeOwned, Serialize},
    std::{
        collections::HashMap,
        fmt::Debug,
        fs::{self, File},
        io::{Read, Write},
        marker::PhantomData,
        os::unix::process::ExitStatusExt,
        path::PathBuf,
        process::{ExitStatus, Stdio},
        time::Duration,
    },
    tempfile::NamedTempFile,
};

fn map_benchmark_spawn_error(
    bench_info: &BenchInfo,
    err: &procspawn::SpawnError,
) -> Option<BenchError> {
    bench_info.map_spawn_error(err)
}

/// Returns the full lib name depending on the platform
/// Exits if the platform isn't supported
fn get_full_lib_name(stem: &str) -> String {
    if cfg!(target_os = "linux") {
        format!("lib{}.so", stem)
    } else if cfg!(target_os = "macos") {
        format!("lib{}.dylib", stem)
    } else {
        CandoError::usage_err("Platform not support. Only supports `linux` and `macos`".into())
            .exit()
    }
}

/// Dynamically load a shared library
///
/// # Args
///
/// `context`: the context of the current harness
/// `lib_name`: string representation of the name of the library to load. Note that this adds `lib`
/// and the appropriate extension (`.so` on Linux or `.dylib` on Mac), so if `lib_name = cool` on
/// Linux this will load the library `libcool.so`, and on Mac `libcool.dylib`.
/// `run_rust`: if we're looking for a Rust library or a C library. Looks for library in `build-ninja`
/// if false or `translated_rust/target/release` if true.
///
/// # Returns
///
/// Ok(library): the loaded library
pub fn get_library(
    context: &HarnessContext,
    lib_name: &str,
    run_rust: bool,
) -> Result<Library, CandoError> {
    let build_dir = context.get_build_dir(run_rust);
    let lib = build_dir.join(get_full_lib_name(lib_name));
    if !lib.exists() {
        let lib_str = lib
            .to_str()
            .ok_or_else(|| CandoError::str_to_err("Couldn't convert library PathBuf to str"))?;
        Err(CandoError::ArtifactNotFound(lib_str.to_string()))
    } else {
        let library = unsafe { Library::new(lib)? };
        Ok(library)
    }
}

/// Represents the output of a library test run
#[derive(Clone, Debug)]
struct LibRunOutput<S: RunnableState<E>, E> {
    state: Option<S>,
    timed_out: bool,
    exit_status: Option<ExitStatus>,
    common_out: CommonOutput,

    /// Using phantom here to stop the compiler from complaining. The `RunnableState` trait needs
    /// to the type E, but this struct doesn't use it
    _phantom: PhantomData<E>,
}

impl<S: RunnableState<E>, E> LibRunOutput<S, E> {
    fn new(
        state: Option<S>,
        timed_out: bool,
        exit_status: Option<ExitStatus>,
        common_out: CommonOutput,
    ) -> Self {
        Self {
            state,
            timed_out,
            exit_status,
            common_out,
            _phantom: PhantomData,
        }
    }
}

/// Convert the result we got from procspawn into a `LibRunOutput` trying
/// to recover as much information about a failure as we can
fn handle_spawn_res<S: RunnableState<E> + DeserializeOwned, E>(
    spawn_res: Result<S, SpawnError>,
    common_out: CommonOutput,
    bench_info: &BenchInfo,
) -> Result<LibRunOutput<S, E>, BenchError> {
    match spawn_res {
        Ok(s) => Ok(LibRunOutput::new(Some(s), false, None, common_out)),
        Err(e) if e.is_timeout() => Ok(LibRunOutput::new(None, true, e.exit_status(), common_out)),
        Err(e) => {
            if let Some(err) = map_benchmark_spawn_error(bench_info, &e) {
                return Err(err);
            }

            // A panic in the subprocess is only captured like this if the panic happened in the
            // Rust part of the `state.run()`, as panics accross the boundary of a dynamic library
            // won't be caught. Because of this we can assume that any panics that are caught here
            // are internal failures, and not the result of the test cases themselves. So, we raise
            // this as an InternalFailure
            if let Some(panic_info) = e.panic_info() {
                Err(BenchError::internal_str(&format!(
                    "Panic occurred in `state.run()` NOT because of a test vector failure. Message: {}",
                    panic_info.to_string()
                )))
            } else if let Some(exit_status) = e.exit_status() {
                // We may have some information about how the process exited. If we do (i.e., `e.exit_status().is_some()`)
                // then we can propogate that result to the caller to do checks for panics/segfaults
                // within the test case
                Ok(LibRunOutput::new(
                    None,
                    false,
                    Some(exit_status),
                    common_out,
                ))
            } else {
                // Otherwise it's an unrecoverable error so propogate it
                Err(BenchError::internal(e))
            }
        }
    }
}

/// Runs `state.run()` in a child process
///
/// # Args
///
/// `test_root_dir`: the root directory of the test case to initialize statics in child
/// `state`: the state defined in the `harness!` macro
/// `stdin`: input to pass to child process as defined by the test vector
/// `timeout`: maximum allotted time to run an individual test_vector
/// `rust`: if this should run Rust or C to initialize statics in child
///
/// # Returns
///
/// `RunOutput` that contains the state after being run and stdout/err
fn fork_and_run<S: RunnableState<E> + DeserializeOwned, E>(
    test_root_dir: PathBuf,
    state: S,
    stdin: Option<String>,
    env: Option<HashMap<String, String>>,
    timeout: Option<u64>,
    run_rust: bool,
    bench_info: &BenchInfo,
) -> Result<LibRunOutput<S, E>, BenchError> {
    // The child will write stdout/err to files so the main thread doesn't get blocked waiting for
    // results from stdout/err, and can actually handle the timeout
    let stdout_file = NamedTempFile::new().map_err(BenchError::internal)?;
    let stderr_file = NamedTempFile::new().map_err(BenchError::internal)?;

    let mut builder = procspawn::Builder::new();
    builder
        .stdin(Stdio::piped())
        .stdout(Stdio::from(
            stdout_file.reopen().map_err(BenchError::internal)?,
        ))
        .stderr(Stdio::from(
            stderr_file.reopen().map_err(BenchError::internal)?,
        ))
        .envs(env.unwrap_or_default());

    let mut handle = builder.spawn(
        (test_root_dir, state, run_rust),
        |(test_root_dir, mut state, run_rust)| {
            TEST_ROOT_DIR.get_or_init(|| Some(test_root_dir));
            RUN_RUST.get_or_init(|| run_rust);
            state.run();
            state
        },
        bench_info.get_bench_wrapper(),
    );

    // Pass stdin if we need to
    if let Some(s) = stdin {
        if !s.is_empty() {
            let stdin_handle = handle
                .stdin()
                .take()
                .ok_or_else(|| BenchError::internal_str("Couldn't get stdin handle"))?;
            stdin_handle
                .write_all(s.as_bytes())
                .map_err(BenchError::internal)?;
        }
    }

    let spawn_res = match timeout {
        Some(t) => {
            log!(LogLevel::VERBOSE, "Running with timeout: {:?}", t);
            handle.join_timeout(Duration::from_secs(t))
        }
        None => {
            log!(LogLevel::VERBOSE, "Running with no timeout");
            handle.join()
        }
    };

    // Read stdout/err from files after we've joined
    let mut stdout = String::new();
    File::open(stdout_file.path())
        .map_err(BenchError::internal)?
        .read_to_string(&mut stdout)
        .map_err(BenchError::internal)?;

    let mut stderr = String::new();
    File::open(stderr_file.path())
        .map_err(BenchError::internal)?
        .read_to_string(&mut stderr)
        .map_err(BenchError::internal)?;

    let common_out = CommonOutput { stdout, stderr };
    handle_spawn_res(spawn_res, common_out, bench_info)
}

pub struct LibRunner {
    lib_opts: LibOptions,
    context: HarnessContext,
    run_rust: bool,
}

impl LibRunner {
    /// Creates new instance of `LibRunner`, wr
    pub fn new(lib_opts: LibOptions, context: HarnessContext, run_rust: bool) -> Self {
        LibRunner {
            lib_opts,
            context,
            run_rust,
        }
    }

    /// Determines if `LibRunner` already has some state
    /// (from the options zero, pattern, random, or fuzzed)
    pub fn has_state(&self) -> bool {
        self.lib_opts.zero
            || self.lib_opts.pattern.is_some()
            || self.lib_opts.random.is_some()
            || self.lib_opts.fuzzed.is_some()
    }

    fn get_state<S: RunnableState<E>, E>(&self) -> Result<S, CatastrophicError> {
        if self.lib_opts.zero {
            Ok(S::zeroed()?)
        } else {
            let bytes = if let Some(size) = self.lib_opts.pattern {
                // FIXME: Leaving this for now but why on earth is this 85???
                vec![85; size]
            } else if let Some(size) = self.lib_opts.random {
                let mut bytes = vec![0; size];
                getrandom::fill(&mut bytes)?;
                bytes
            } else if let Some(filename) = &self.lib_opts.fuzzed {
                let fuzz_file = self.context.test_root_dir.join(filename);
                fs::read(fuzz_file)?
                // Originally cando called `.as_slice()` here. It doesn't look like that was
                // necessary but leaving this comment for future debugging
            } else {
                return Err(CatastrophicError::Usage("Got bad library options".into()));
            };

            Ok(S::from_bytes(&bytes)?)
        }
    }
}

/// On an `UnknownFailure` from the lib runner we need to check the exit
/// status (for SIGABRT) and stderr ("panic in a function that cannot unwind")
/// to determine if the failure was becuase of a Rust panic
fn check_panic<
    S: RunnableState<E> + Clone + PartialEq + PartialEq<E>,
    E: DeserializeOwned + Serialize + Clone + Debug,
>(
    test_outcome: TestOutcome<S, E>,
) -> Result<TestOutcome<S, E>, CatastrophicError> {
    // If we got an UnknownFailure we need to check if there was a
    // SIGABRT in the child because that would be a Rust panic
    // if stderr has "panic in a function that cannot unwind"
    match test_outcome {
        TestOutcome::UnknownFailure {
            common_out,
            wait_status,
        } => {
            let exit_status = ExitStatus::from_raw(wait_status);
            if exit_status.signal().is_some_and(|s| {
                s == SIGABRT as i32
                    && common_out
                        .stderr
                        .contains("panic in a function that cannot unwind")
            }) {
                return Ok(TestOutcome::Panic(common_out));
            }
            return Ok(TestOutcome::UnknownFailure {
                wait_status: exit_status.into_raw(),
                common_out,
            });
        }
        _ => return Ok(test_outcome),
    }
}

/// Handle the return code for libraries. Checks for non-zero rc and an rc for
/// `SymbolNotFound`. Otherwise just returns UnknownFailure
fn check_rc<
    S: RunnableState<E> + Clone + PartialEq + PartialEq<E>,
    E: DeserializeOwned + Serialize + Clone + Debug,
>(
    got_rc: i32,
    exit_status: ExitStatus,
    common_out: CommonOutput,
) -> Result<TestOutcome<S, E>, CatastrophicError> {
    // I suppose we could support checking if a library test exits with a
    // certain code, but library tests really shouldn't be exiting on purpose
    // so we'll just check for a non-zero return code.
    if got_rc == 0 {
        Ok(TestOutcome::Pass(common_out))
    } else if got_rc == ReturnCode::SymbolNotFound as i32 {
        // We're also going to check if the runner returns with the
        // SymbolNotFound exit code as that signifies the run function attempted
        // to get a symbol that it couldn't find in the translated Rust, but
        // was present in the oroginal C. If this is happening with the
        // original C then it's an issue with the runner. It's fine to exit
        // here because this will be the same for all test vectors.
        CandoError::SymbolNotFound(common_out.stderr.trim_end().to_owned()).exit();
    } else {
        Ok(TestOutcome::UnknownFailure {
            wait_status: exit_status.into_raw(),
            common_out: common_out,
        })
    }
}

// If we didn't get any output state from running the child process then it means
// some failure occurred, figure out the cause of the error here
fn handle_no_state<
    S: RunnableState<E> + Clone + PartialEq + PartialEq<E>,
    E: DeserializeOwned + Serialize + Clone + Debug,
>(
    exit_status: Option<ExitStatus>,
    common_out: CommonOutput,
) -> Result<TestOutcome<S, E>, CatastrophicError> {
    match exit_status {
        Some(status) => {
            // If we get an error from parsing the exit status turn it into an Ok variable and
            // propgate because checking the return code doesn't make sense
            let got_rc = match parse_exit_status::<S, E>(status, common_out.clone()) {
                Ok(code) => code,
                Err(test_outcome) => {
                    return check_panic(test_outcome);
                }
            };

            check_rc(got_rc, status, common_out)
        }
        None => Err(CatastrophicError::str_to_err(
            "Didin't get state and didn't get an exit status",
        )),
    }
}

/// Handle the different cases for the library returning a state. If there is
/// no test vector we need to create a new test vector with the output we got.
/// Otherwise we just need to check if the comparison failed or suceeded.
fn handle_state<
    S: RunnableState<E> + Clone + PartialEq + PartialEq<E>,
    E: DeserializeOwned + Serialize + Clone + Debug,
>(
    state_in: S,
    state_out: S,
    test_vector: Option<&TestVector<S, E>>,
    common_out: CommonOutput,
) -> Result<TestOutcome<S, E>, CatastrophicError> {
    if let Some(vector) = test_vector {
        // Compare state
        if vector.equals_expected(&state_out)? {
            Ok(TestOutcome::Pass(common_out))
        } else {
            // Convert the expected state and got state out to JSON strings for better
            // diff formatting
            let expected_json = serde_json::to_string(&vector.lib_state_out)?;
            let got_json = serde_json::to_string(&state_out)?;
            Ok(TestOutcome::VectorComparisonFailed {
                diff: String::from(format!(
                    "Library State Mismatch. Expected:\n{}\nGot:\n{}",
                    expected_json, got_json
                )),
                common_out: common_out,
            })
        }
    } else {
        // If there's no vector there there's nothing to compare. Create a new vector with
        // lib_state_in/out and stdout/err that we got
        let new_vector: TestVector<S, E> = TestVector {
            lib_state_in: Some(state_in),
            lib_state_out: Some(state_out.to_expected_state()),
            stdout: Some(Output {
                pattern: common_out.stdout,
                is_regex: None,
            }),
            stderr: Some(Output {
                pattern: common_out.stderr,
                is_regex: None,
            }),
            env: None,
            argv: None,
            stdin: None,
            has_ub: false,
            note: None,
            rc: None,
        };
        Ok(TestOutcome::NoCompare(Some(new_vector)))
    }
}

impl<
        S: RunnableState<E> + Clone + PartialEq + PartialEq<E>,
        E: DeserializeOwned + Serialize + Clone + Debug,
    > Runner<S, E> for LibRunner
{
    fn run(
        &self,
        test_vector: Option<&TestVector<S, E>>,
        timeout: Option<u64>,
        bench_info: BenchInfo,
    ) -> Result<TestOutcome<S, E>, CatastrophicError> {
        let (state_in, stdin, env) = match test_vector {
            Some(v) => (v.lib_state()?, v.stdin.clone(), v.env.clone()),
            None => (self.get_state::<S, E>()?, None, None),
        };

        let lib_run_out = fork_and_run(
            self.context.test_root_dir.clone(),
            state_in.clone(),
            stdin,
            env,
            timeout,
            self.run_rust,
            &bench_info,
        );
        let lib_run_out = match lib_run_out {
            Ok(out) => out,
            Err(BenchError::BenchFail(message)) => return Ok(TestOutcome::BenchFail(message)),
            Err(BenchError::Internal(err)) => return Err(err),
        };
        log!(LogLevel::VERBOSE, "{:#?}", lib_run_out);

        if lib_run_out.timed_out {
            return Ok(TestOutcome::Timeout(lib_run_out.common_out));
        }
        let bench_info_json = match bench_info.to_json() {
            Ok(bench_info_json) => bench_info_json,
            Err(BenchError::BenchFail(message)) => return Ok(TestOutcome::BenchFail(message)),
            Err(BenchError::Internal(err)) => return Err(err),
        };
        if let Some(bench_info_json) = bench_info_json {
            return Ok(TestOutcome::Benchmark(bench_info_json));
        }

        match lib_run_out.state {
            None => handle_no_state(lib_run_out.exit_status, lib_run_out.common_out),
            Some(state_out) => {
                handle_state(state_in, state_out, test_vector, lib_run_out.common_out)
            }
        }
    }
}
