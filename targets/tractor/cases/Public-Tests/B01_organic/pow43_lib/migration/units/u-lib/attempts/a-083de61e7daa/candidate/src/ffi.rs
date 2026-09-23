#[no_mangle]
pub unsafe extern "C" fn pow43(x: i32) -> f32 {
    crate::logic::pow43(x)
}
