#[no_mangle]
pub unsafe extern "C" fn rgb_to_hsv(dest: *mut f32, src: *const f32) {
    let dest_slice = std::slice::from_raw_parts_mut(dest, 3);
    let src_slice = std::slice::from_raw_parts(src, 3);
    crate::logic::rgb_to_hsv(dest_slice, src_slice);
}
