#[no_mangle]
pub unsafe extern "C" fn gaussian_kernel(dest: *mut f32, size: i32, radius: f32) {
    // Mirror the C function's own access pattern exactly: it writes
    // `2*hsize + 1` consecutive elements starting at `dest` (where
    // hsize = size / 2, C-style truncating division), which is not
    // always equal to `size` (e.g. it is one larger for even `size`,
    // and can be nonzero even when `size` is 0 or negative). When that
    // count is zero the C code never touches `dest` at all, so we avoid
    // forming any slice from the raw pointer in that case.
    let hsize: i32 = size / 2;
    let n: usize = if hsize >= 0 { (2 * hsize + 1) as usize } else { 0 };

    if n == 0 {
        crate::logic::gaussian_kernel(&mut [], size, radius);
    } else {
        let slice = core::slice::from_raw_parts_mut(dest, n);
        crate::logic::gaussian_kernel(slice, size, radius);
    }
}
