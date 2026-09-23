#[no_mangle]
pub unsafe extern "C" fn tfm(dest: *mut f32, src: *const f32, count: i32) {
    if !dest.is_null() && !src.is_null() && count > 0 {
        let dest_len = (count as usize).wrapping_mul(2);
        let src_len = (count as usize).wrapping_mul(3);
        let dest_slice = std::slice::from_raw_parts_mut(dest, dest_len);
        let src_slice = std::slice::from_raw_parts(src, src_len);
        crate::logic::tfm_inner(count, dest_slice, src_slice);
    }
}
