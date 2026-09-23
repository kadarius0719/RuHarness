use crate::logic;

#[no_mangle]
pub unsafe extern "C" fn stereo_samples(
    ibuf0: *mut i16,
    ibuf1: *mut i16,
    len: i32,
) -> i32 {
    let len_abs = if len < 0 { 0 } else { len as usize };
    let buf0_len = if len_abs > 0 { len_abs * 2 } else { 0 };
    let buf1_len = if len_abs > 0 { len_abs * 2 } else { 0 };

    let ibuf0_slice = if buf0_len > 0 {
        std::slice::from_raw_parts(ibuf0, buf0_len)
    } else {
        &[]
    };
    let ibuf1_slice = if buf1_len > 0 {
        std::slice::from_raw_parts(ibuf1, buf1_len)
    } else {
        &[]
    };

    logic::stereo_samples(ibuf0_slice, ibuf1_slice, len)
}
