// C ABI surface of the katajainen unit: pointer-to-slice conversion and one
// call into crate::logic. All functionality lives in the logic module.

// int ZopfliLengthLimitedCodeLengths(const size_t*, int, int, unsigned*)
#[no_mangle]
pub unsafe extern "C" fn ZopfliLengthLimitedCodeLengths(
    frequencies: *const usize,
    n: i32,
    maxbits: i32,
    bitlengths: *mut u32,
) -> i32 {
    let len: usize = if n > 0 { n as usize } else { 0 };
    let freqs: &[usize] = if len == 0 || frequencies.is_null() {
        &[]
    } else {
        unsafe { core::slice::from_raw_parts(frequencies, len) }
    };
    let lengths: &mut [u32] = if len == 0 || bitlengths.is_null() {
        &mut []
    } else {
        unsafe { core::slice::from_raw_parts_mut(bitlengths, len) }
    };
    crate::logic::length_limited_code_lengths(freqs, maxbits, lengths)
}
