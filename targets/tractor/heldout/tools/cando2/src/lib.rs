// © 2026 Massachusetts Institute of Technology
// MIT License

//! The `candidate` macro is the main feature of this library.
//!
//! It allows you to write a test harness for a candidate like so:
//!
//! ```ignore
//! harness! {
//!     state: {
//!         foo: bool,
//!         bar: Vec<c_char>,
//!         returns: i32
//!     },
//!
//!     signature: extern "C" fn(bool, *mut c_char) -> i32,
//!
//!     fn run(&mut self) {
//!         self.returns = unsafe {
//!             (*SYMBOL)(
//!                 self.foo,
//!                 self.bar.as_mut_ptr()
//!             )
//!         };
//!     }
//! }
//! ```
//!
//! The `state` argument defines the state that is reachable from
//! the input parameters to the candidate, as well as the return value.
//!
//! The `signature` defines how the dynamically-linked symbol will be called.
//!
//! The `run` function does the work of calling the symbol,
//! and the `self` parameter is an instance of the state defined previously.
//!
//! The macro generates a `main` function that automatically provides the
//! program with a command-line interface for running the harness.
//! See the documentation for `invoke_cli` for information on how to use it.

pub mod approx_eq;
pub mod bench;
pub mod cli;
pub mod common;
pub mod error;
pub mod log;
pub mod runners;
pub mod state;
pub mod test_vector;
pub mod utils;

// The `candidate` macro generates code that uses the following items internally.
// In order to make using this crate easy, this library re-exports all of them
// so that users can do `use cando::*;` to easily get all of them.
pub use {
    approx::Relative,
    arbitrary::{self, Arbitrary, Unstructured},
    argh,
    clap::Parser,
    common::HarnessContext,
    error::{CandoError, CatastrophicError},
    getrandom,
    libloading::{Library, Symbol},
    log::LogLevel,
    procspawn,
    runners::{lib_runner, runner},
    serde::{self, Deserialize, Serialize},
    serde_json::{from_str, to_string_pretty},
    state::RunnableState,
    std::{
        env,
        ffi::*,
        fs::File,
        io::Write,
        os::fd::FromRawFd,
        path::{Path, PathBuf},
        sync::{LazyLock, OnceLock},
    },
    test_vector::TestVector,
};

// These are defined like so because we can't change SYMBOL or LIBRARY to OnceLocks
// to allow parameters to be passed to their initializtion to maintain backwards
// compatibility with the existing library tests
pub static RUN_RUST: OnceLock<bool> = OnceLock::new();
pub static TEST_ROOT_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Handy macro to define structs that will be uses in `state` within the `harness!` macro.
/// Automatically derives necessary traits and declares it a C struct representation
#[macro_export]
macro_rules! state_member {
    ($struc:item) => {
        #[repr(C)]
        #[derive(Debug, Clone, Arbitrary, Serialize, Deserialize, PartialEq)]
        #[serde(crate = "self::serde")]
        $struc
    };
}

