// © 2026 Massachusetts Institute of Technology
// MIT License

//! Generic runner functions relevant to both library and binary tests

use {
    crate::{
        bench::BenchInfo,
        cli::BenchMode,
        error::{CandoError, CatastrophicError},
        log,
        test_vector::Output,
        LogLevel, RunnableState, TestVector,
    },
    diffy::{DiffOptions, PatchFormatter},
    process_fun::sys::Signal::SIGSEGV,
    regex::Regex,
    serde::{de::DeserializeOwned, ser::SerializeStruct, Serialize, Serializer},
    std::{
        collections::HashMap,
        fmt::Debug,
        fs,
        os::unix::process::ExitStatusExt,
        path::{Path, PathBuf},
        process::ExitStatus,
    },
};

/// The result of an individual test vector run
#[derive(Debug, Clone, PartialEq)]
pub enum TestOutcome<S: RunnableState<E> + PartialEq + Clone, E: Clone + Serialize + Debug> {
    /// The vector was skipped
    Skip,
    /// The vector was run, but nothing was compared. The value is `Some(v)` if we collected some
    /// information about a test vector (that can be written to disk with `write_json` option
    /// specified), if None we didn't collect anything
    NoCompare(Option<TestVector<S, E>>),
    /// The vector ran and initially passes
    Pass(CommonOutput),
    /// The vector ran but failed some comparison within the vector
    VectorComparisonFailed {
        diff: String,
        common_out: CommonOutput,
    },
    /// The vector ran but triggered a Rust panic
    Panic(CommonOutput),
    /// The vector ran but triggered a segmentation fault
    SegmentationFault(CommonOutput),
    /// The vector ran but timed out
    Timeout(CommonOutput),
    /// Benchmarking failed because the benchmarked process exited too early
    BenchFail(String),
    /// Output from benchmarking, generic JSON type
    Benchmark(serde_json::Value),
    /// The vector ran but trigger some other signal/failure
    UnknownFailure {
        /// NOTE: This is the wait status NOT the return code
        wait_status: i32,
        common_out: CommonOutput,
    },
}

/// Custom serializer implementation to handle options and formatting better
impl<S, E> Serialize for TestOutcome<S, E>
where
    S: RunnableState<E> + PartialEq + Clone,
    E: Clone + Serialize + Debug,
{
    fn serialize<SER>(&self, serializer: SER) -> Result<SER::Ok, SER::Error>
    where
        SER: Serializer,
    {
        match self {
            TestOutcome::Skip => {
                let mut state = serializer.serialize_struct("RunResult", 1)?;
                state.serialize_field("result", "Skip")?;
                state.end()
            }
            TestOutcome::NoCompare(test_vector) => {
                let mut state = serializer.serialize_struct("RunResult", 2)?;
                state.serialize_field("result", "NoCompare")?;
                if let Some(v) = test_vector {
                    state.serialize_field("output", v)?;
                }
                state.end()
            }
            TestOutcome::Pass(output) => {
                let mut state = serializer.serialize_struct("RunResult", 2)?;
                state.serialize_field("result", "Pass")?;
                state.serialize_field("output", output)?;
                state.end()
            }
            TestOutcome::VectorComparisonFailed { diff, common_out } => {
                let mut state = serializer.serialize_struct("RunResult", 3)?;
                state.serialize_field("result", "VectorComparisonFailed")?;
                state.serialize_field("diff", diff)?;
                state.serialize_field("output", common_out)?;
                state.end()
            }
            TestOutcome::Panic(output) => {
                let mut state = serializer.serialize_struct("RunResult", 2)?;
                state.serialize_field("result", "Panic")?;
                state.serialize_field("output", output)?;
                state.end()
            }
            TestOutcome::SegmentationFault(output) => {
                let mut state = serializer.serialize_struct("RunResult", 2)?;
                state.serialize_field("result", "SegmentationFault")?;
                state.serialize_field("output", output)?;
                state.end()
            }
            TestOutcome::Timeout(output) => {
                let mut state = serializer.serialize_struct("RunResult", 2)?;
                state.serialize_field("result", "Timeout")?;
                state.serialize_field("output", output)?;
                state.end()
            }
            TestOutcome::BenchFail(message) => {
                let mut state = serializer.serialize_struct("RunResult", 2)?;
                state.serialize_field("result", "BenchFail")?;
                state.serialize_field("message", message)?;
                state.end()
            }
            TestOutcome::Benchmark(output) => {
                let mut state = serializer.serialize_struct("RunResult", 2)?;
                state.serialize_field("result", "Benchmark")?;
                state.serialize_field("output", output)?;
                state.end()
            }
            TestOutcome::UnknownFailure {
                common_out,
                wait_status,
            } => {
                let mut state = serializer.serialize_struct("RunResult", 3)?;
                state.serialize_field("result", "UnknownFailure")?;

                state.serialize_field("wait_status", wait_status)?;
                state.serialize_field("output", common_out)?;
                state.end()
            }
        }
    }
}

