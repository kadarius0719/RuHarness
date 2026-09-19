#[no_mangle]
pub unsafe extern "C" fn ZopfliLengthLimitedCodeLengths(
    frequencies: *const size_t,
    n: i32,
    maxbits: u32,
    bitlengths: *mut usize,
) -> i32 {
    // implementation
}
