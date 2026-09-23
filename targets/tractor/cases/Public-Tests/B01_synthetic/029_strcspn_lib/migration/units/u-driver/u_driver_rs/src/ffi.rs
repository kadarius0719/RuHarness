use std::ffi::CStr;

extern "C" {
    fn putchar(c: i32) -> i32;
}

pub fn put_byte(b: u8) {
    unsafe { putchar(b as i32); }
}

#[no_mangle]
pub unsafe extern "C" fn driver(s1: *const u8, s2: *const u8) {
    let s1_bytes = CStr::from_ptr(s1 as *const i8).to_bytes();
    let s2_bytes = CStr::from_ptr(s2 as *const i8).to_bytes();

    crate::logic::driver_impl(s1_bytes, s2_bytes);
}
