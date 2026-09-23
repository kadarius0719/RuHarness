// © 2026 Massachusetts Institute of Technology
// MIT License

use {
    crate::CatastrophicError,
    arbitrary::{Arbitrary, Unstructured},
    serde::{Serialize, de::DeserializeOwned},
    std::fmt::Debug,
};

/// Wrapper trait to hide Arbitrary's lifetime complexity
pub trait ArbitraryState: Sized {
    fn arbitrary_from_bytes(bytes: &[u8]) -> Result<Self, CatastrophicError>;
}

/// Implementation for any type that implements Arbitrary
impl<T> ArbitraryState for T
where
    T: for<'a> Arbitrary<'a>,
{
    fn arbitrary_from_bytes(bytes: &[u8]) -> Result<Self, CatastrophicError> {
        Ok(<Self as Arbitrary>::arbitrary(&mut Unstructured::new(
            bytes,
        ))?)
    }
}

/// Trait used by `State` in `lib.rs` to handle serialization
/// Used so `State` can be used in accross this project
pub trait RunnableState<E>: ArbitraryState + Sized + DeserializeOwned + Serialize + Debug {
    /// Creates instance of `Self` with "zeroed" values.
    ///
    /// "zeroed" values mean various things for different types. For example
    /// `i32` -> 0, `*const` or `*mut` = `std::ptr::null()`, etc...
    fn zeroed() -> Result<Self, CatastrophicError> {
        Ok(Self::from_bytes(&[])?)
    }

    /// Uses `Arbitrary` crate to create and instance of `Self` from arbitrary `bytes`.
    ///
    /// You don't need to be careful about what `bytes` you pass into this
    /// because it's not a transmute.
    fn from_bytes(bytes: &[u8]) -> Result<Self, CatastrophicError> {
        Ok(Self::arbitrary_from_bytes(bytes)?)
    }

    /// Create instance of `Self` from JSON
    fn from_json(s: &str) -> Result<Self, CatastrophicError> {
        Ok(serde_json::from_str(s)?)
    }

    /// Converts 'Self` to JSON
    fn to_json(&self) -> Result<String, CatastrophicError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Run function that is defined by the user in the `harness!` macro.
    ///
    /// This can't be defined here as it relies on the concrete definition
    /// of `State` in the macro.
    fn run(&mut self);

    /// Converts `State` to `ExpectedState` by wrapping in `Some`
    fn to_expected_state(&self) -> E;
}

/// Default implementation for a binary test vector.
/// NOTE: `run` and `to_expected_state` will never make sense for a binary test, but `zeroed` or
/// `from_bytes` could possibly be implemented for binary tests (either for `argv` or `stdin`). For
/// right now this is just here to get rid of compiler errors.
impl RunnableState<()> for () {
    fn zeroed() -> Result<Self, CatastrophicError> {
        Err(CatastrophicError::str_to_err(
            "`zeroed` is not yet implemented for binary tests",
        ))
    }

    fn from_bytes(_bytes: &[u8]) -> Result<Self, CatastrophicError> {
        Err(CatastrophicError::str_to_err(
            "`from_bytes` is not yet implemented for binary tests",
        ))
    }

    fn run(&mut self) {
        unimplemented!()
    }

    fn to_expected_state(&self) -> () {
        unimplemented!()
    }
}
