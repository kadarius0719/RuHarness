// © 2026 Massachusetts Institute of Technology
// MIT License

use {
    crate::{CatastrophicError, log, log::LogLevel},
    serde::{self, Deserialize, Deserializer, Serialize, de::DeserializeOwned},
    serde_json::from_str,
    std::{collections::HashMap, fmt::Debug, fs, path::Path},
};

/// Need a custom deserializer for `has_ub` because some of the binary tests have numbered fields
/// while others have string fields. So we just deserialize it to a `bool` if it's present and
/// ignore any value
fn deserialize_has_ub<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(serde_json::Value::deserialize(deserializer).is_ok())
}

/// This type represents a single test vector defined in a JSON file.
/// `S` and `E` are types for the state as defined in the `harness!` macro inside `lib.rs`. These
/// are not relevant for binary tests and can be passed as `()`
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(crate = "self::serde", deny_unknown_fields)]
pub struct TestVector<S, E: Serialize> {
    /// Options used for BOTH library and binary tests
    /// Data to pipe to the candidate over stdin
    pub stdin: Option<String>,
    /// The contents of stdout
    pub stdout: Option<Output>,
    /// The contents of stderr
    pub stderr: Option<Output>,
    /// Environment variables to pass to the candidate
    pub env: Option<HashMap<String, String>>,
    /// Whether or not the test vector deliberately exhibits UB.
    #[serde(default, deserialize_with = "deserialize_has_ub")]
    pub has_ub: bool,
    /// Freeform string for test-specific comments, unused by test runner
    pub note: Option<String>,

    /// Options used ONLY for binary tests
    /// Arguments to `main`
    pub argv: Option<Vec<String>>,
    /// Return code
    pub rc: Option<i32>,

    /// Options used ONLY for library tests
    /// State before invoked the symbol
    pub lib_state_in: Option<S>,
    /// State after invoked the symbol
    pub lib_state_out: Option<E>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(crate = "self::serde", deny_unknown_fields)]
pub struct Output {
    /// Output pattern expected
    pub pattern: String,
    /// When true, indicates that the `pattern` field should be compiled as a regex instead
    /// of used for a direct string comparison
    pub is_regex: Option<bool>,
}

impl<S, E> TestVector<S, E>
where
    S: DeserializeOwned + Serialize + Clone + PartialEq<E>,
    E: DeserializeOwned + Serialize + Clone,
{
    /// Instantiates `TestVector` from `json`
    pub fn from_json(json: &str) -> Result<Self, CatastrophicError> {
        Ok(from_str(json)?)
    }

    /// Instantiates `TestVector` from an arbitrary path
    ///
    /// # Returns
    ///
    /// JSON deserialized contents of `path` or None if there was nothing in the file
    pub fn from_file<T: AsRef<Path> + Debug>(path: T) -> Result<Option<Self>, CatastrophicError> {
        let json = std::fs::read_to_string(&path)?;
        if json.is_empty() {
            log!(LogLevel::VERBOSE, "{:?} is empty. Skipping...", path);
            Ok(None)
        } else {
            log!(LogLevel::VERBOSE, "From {:?} got JSON: {:#?}", path, json);
            Ok(Some(Self::from_json(&json)?))
        }
    }

    /// Converts `TestVector` representation to JSON formatted `String`
    pub fn to_json(&self, pretty_print: bool) -> Result<String, CatastrophicError> {
        if pretty_print {
            Ok(serde_json::to_string_pretty(&self)?)
        } else {
            Ok(serde_json::to_string(&self)?)
        }
    }

    /// Instantiates `TestVector` from `filename` relative to `test_vector_dir`
    ///
    /// # Returns
    ///
    /// JSON deserialized contents of given `filename` or None if there was nothing in the file
    pub fn from_test_vector_dir(
        test_vector_dir: &Path,
        filename: &Path,
    ) -> Result<Option<Self>, CatastrophicError> {
        Ok(Self::from_file(test_vector_dir.join(filename))?)
    }

    /// Instantiates HashMap of `TestVector`s from all files within `test_vector_dir`
    pub fn load_all(test_vector_dir: &Path) -> Result<HashMap<String, Self>, CatastrophicError> {
        let mut tcs = HashMap::new();

        for entry in std::fs::read_dir(test_vector_dir)? {
            let path = entry?.path();

            if let Some(ext) = path.extension()
                && ext.eq_ignore_ascii_case("json")
            {
                let file_name = path
                    .file_name()
                    .ok_or_else(|| CatastrophicError::str_to_err("Couldn't get filename"))?
                    .to_str()
                    .ok_or_else(|| CatastrophicError::str_to_err("Couldn't convert OsStr to &str"))?
                    .to_string();
                if let Some(tc) = Self::from_file(&path)? {
                    tcs.insert(file_name, tc);
                }
            }
        }
        Ok(tcs)
    }

    /// Loads all the specified `vector_names`.
    /// If they're qualified (either / or ./) then use that, otherwise they're relative to
    /// `test_vector_dir`.
    /// If `vector_names` is empty then loads all test vectors in `test_vector_dir`
    ///
    /// # Returns
    ///
    /// Ok(vectors): a map where the key is the name of the vector, and the value is it's
    /// serialized representation
    pub fn load_vectors(
        test_vector_dir: &Path,
        vector_names: Vec<String>,
    ) -> Result<HashMap<String, Self>, CatastrophicError> {
        if vector_names.is_empty() {
            Self::load_all(test_vector_dir)
        } else {
            let mut vectors = HashMap::new();
            for name in vector_names {
                let path = Path::new(&name);

                let vector = if path.is_absolute() {
                    Self::from_file(path)
                } else if path.starts_with("./") {
                    Self::from_file(path)
                } else {
                    Self::from_test_vector_dir(test_vector_dir, path)
                }?;

                if let Some(v) = vector {
                    vectors.insert(name, v);
                }
            }
            Ok(vectors)
        }
    }

    /// Write the test vector (`self`) to given `filename` relative to `test_vector_dir`
    pub fn write_to_vector_dir(
        &self,
        test_vector_dir: &Path,
        filename: &str,
    ) -> Result<(), CatastrophicError> {
        let full_path = test_vector_dir.join(filename);
        let json_str = self.to_json(true)?;
        Ok(fs::write(full_path, &json_str)?)
    }

    /// Loads a copy of the lib state.
    ///
    /// Once loaded, it can be run run via the `run` method on `State`.
    /// The copy here ensures that running the test does not
    /// unintentionally mutate this `TestVector`.
    pub fn lib_state(&self) -> Result<S, CatastrophicError> {
        Ok(self
            .lib_state_in
            .clone()
            .ok_or_else(|| CatastrophicError::str_to_err("Couldn't get copy of `lib_state`"))?)
    }

    /// Uses `PartialEq` implementation for `State` and `ExpectedState` in the `harness!` macro to
    /// determine equality of the expected and got output states
    pub fn equals_expected(&self, state: &S) -> Result<bool, CatastrophicError> {
        Ok(*state
            == *self
                .lib_state_out
                .as_ref()
                .ok_or_else(|| CatastrophicError::str_to_err("Couldn't get `lib_state_out`"))?)
    }
}
