#[no_mangle]
pub unsafe extern "C" fn gaussian_kernel(dest: *mut f32, size: i32, radius: f32) {
    let size_usize = if size < 0 { 0 } else { size as usize };
    let dest_slice = std::slice::from_raw_parts_mut(dest, size_usize);
    crate::logic::gaussian_kernel(dest_slice, size, radius)
}
