// © 2026 Massachusetts Institute of Technology
// MIT License

use {
    crate::CatastrophicError,
    std::{env, ffi::OsStr, path::PathBuf},
};

/// Directory structure information for where the `harness!` macro is used
#[derive(Debug, Clone)]
pub struct HarnessContext {
    /// The root directory that holds all test case information (e.g., `001_helloworld` or
    /// `001_helloworld_lib`)
    pub test_root_dir: PathBuf,
    /// Directory that holds the C test case. Named `test_case` relative to `test_root_dir`
    pub test_case_dir: PathBuf,
    /// Directory that holds test vectors named `test_vectors` relative to `test_root_dir`
    pub test_vector_dir: PathBuf,
}

impl HarnessContext {
    /// Looks for the appropriate paths for a test case relative to the given `test_root_dir`
    /// if specified otherwise in the current directory
    pub fn discover(test_root_dir: Option<PathBuf>) -> Result<Self, CatastrophicError> {
        let test_root_dir = Self::test_root_dir(test_root_dir)?;
        let test_case_dir = test_root_dir.join("test_case");
        let test_vector_dir = test_root_dir.join("test_vectors");

        Ok(Self {
            test_root_dir,
            test_case_dir,
            test_vector_dir,
        })
    }

    fn test_root_dir(test_root_dir: Option<PathBuf>) -> Result<PathBuf, CatastrophicError> {
        let test_root_dir = match test_root_dir {
            Some(p) => return Ok(p),
            None => env::current_dir()?,
        };

        // When running integration tests we need to be able to find the path to the
        // example test cases. So we'll use the precesence of the `CANDO_TESTS` environment
        // variable that's set by the tests to do that
        if env::var("CANDO_TESTS").is_ok() {
            Ok(test_root_dir
                .join("tests")
                .join("test_data")
                .join("mock_candidate_lib"))
        } else if cfg!(fuzzing) || test_root_dir.file_name() == Some(OsStr::new("runner")) {
            // If we're fuzzing or in `runner` then we're running a library test and need to
            // get the parent directory to be in the test_case_root
            Ok(test_root_dir
                .parent()
                .ok_or_else(|| {
                    CatastrophicError::str_to_err("Couldn't get parent directory of `manifest_dir`")
                })?
                .to_path_buf())
        } else {
            // Any other case the user has specified a test_case_root so just use that
            Ok(test_root_dir)
        }
    }

    /// Returns the build directory.
    ///
    /// # Args
    ///
    /// `run_rust`: if we're looking for the Rust build directory or C. `run_rust == true` then this
    /// will return `<test_root_dir>/translated_rust/target/release`, otherwise
    /// `<test_root_dir>/build-ninja`
    pub fn get_build_dir(&self, run_rust: bool) -> PathBuf {
        if run_rust {
            self.test_root_dir
                .join("translated_rust")
                .join("target")
                .join("release")
        } else {
            self.test_root_dir.join("build-ninja")
        }
    }
}
