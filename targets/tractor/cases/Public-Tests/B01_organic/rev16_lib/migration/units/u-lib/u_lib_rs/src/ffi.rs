#[no_mangle]
pub unsafe extern "C" fn rev16(a: u32) -> u32 {
    crate::logic::rev16(a)
}
