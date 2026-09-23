#[no_mangle]
pub unsafe extern "C" fn half2float(h: u16) -> f32 {
    crate::logic::half2float(h)
}
