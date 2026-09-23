#[repr(C)]
pub struct cb_rgb_255 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[no_mangle]
pub unsafe extern "C" fn contrast_ratio(a: cb_rgb_255, b: cb_rgb_255) -> f32 {
    crate::logic::contrast_ratio(a.r, a.g, a.b, b.r, b.g, b.b)
}
