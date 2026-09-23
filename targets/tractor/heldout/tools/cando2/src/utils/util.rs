// © 2026 Massachusetts Institute of Technology
// MIT License

//! General utility functions for writing cando harness

use std::ffi::CString;

use libc::c_char;

/// Converts an arbitrary `Vec` to a const raw pointer
///
/// The only difference between this function and the normal `as_ptr()` implementation is when
/// `vec` is empty it returns `NULL` rather than `0x1` which is necessary for passing it over FFI
/// to C.
pub fn vec_as_ptr<T>(vec: &Vec<T>) -> *const T {
    if vec.is_empty() {
        std::ptr::null()
    } else {
        vec.as_ptr()
    }
}

/// Converts an arbitrary `Vec` to a mutable raw pointer in the same way as above
pub fn vec_as_mut_ptr<T>(vec: &mut Vec<T>) -> *mut T {
    if vec.is_empty() {
        std::ptr::null_mut()
    } else {
        vec.as_mut_ptr()
    }
}

/// Converts `Option<CString>` to a const raw pointer
///
/// If `str.is_none()` then this will return `NULL` otherwise converts the interior value to a raw
/// pointer. We don't need to handle empty strings similar as to how we would `Vec` because it will
/// always be a valid pointer (because of the null-terminator byte at the end)
pub fn cstr_as_ptr(str: &Option<CString>) -> *const c_char {
    match str {
        Some(s) => s.as_ptr(),
        None => std::ptr::null(),
    }
}

/// Converts `Option<CString>` to a mutable raw pointer in the same way as above
pub fn cstr_as_mut_ptr(str: Option<CString>) -> *mut c_char {
    match str {
        Some(s) => s.into_raw(),
        None => std::ptr::null_mut(),
    }
}

/// Converts a raw pointer to an `Option<CString>` in the opposite fashion as above
pub fn ptr_to_cstr(ptr: *mut c_char) -> Option<CString> {
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { CString::from_raw(ptr) })
    }
}

/// Defines a Rust function that calls a C function in the given `library`
/// with some error handling
///
/// # Args
///
/// * `name`: the name to give the function
/// * `library`: the library to get the `symbol` from
/// * `symbol`: the binary name of the symbol
/// * `signature` the signature of `symbol`
///
/// # Returns
///     
/// A callable function. If `symbol` and/or `signature` don't exist then this forcefully
/// exits with the `SymbolNotFound` exit code.
#[macro_export]
macro_rules! lib_fn {
    (
        name: $name:ident,
        library: $lib:expr,
        symbol: $sym:expr,
        signature: $sig:ty,
    ) => {
        unsafe fn $name() -> $sig {
            unsafe {
                let sym = $lib.get::<$sig>($sym).unwrap_or_else(|e| {
                    let sym_name = String::from_utf8_lossy($sym).to_string();
                    CandoError::SymbolNotFound(sym_name).exit()
                });
                *sym
            }
        }
    };
}