impl<S: RunnableState<E> + PartialEq + Clone, E: Clone + Serialize + Debug> TestOutcome<S, E> {
    /// `True` if the `RunResult` is not a failure of any kind
    fn fail(&self) -> bool {
        match self {
            Self::Skip | Self::NoCompare(_) | Self::Pass(_) | Self::Benchmark(_) => false,
            _ => true,
        }
    }

    fn bench_fail(&self) -> bool {
        matches!(self, Self::BenchFail(_))
    }
}

/// Wrapper around diffy to format diffs.
/// Creates a colored diff using expected and got rather
/// than original and modified
pub fn get_diff(expected: &str, got: &str) -> String {
    let patch = DiffOptions::new()
        .set_original_filename("expected")
        .set_modified_filename("got")
        .create_patch(expected, got);
    let f = PatchFormatter::new().with_color();
    format!("{}", f.fmt_patch(&patch))
}

/// Gets diff of `expected` vs `got`.
///
/// Evaluates as regex if specified in `expected.is_regex`, and not if otherwise
/// gets a colored diff (if any) from diffy
///
/// # Returns
///
/// `Some(s)`: `s` is the string representation of a diff
/// `None`: if there is no diff
fn get_output_diff(expected: &Output, got: &str) -> Result<Option<String>, CatastrophicError> {
    if expected.is_regex.is_some_and(|r| r) {
        let re = Regex::new(&expected.pattern)?;

        if re.is_match(got) {
            Ok(None)
        } else {
            Ok(Some(format!(
                "Regex didn't match.\nExpected regex:\n`{}`\nGot:\n`{}`",
                expected.pattern, got
            )))
        }
    } else {
        if expected.pattern == got {
            Ok(None)
        } else {
            Ok(Some(get_diff(&expected.pattern, got)))
        }
    }
}

/// If `expected` is `None` then returns new `Output` with pattern = ""
fn get_expected(expected: Option<Output>) -> Output {
    match expected {
        Some(e) => e,
        None => Output {
            pattern: String::from(""),
            is_regex: Some(false),
        },
    }
}

/// Stored stdout/stderr from the output of either a binary or library test.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CommonOutput {
    pub stdout: String,
    pub stderr: String,
}

impl CommonOutput {
    /// Compares stdout/err to their specified values in the test vector
    ///
    /// # Args
    ///
    /// `current_diff`: the current diff at this point in the runtime (if `None` there's no diff)
    /// `expected_stdout`: the expected stdout from the test vector
    /// `expected_stderr`: the expected stderr from the test vector
    ///
    /// # Returns
    ///
    /// Ok(None): if there are no diffs
    /// Ok(Some(diff)): string representation of the diff, (adding to the existing `current_diff`
    /// if it is `Some`)
    fn compare(
        &self,
        current_diff: Option<String>,
        expected_stdout: &Option<Output>,
        expected_stderr: &Option<Output>,
    ) -> Result<Option<String>, CatastrophicError> {
        let expected_stdout = get_expected(expected_stdout.clone());
        let stdout_diff = get_output_diff(&expected_stdout, &self.stdout)?;

        let expected_stderr = get_expected(expected_stderr.clone());
        let stderr_diff = get_output_diff(&expected_stderr, &self.stderr)?;

        // Start from current_diff or empty string
        let mut diff = current_diff.unwrap_or_default();

        if let Some(stdout_diff) = stdout_diff {
            if !diff.is_empty() {
                diff.push('\n');
            }
            diff.push_str("Stdout diff: ");
            diff.push_str(&stdout_diff);
        }
        if let Some(stderr_diff) = stderr_diff {
            if !diff.is_empty() {
                diff.push('\n');
            }
            diff.push_str("Stderr diff: ");
            diff.push_str(&stderr_diff);
        }

        if diff.is_empty() {
            Ok(None)
        } else {
            Ok(Some(diff))
        }
    }
}

