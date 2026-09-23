use crate::logic;

#[no_mangle]
pub unsafe extern "C" fn hex2bin(
    bin: *mut u8,
    bin_maxlen: usize,
    hex: *const u8,
    hex_len: usize,
    ignore: *const u8,
    hex_end_p: *mut *const u8,
) -> i32 {
    let bin_slice = std::slice::from_raw_parts_mut(bin, bin_maxlen);
    let hex_slice = std::slice::from_raw_parts(hex, hex_len);

    let ignore_slice = if ignore.is_null() {
        None
    } else {
        let mut ignore_len = 0;
        while *ignore.add(ignore_len) != 0 {
            ignore_len += 1;
        }
        Some(std::slice::from_raw_parts(ignore, ignore_len))
    };

    let (mut ret, mut bin_pos, hex_pos) = logic::hex2bin(bin_slice, hex_slice, ignore_slice);

    if ret != 0 {
        bin_pos = 0;
    }

    if !hex_end_p.is_null() {
        *hex_end_p = hex.add(hex_pos);
    } else if hex_pos != hex_len {
        ret = -1;
    }

    if ret != 0 {
        ret
    } else {
        bin_pos as i32
    }
}