/// The harness macro has two forms.
///
/// If you're using the standardized naming where every candidate's shared library
/// filename and symbol name are identical to the candidate itself,
/// then you can use the short form,
/// without the explicit `library` and `symbol` arguments:
///
/// ```ignore
/// harness! {
///     state: { foo: i32 },
///     signature: extern "C" fn(),
///     fn run(&mut self) {}
/// }
/// ```
///
/// If you're not using standardized naming,
/// then specify the shared library filename and symbol name as follows:
///
/// ```ignore
/// harness! {
///     state: { foo: i32 },
///     library: "hello",
///     symbol: "my_hello",
///     signature: extern "C" fn(),
///     fn run(&mut self) {}
/// }
/// ```
///
/// NOTE: for the `library` argument,
/// if attempting to link "libfoo.so",
/// the argument should be "foo",
/// including neither the "lib" prefix nor the file extension.
#[macro_export]
macro_rules! harness {
    {
        state: {
            $(
                $field: ident: $typ: ty
            ),* $(,)?
        },

        signature: $sig:ty,

        $run_fn:item
    } => {
        harness! {
            state: { $($field : $typ),* },
            library: &*CANDIDATE_NAME,
            symbol: &*CANDIDATE_SYMBOL_NAME,
            signature: $sig,
            $run_fn
        }

    };

    {
        state: {
            $(
                $field: ident: $typ: ty
            ),* $(,)?
        },

        library: $lib:expr,

        symbol: $sym:expr,

        signature: $sig:ty,

        $run_fn:item
    } => {
        #[cfg(not(fuzzing))]
        fn main() {
            let args: Vec<String> = std::env::args().collect();
            let slice: Vec<&str> = args.iter().map(|e| &**e).collect();

            match conduct_with(&slice) {
                Ok(res) => {
                    if res.any_bench_failed {
                        CandoError::AnyBenchFailed.exit()
                    }
                    if res.any_vector_failed {
                        CandoError::AnyVectorFailed.exit()
                    }
                },
                Err(e) => e.exit()
            }
        }

        #[cfg(fuzzing)]
        libfuzzer_sys::fuzz_target!(|input: State| {
            let mut state = input;
            state.run();
        });

        /// Main entrypoint for running a library test
        ///
        /// We return all of the results to make testing easier
        fn conduct_with(raw_args: &[&str]) -> Result<runner::RunAllTestResult<State, ExpectedState>, CandoError> {
            // Initialize procspawn. Do this here so it's initialized for tests
            // NOTE: This re-runs all? code that is run before calling `init()` so things
            // putting it here means that only things created from static variables will
            // be run whenever we create a subprocess using `procspawn::spawn()`. Because
            // of this it's important to not print anything before this is called (i.e.,
            // during the creation of static variables) as that will get captured during
            // the subprocess run, and mess up the test vector comparison.
            procspawn::init();

            let opts = cli::parse_args(raw_args)?;
            log::set_log_level(opts.log_level);

            match opts.subcommand {
                cli::TopLevelSubcommand::Lib(lopts) => {
                    // Force initialization of LIBRARY and SYMBOL here so they get initialized
                    // in the parent, and can raise the error easier.
                    TEST_ROOT_DIR.get_or_init(|| opts.test_root_dir);
                    RUN_RUST.get_or_init(|| opts.rust);
                    let _ = &*LIBRARY;
                    let _ = &*SYMBOL;

                    let lib_runner = lib_runner::LibRunner::new(lopts, (*CONTEXT).clone(), opts.rust);
                    let has_state = lib_runner.has_state();
                    let res = runner::run_all_tests::<State, ExpectedState>(
                        &CONTEXT.test_vector_dir,
                        opts.vectors,
                        lib_runner,
                        opts.write_json,
                        has_state,
                        opts.timeout,
                        opts.bench,
                    )?;
                    res.output_report(opts.output)?;
                    Ok(res)
                },
                cli::TopLevelSubcommand::Bin(_) => Err(CandoError::usage_err("The `harness!` macro can only be used with `lib` subcommand"))
            }
        }

        /// Common context used throughout
        static CONTEXT: LazyLock<HarnessContext> = LazyLock::new(|| {
            let test_root_dir = TEST_ROOT_DIR.get()
                .unwrap_or_else(|| CandoError::str_to_err("`TEST_ROOT_DIR` OnceLock should be set before CONTEXT initialization").exit());
            HarnessContext::discover(test_root_dir.to_owned()).unwrap_or_else(|e| CandoError::from(e).exit())
        });

        /// Default name of the candidate/library. The name of the test_root_dir that the
        /// `harness!` macro is in
        static CANDIDATE_NAME: LazyLock<String> = LazyLock::new(|| {
            let path = CONTEXT.test_root_dir.clone();

            path.file_name()
                .unwrap_or_else(|| CandoError::str_to_err("Couldn't get filename from `test_root_dir`").exit())
                .to_os_string()
                .into_string()
                .unwrap_or_else(|_| CandoError::str_to_err("Couldn't convert OsStr for filename to String").exit())
        });

        /// Default name of the symbol/function to run. Defaults to `CANDIDATE_NAME` without the
        /// `_lib` suffix at the end
        static CANDIDATE_SYMBOL_NAME: LazyLock<String> = LazyLock::new(|| {
            CANDIDATE_NAME.strip_suffix("_lib")
                .unwrap_or_else(|| CandoError::str_to_err("Error stripping `_lib` sufix from candidate name").exit())
                .to_string()
        });

        /// The shared library from which to load the symbol for the candidate.
        static LIBRARY: LazyLock<Library> = LazyLock::new(|| unsafe {
            let run_rust = RUN_RUST.get()
                .unwrap_or_else(|| CandoError::str_to_err("`RUN_RUST` OnceLock should be set before LIBRARY initialization").exit());
            runners::lib_runner::get_library(&*CONTEXT, $lib, *run_rust).unwrap_or_else(|e| e.exit())
        });

        /// The symbol for the function that will be invoked as a test candidate.
        static SYMBOL: LazyLock<Symbol<$sig>> = LazyLock::new(|| unsafe {
            LIBRARY.get($sym.as_bytes()).unwrap_or_else(|e| CandoError::SymbolNotFound($sym.to_owned()).exit())
        });

        /// Created from fields listed in the `state` argument of this macro.
        /// It represents the memory that is directly reachable from an invocation
        /// of the candidate function meaning the input parameters (include any
        /// memory indirectly reachable from the input parameters, such as memory
        /// reachable via a pointer) and the function's return value (whose location
        /// in memory must be manually represented by the field listed in the `state`
        /// argument). This struct DOESN'T track file I/O or global variable.
        #[derive(Arbitrary, Clone, Debug, Deserialize, Serialize, PartialEq)]
        #[serde(crate = "self::serde", deny_unknown_fields)]
        pub struct State {
            $($field: $typ,)*
        }

        impl RunnableState<ExpectedState> for State {
            /// User defined `run` function.
            /// Invokes the test candidate with the appropriate parameters.
            /// The macro defines a global variable `SYMBOL` which is a
            /// function pointer to the dynamically-linked test candidate.
            $run_fn

            fn to_expected_state(&self) -> ExpectedState {
                ExpectedState {
                    $(
                        $field: Some(self.$field.clone()),
                    )*
                }
            }
        }

        /// Used for deserializing the expected output of a test case to compare
        /// against `State`. It's fields are all optional, which allows test cases
        /// to omit fields they don't want to check.
        #[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
        #[serde(crate = "self::serde", deny_unknown_fields)]
        pub struct ExpectedState {
            $($field: Option<$typ>,)*
        }

        impl PartialEq<ExpectedState> for State {
            fn eq(&self, other: &ExpectedState) -> bool {
                // For every field that isn't `None`, compare against
                // the equivalent field in `State`.
                $(
                    if let Some(other_val) = &other.$field {
                        let type_name = std::any::type_name::<$typ>();

                        // Safety: Here we defer type checking to runtime, because getting this
                        // done at compile time doesn't seem possible. All of the trasmutes are
                        // safe because we've already checked their respective types before the
                        // transmute. We call approx_eq for all floating point types, and defer to
                        // the normal equality checking for all other types
                        let equal = if type_name == "f32" {
                            let a: f32 = unsafe { std::mem::transmute_copy(&self.$field) };
                            let b: f32 = unsafe { std::mem::transmute_copy(&*other_val) };
                            approx_eq::approx_eq(a, b)
                        } else if type_name == "f64" {
                            let a: f64 = unsafe { std::mem::transmute_copy(&self.$field) };
                            let b: f64 = unsafe { std::mem::transmute_copy(&*other_val) };
                            approx_eq::approx_eq(a, b)
                        } else if type_name == "c_float" {
                            let a: c_float = unsafe { std::mem::transmute_copy(&self.$field) };
                            let b: c_float = unsafe { std::mem::transmute_copy(&*other_val) };
                            approx_eq::approx_eq(a, b)
                        }  else if type_name == "c_double" {
                            let a: c_double = unsafe { std::mem::transmute_copy(&self.$field) };
                            let b: c_double = unsafe { std::mem::transmute_copy(&*other_val) };
                            approx_eq::approx_eq(a, b)
                        } else {
                            self.$field == *other_val
                        };

                        if !equal {
                            return false;
                        }
                    }
                )*
                return true;
            }
        }
    };
}
