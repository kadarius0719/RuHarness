use crate::logic;

#[no_mangle]
pub unsafe extern "C" fn hdr_compare(h1: *const u8, h2: *const u8) -> i32 {
    let h1_slice = std::slice::from_raw_parts(h1, 3);
    let h2_slice = std::slice::from_raw_parts(h2, 3);
    logic::hdr_compare(h1_slice, h2_slice)
}