/// Generic runner trait (implemented by both lib_runner and bin_runner)
pub trait Runner<S: RunnableState<E> + PartialEq + Clone, E: Clone + Serialize + Debug> {
    /// Run a `test_vector`.
    ///
    /// If `test_vector` is None then there should be some other way for the runner
    /// to run something. ATM this is only relevant for library tests that can initialize
    /// state from options like --zero, or --fuzzed, and return a CatastrophicError error
    /// for binary tests.
    fn run(
        &self,
        test_vector: Option<&TestVector<S, E>>,
        timeout: Option<u64>,
        bench_mode: BenchInfo,
    ) -> Result<TestOutcome<S, E>, CatastrophicError>;
}

/// Checks if FAKETIME is in the env and if so load the libfaketime.so
fn handle_faketime(env: &mut Option<HashMap<String, String>>) -> Result<(), CatastrophicError> {
    if let Some(env) = env {
        if env.contains_key("FAKETIME") {
            let libfaketime_path =
                        std::env::var("LIBFAKETIME_SO")
                        .map_err(|_| CatastrophicError::str_to_err("Couldn't get path to `libfaketime`. Should be specified as `LIBFAKETIME_SO` in flake."))?;
            env.insert(String::from("LD_PRELOAD"), libfaketime_path);
        }
    }
    Ok(())
}

/// Returns the appropriate `TestOutcome` from what was returned
fn handle_test_outcome<
    S: RunnableState<E> + PartialEq + PartialEq<E> + Clone,
    E: Debug + Clone + DeserializeOwned + Serialize,
>(
    test_outcome: TestOutcome<S, E>,
    expected_stdout: &Option<Output>,
    expected_stderr: &Option<Output>,
    bench_mode: Option<BenchMode>,
) -> Result<TestOutcome<S, E>, CatastrophicError> {
    match test_outcome {
        TestOutcome::Panic(_)
        | TestOutcome::SegmentationFault(_)
        | TestOutcome::UnknownFailure {
            wait_status: _,
            common_out: _,
        }
        | TestOutcome::Timeout(_)
        | TestOutcome::BenchFail(_) => {
            // Any of these results mean that some problem occurred trying to run
            // so a comparison of stdout/err wouldn't make much sense
            Ok(test_outcome)
        }
        TestOutcome::Skip | TestOutcome::NoCompare(_) => Err(CatastrophicError::str_to_err(
            "`Skip` and `NoCompare` shouldn't be reached here",
        )),
        TestOutcome::Pass(c) => Ok(match c.compare(None, expected_stdout, expected_stderr)? {
            Some(diff) => TestOutcome::VectorComparisonFailed {
                diff,
                common_out: c,
            },
            None => TestOutcome::Pass(c),
        }),
        TestOutcome::VectorComparisonFailed {
            diff: current_diff,
            common_out: c,
        } => match c.compare(Some(current_diff), expected_stdout, expected_stderr)? {
            Some(diff) => Ok(TestOutcome::VectorComparisonFailed {
                diff,
                common_out: c,
            }),
            None => Err(CatastrophicError::str_to_err(
                "Comparing stdout/stderr with an existing diff shouldn't pass",
            )),
        },
        TestOutcome::Benchmark(b) => match bench_mode {
            Some(_) => Ok(TestOutcome::Benchmark(b)),
            None => Err(CatastrophicError::str_to_err(
                "Vector returned with `TestOutcome::Benchmark` but user didn't specify `--bench`",
            )),
        },
    }
}

