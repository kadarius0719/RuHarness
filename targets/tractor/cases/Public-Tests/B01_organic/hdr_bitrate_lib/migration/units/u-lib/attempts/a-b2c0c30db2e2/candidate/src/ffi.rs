#[no_mangle]
pub unsafe extern "C" fn hdr_bitrate(h: *const u8) -> u32 {
    let h_slice = std::slice::from_raw_parts(h, 3);
    crate::logic::hdr_bitrate(h_slice)
}
