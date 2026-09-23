#[no_mangle]
pub unsafe extern "C" fn premultiply(img: *mut crate::logic::CpImageT) {
    if !img.is_null() {
        let img_ref = &*img;
        let w = img_ref.w;
        let h = img_ref.h;

        if w > 0 && h > 0 {
            let num_pixels = (w as usize).wrapping_mul(h as usize);
            let pixels = std::slice::from_raw_parts_mut(img_ref.pix, num_pixels);
            crate::logic::premultiply_inner(pixels);
        }
    }
}