/// Runs a single test vector.
///
/// # Args
///
/// `vector`: the test vector to run. If `None` then just use the generated state (from options
/// like --zero or --fuzzed)
/// `runner`: struct that implements Runner trait to actually run the test vector
/// `timeout`: maximum allotted time to run an individual test_vector
///
/// # Returns
///
/// Ok(res): the result of the run (e.g., pass, vector comparison failed, artifact not found, ...)
fn run_test<
    S: RunnableState<E> + PartialEq + PartialEq<E> + Clone,
    E: Debug + Clone + DeserializeOwned + Serialize,
>(
    vector: Option<TestVector<S, E>>,
    runner: &impl Runner<S, E>,
    timeout: Option<u64>,
    bench_mode: Option<BenchMode>,
) -> Result<TestOutcome<S, E>, CatastrophicError> {
    match vector {
        Some(mut v) => {
            // If it's UB we skip it, for now...
            if v.has_ub {
                log!(LogLevel::VERBOSE, "Skipping because of UB");
                return Ok(TestOutcome::Skip);
            }

            handle_faketime(&mut v.env)?;

            let bench_info = BenchInfo::new(bench_mode)?;
            let test_outcome = runner.run(Some(&v), timeout, bench_info)?;

            handle_test_outcome(test_outcome, &v.stdout, &v.stderr, bench_mode)
        }
        None => {
            // This sort of run should never be called with benchmarking
            let bench_info = BenchInfo::new(None)?;
            let res = runner.run(None, timeout, bench_info)?;

            // There's nothing to compare without a full test vector so we just make sure the
            // runner did the right thing by marking it as `NoCompare` and raise to to the caller
            match res {
                TestOutcome::NoCompare(_) => Ok(res),
                _ => Err(CatastrophicError::str_to_err(
                    "A run with no test vector should return `NoCompare`",
                )),
            }
        }
    }
}

/// Stores the combined result of all the test vectors that were run
pub struct RunAllTestResult<S: RunnableState<E> + PartialEq + Clone, E: Clone + Serialize + Debug> {
    /// The internal representation of all the test vectors that were run, where the key is the
    /// name of the test vector (if it is an unnamed test vector such as from using --zero then the
    /// key is None (which is serialized as "null")).
    pub report: HashMap<Option<String>, TestOutcome<S, E>>,
    /// Boolean for if any of the test vectors during the run failed because of something like a
    /// vector comparison failing, segfault, etc..., just not things like not comparing ouputs (for
    /// options like --zero) or skipping vectors
    pub any_vector_failed: bool,
    /// Boolean for if any benchmark run failed before results could be collected
    pub any_bench_failed: bool,
}

impl<S, E> RunAllTestResult<S, E>
where
    S: RunnableState<E> + PartialEq + Clone,
    E: Clone + Serialize + Debug,
{
    /// Outputs `self.report` to stdout and an optional `file`
    ///
    /// Converts all keys == `None` to be `null` in JSON. If `NoCompare` value is `None` it just
    /// sets that to `NoCompare`
    ///
    /// Pretty-prints the output if the log level isn't quiet, otherwise prints properly escaped
    /// JSON that can be injested by a higher level orchestrator
    pub fn output_report(&self, out_file: Option<PathBuf>) -> Result<(), CandoError> {
        let report: HashMap<_, _> = self
            .report
            .iter()
            .map(|(key, val)| {
                let new_key = key.clone().unwrap_or_else(|| String::from("null"));
                (new_key, val)
            })
            .collect();

        // Print report as JSON (pretty printing if we're not in LogLevel::QUIET)
        log!(LogLevel::QUIET, "{}", serde_json::to_string(&report)?);

        let json_str = serde_json::to_string_pretty(&report)?;
        log!(LogLevel::NORMAL, "{}", json_str);

        if let Some(f) = out_file {
            fs::write(f, json_str)?;
        }

        Ok(())
    }
}

