#[no_mangle]
pub unsafe extern "C" fn normalize(dest: *mut f32, src: *const f32, size: i32) {
    let size_usize = if size < 0 { 0 } else { size as usize };
    let dest_slice = std::slice::from_raw_parts_mut(dest, size_usize);
    let src_slice = std::slice::from_raw_parts(src, size_usize);
    crate::logic::normalize(dest_slice, src_slice);
}
