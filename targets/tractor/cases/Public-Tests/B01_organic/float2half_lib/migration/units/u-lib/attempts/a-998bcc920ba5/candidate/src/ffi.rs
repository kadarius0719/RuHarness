#[no_mangle]
pub unsafe extern "C" fn float2half(flt: f32) -> u16 {
    crate::logic::float2half(flt)
}