/// Runs all test vectors for a test case, and prints a report of all vectors.
///
/// # Args
///
/// `test_vector_dir`: path to `test_vectors` directory
/// `vector_names`: list of filenames relative to `test_vector_dir`
/// `runner`: runner capable of running individual test case (implements Runner trait)
/// `write_json`: if when running these vectors should we write them to a file
/// `has_state`: if the runner already has some `state` (a library runner can initialize state from
/// --zero or --fuzzed for example)
/// `timeout`: maximum allotted time to run an individual test_vector
///
/// # Returns
///
/// The HashMap report for all of the vectors that were run, along with a boolean for if any of
/// those vectors failed to run (because of something like a comparison failure, segfault, etc...)
/// not because of something being skipped or not compared.
pub fn run_all_tests<
    S: RunnableState<E> + Clone + PartialEq + PartialEq<E>,
    E: Debug + DeserializeOwned + Serialize + Clone,
>(
    test_vector_dir: &Path,
    vector_names: Vec<String>,
    runner: impl Runner<S, E>,
    write_json: Option<String>,
    has_state: bool,
    timeout: Option<u64>,
    bench_mode: Option<BenchMode>,
) -> Result<RunAllTestResult<S, E>, CatastrophicError> {
    // If the runner already has state (from lib options like --zero, etc...)
    // the there's no need to load any vectors
    if has_state {
        let run_result = run_test::<S, E>(None, &runner, timeout, None)?;

        // If the result was a NoCompare let's check if we should write it to disk
        if let TestOutcome::NoCompare(Some(vector)) = &run_result {
            if let Some(ref v_name) = write_json {
                vector.write_to_vector_dir(test_vector_dir, &v_name)?;
            }
        }

        let report = HashMap::from([(None, run_result)]);
        return Ok(RunAllTestResult {
            report,
            any_vector_failed: false,
            any_bench_failed: false,
        });
    }

    // Otherwise we need to load the test vectors
    let test_vectors = TestVector::load_vectors(test_vector_dir, vector_names)?;
    let mut report = HashMap::new();
    let mut vector_fail = false;
    let mut bench_fail = false;

    for (name, vector) in test_vectors {
        log!(LogLevel::VERBOSE, "Vector name: {:#?}", name);
        log!(LogLevel::VERBOSE, "Vector: {:#?}", vector);

        let run_result = run_test(Some(vector), &runner, timeout, bench_mode)?;
        log!(LogLevel::VERBOSE, "Result: {:#?}", run_result);

        report.insert(Some(name.clone()), run_result.clone());

        if run_result.fail() {
            vector_fail = true;
        }
        if run_result.bench_fail() {
            bench_fail = true;
        }
    }

    Ok(RunAllTestResult {
        report,
        any_vector_failed: vector_fail,
        any_bench_failed: bench_fail,
    })
}

/// Parses the `exit_status` and `stdout` / `stderr` of the child process
///
/// Determines if the `ExitStatus` is the result of a SegmentationFault, or Rust panic (using
/// return code 101), and returns those results as Err(RunResult). If it's not a failure due to a
/// signal, or a 101 rc then it returns the rc for use by the caller
pub fn parse_exit_status<S: RunnableState<E> + PartialEq + Clone, E: Clone + Serialize + Debug>(
    exit_status: ExitStatus,
    common_out: CommonOutput,
) -> Result<i32, TestOutcome<S, E>> {
    match exit_status.code() {
        None => {
            // If the exit code is None it means that the program was interupted by a
            // signal. For right now we only care about SIGSEGV
            if exit_status
                .signal()
                .is_some_and(|signal| signal == SIGSEGV as i32)
            {
                return Err(TestOutcome::SegmentationFault(common_out));
            } else {
                return Err(TestOutcome::UnknownFailure {
                    wait_status: exit_status.into_raw(),
                    common_out,
                });
            }
        }
        Some(code) => {
            // NOTE: This probably isn't the best way to do this but for the most part any Rust
            // panics should set the exit code to 101
            if code == 101 {
                return Err(TestOutcome::Panic(common_out));
            } else {
                Ok(code)
            }
        }
    }
}
