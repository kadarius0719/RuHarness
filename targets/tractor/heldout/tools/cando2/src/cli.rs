// © 2026 Massachusetts Institute of Technology
// MIT License

use {
    crate::{CatastrophicError, LogLevel},
    clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum, error::ErrorKind},
    std::path::PathBuf,
};

/// Tool for running test cases for translation candidates
#[derive(Debug, Parser)]
#[command(version, about)]
pub struct TopLevelOptions {
    #[command(subcommand)]
    pub subcommand: TopLevelSubcommand,

    /// Controls what gets printed. Options are `none`, `quiet`, `normal`, `verbose`
    #[arg(short, long, default_value = "normal")]
    pub log_level: LogLevel,

    /// The path to the root of the test case directory to test. Defaults to the current directory
    #[arg(short, long)]
    pub test_root_dir: Option<PathBuf>,

    /// Write JSON test vector to file (relative to `test_vectors` directory)
    #[arg(short, long, conflicts_with = "vectors")]
    pub write_json: Option<String>,

    /// If not given a qualified path (either / or ./) then this is relative to the `test_vectors`
    /// directory
    #[arg(short, long, conflicts_with = "write_json")]
    pub vectors: Vec<String>,

    /// Run Rust
    #[arg(short, long, default_value = "false")]
    pub rust: bool,

    /// Timeout (in seconds) to run an individual test vector for
    #[arg(long)]
    pub timeout: Option<u64>,

    /// File to output the JSON report for running the test vectors
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Select benchmarking option. The runtime option currently uses perf-stat which
    /// is required to be installed, set in the user's PATH and will
    /// usually require root permissions and/or setting kernel.perf_event_paranoid to run.
    /// Additionally, for memory, valgrind should be installed and set in PATH
    #[arg(short, long)]
    pub bench: Option<BenchMode>,
}

/// User specified benchmarking option
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum BenchMode {
    Memory,
    Runtime,
}

/// Subcommand to run a library vs. binary test
#[derive(Debug, Subcommand)]
pub enum TopLevelSubcommand {
    Lib(LibOptions),
    Bin(BinOptions),
}

/// Run tests for a library candidate. These options effect the values defined in `lib_state_in`
/// which are defined in the `state` of the `harness!` macro.
#[derive(Debug, Args)]
#[group(multiple = false)]
pub struct LibOptions {
    /// Execute the candidate using a zeroed state
    #[arg(short, long)]
    pub zero: bool,

    // FIXME: Not quite sure what this is
    /// Execute the candidate using a pattern of the specified length
    #[arg(short, long)]
    pub pattern: Option<usize>,

    /// Execute the candidate using random bytes of the specified length
    #[arg(short, long)]
    pub random: Option<usize>,

    /// Execute the candidate using a file produced by cargo-fuzz, path relative to runner dir
    #[arg(short, long)]
    pub fuzzed: Option<String>,
}

/// Run tests for a binary candidate
#[derive(Debug, Args)]
pub struct BinOptions {
    /// Name of binary. Relative to the appropriate build directory (`build-ninja` for C or
    /// `translated_rust/target/release` for Rust) unless absolute. Defaults to `driver`.
    #[arg(short, long, default_value = "driver")]
    pub name: String,
}

/// Parses CLI arguments into `TopLevelOptions`.
/// This does some post parsing to ensure that `--vectors` isn't used with
/// any of the other library options.
pub fn parse_args(raw_args: &[&str]) -> Result<TopLevelOptions, CatastrophicError> {
    match TopLevelOptions::try_parse_from(raw_args) {
        Ok(opts) => {
            if let TopLevelSubcommand::Lib(lib_opts) = &opts.subcommand {
                if lib_opts.zero
                    || lib_opts.pattern.is_some()
                    || lib_opts.random.is_some()
                    || lib_opts.fuzzed.is_some()
                {
                    // We need to maker sure the user didn't specify multiple different input options for
                    // library tests. This is only relevant for --vectors and any of the libary options.
                    // Within the library options clap already handle ensuring they're mututally exlusive
                    if !opts.vectors.is_empty() {
                        let err = TopLevelOptions::command().error(ErrorKind::ArgumentConflict, "The `--vector` option cannot be used with library options (`--zero`, `--pattern`, `--random`, or `--fuzzed`)");
                        return Err(CatastrophicError::Usage(Box::new(err)));
                    }

                    // It doesn't make sense to benchmarking the generation of a test vector so
                    // we'll enforce that here
                    if opts.bench.is_some() {
                        let err = TopLevelOptions::command().error(ErrorKind::ArgumentConflict, "The `--bench` option can't be used with any of the library pattern generators");
                        return Err(CatastrophicError::Usage(Box::new(err)));
                    }
                }
            }

            Ok(opts)
        }
        Err(e) => Err(CatastrophicError::Usage(Box::new(e))),
    }
}

#[cfg(test)]
mod tests {
    use crate::error::CandoError;

    use super::*;

    /// Helper to make sure error is a usage error and the rc will be 4
    fn assert_usage_err(res: Result<TopLevelOptions, CatastrophicError>) {
        assert!(matches!(res, Err(CatastrophicError::Usage(_))));
        assert!(res.is_err_and(|e| CandoError::CatastrophicError(e).exit_code() == 4));
    }

