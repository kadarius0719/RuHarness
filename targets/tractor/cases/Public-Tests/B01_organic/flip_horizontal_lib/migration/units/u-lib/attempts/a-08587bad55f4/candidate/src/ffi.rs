use crate::logic::{CpImageT, CpPixelT, flip_horizontal_impl};

#[no_mangle]
pub unsafe extern "C" fn flip_horizontal(img: *mut CpImageT) {
    if !img.is_null() {
        let img_ref = &mut *img;
        let w = img_ref.w as usize;
        let h = img_ref.h as usize;
        let pixel_count = w * h;
        let pixels = std::slice::from_raw_parts_mut(img_ref.pix, pixel_count);
        flip_horizontal_impl(pixels, w, h);
    }
}
