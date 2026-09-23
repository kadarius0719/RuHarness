use crate::logic;

#[no_mangle]
pub unsafe extern "C" fn bin2hex(
    hex: *mut u8,
    hex_maxlen: usize,
    bin: *const u8,
    bin_len: usize,
) -> *mut u8 {
    if hex.is_null() || bin.is_null() {
        return hex;
    }

    let hex_slice = std::slice::from_raw_parts_mut(hex, hex_maxlen);
    let bin_slice = std::slice::from_raw_parts(bin, bin_len);

    logic::bin2hex(hex_slice, bin_slice);
    hex
}