    #[test]
    fn test_none() {
        let args = [];
        assert_usage_err(parse_args(&args));
    }

    #[test]
    fn test_help() {
        let args = ["--", "help"];
        assert_usage_err(parse_args(&args));
    }

    #[test]
    fn test_log_level() {
        let args = ["--", "-l", "quiet", "lib"];
        let parsed = parse_args(&args).unwrap();
        assert_eq!(parsed.log_level, LogLevel::QUIET);

        let args = ["--", "-l", "normal", "bin"];
        let parsed = parse_args(&args).unwrap();
        assert_eq!(parsed.log_level, LogLevel::NORMAL);

        let args = ["--", "--log-level", "verbose", "lib"];
        let parsed = parse_args(&args).unwrap();
        assert_eq!(parsed.log_level, LogLevel::VERBOSE);
    }

    #[test]
    fn test_write_json() {
        let args = ["--", "bin"];
        let parsed = parse_args(&args).unwrap();
        assert_eq!(parsed.write_json, None);

        let args = ["--", "-w", "test.json", "lib"];
        let parsed = parse_args(&args).unwrap();
        assert_eq!(parsed.write_json, Some(String::from("test.json")));
    }

    #[test]
    fn test_vectors() {
        let args = ["--", "lib"];
        let parsed = parse_args(&args).unwrap();
        assert!(parsed.vectors.is_empty());

        let args = ["--", "-v"];
        assert_usage_err(parse_args(&args));

        let args = ["--", "-v", "test1.json", "lib"];
        let parsed = parse_args(&args).unwrap();
        assert_eq!(parsed.vectors, vec!["test1.json"]);

        let args = [
            "--",
            "--vectors",
            "test1.json",
            "--vectors",
            "test2.json",
            "bin",
        ];
        let parsed = parse_args(&args).unwrap();
        assert_eq!(parsed.vectors, vec!["test1.json", "test2.json"]);
    }

    #[test]
    fn test_rust() {
        let args = ["--", "lib"];
        let parsed = parse_args(&args).unwrap();
        assert!(!parsed.rust);

        let args = ["--", "--rust", "bin"];
        let parsed = parse_args(&args).unwrap();
        assert!(parsed.rust);
    }

    #[test]
    fn test_top_level_invalid() {
        // Not going to support writing a test vector that is already on disk because their could
        // be multiple and it will get messy
        let args = ["--", "-v", "test1.json", "-w", "test1.json", "bin"];
        assert_usage_err(parse_args(&args));

        // Can't be given a test vector and generate one
        let args = ["--", "-v", "test1.json", "lib", "-z"];
        assert_usage_err(parse_args(&args));

        let args = ["--", "-v", "test1.json", "lib", "-p", "100"];
        assert_usage_err(parse_args(&args));

        let args = ["--", "-v", "test1.json", "lib", "-r", "10"];
        assert_usage_err(parse_args(&args));

        let args = ["--", "-v", "test1.json", "lib", "-f", "fuzz_profile"];
        assert_usage_err(parse_args(&args));
    }

    #[test]
    fn test_lib_valid() {
        let args = ["--", "lib", "--zero"];
        let parsed = parse_args(&args).unwrap();
        match parsed.subcommand {
            TopLevelSubcommand::Lib(lib_opts) => assert!(lib_opts.zero),
            _ => panic!("Expected lib subcommand"),
        }

        let args = ["--", "lib", "--pattern", "10"];
        let parsed = parse_args(&args).unwrap();
        match parsed.subcommand {
            TopLevelSubcommand::Lib(lib_opts) => assert_eq!(lib_opts.pattern, Some(10)),
            _ => panic!("Expected lib subcommand"),
        }

        let args = ["--", "lib", "-r", "100"];
        let parsed = parse_args(&args).unwrap();
        match parsed.subcommand {
            TopLevelSubcommand::Lib(lib_opts) => assert_eq!(lib_opts.random, Some(100)),
            _ => panic!("Expected lib subcommand"),
        }

        let args = ["--", "lib", "-f", "fuzz_profile"];
        let parsed = parse_args(&args).unwrap();
        match parsed.subcommand {
            TopLevelSubcommand::Lib(lib_opts) => {
                assert_eq!(lib_opts.fuzzed, Some(String::from("fuzz_profile")))
            }
            _ => panic!("Expected lib subcommand"),
        }
    }

    /// Shouldn't be able to use multiple lib options
    #[test]
    fn test_lib_invalid() {
        let args = ["--", "lib", "-z", "-p", "100"];
        assert_usage_err(parse_args(&args));

        let args = ["--", "lib", "-r", "5", "-f", "fuzz_profile"];
        assert_usage_err(parse_args(&args));
    }

    #[test]
    fn test_bin() {
        let args = ["--", "bin"];
        let parsed = parse_args(&args).unwrap();
        match parsed.subcommand {
            TopLevelSubcommand::Bin(bin_opts) => assert_eq!(bin_opts.name, String::from("driver")),
            _ => panic!("Expect bin subcommand"),
        }

        let args = ["--", "bin", "-n", "cool_bin"];
        let parsed = parse_args(&args).unwrap();
        match parsed.subcommand {
            TopLevelSubcommand::Bin(bin_opts) => {
                assert_eq!(bin_opts.name, String::from("cool_bin"))
            }
            _ => panic!("Expected bin subcommand"),
        }
    }
}
