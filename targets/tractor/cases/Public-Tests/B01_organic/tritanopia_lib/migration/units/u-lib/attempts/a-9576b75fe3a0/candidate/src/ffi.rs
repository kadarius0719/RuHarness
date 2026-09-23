#[no_mangle]
pub unsafe extern "C" fn tritanopia(rgb: crate::logic::cb_rgb_255) -> crate::logic::cb_rgb_255 {
    crate::logic::tritanopia(rgb)
}
