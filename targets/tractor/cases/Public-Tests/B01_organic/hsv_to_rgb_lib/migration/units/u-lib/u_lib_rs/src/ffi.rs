use crate::logic;

#[no_mangle]
pub unsafe extern "C" fn hsv_to_rgb(dest: *mut f32, src: *const f32) {
    let dest_slice = std::slice::from_raw_parts_mut(dest, 3);
    let src_slice = std::slice::from_raw_parts(src, 3);
    logic::hsv_to_rgb(dest_slice, src_slice);
}
