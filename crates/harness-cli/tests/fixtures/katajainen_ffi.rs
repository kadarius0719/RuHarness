//! C ABI shim for unit u001 (fixture for the executor e2e test).

use core::ffi::{c_int, c_uint};

/// `int ZopfliLengthLimitedCodeLengths(const size_t*, int, int, unsigned*)`
///
/// # Safety
/// `frequencies` and `bitlengths` must each point to `n` valid elements.
#[no_mangle]
pub unsafe extern "C" fn ZopfliLengthLimitedCodeLengths(
    frequencies: *const usize,
    n: c_int,
    maxbits: c_int,
    bitlengths: *mut c_uint,
) -> c_int {
    let len = usize::try_from(n).unwrap_or(0);
    let (freqs, bits): (&[usize], &mut [u32]) = if len == 0 {
        (&[], &mut [])
    } else {
        (
            core::slice::from_raw_parts(frequencies, len),
            core::slice::from_raw_parts_mut(bitlengths, len),
        )
    };
    crate::logic::length_limited_code_lengths(freqs, maxbits, bits)
}
