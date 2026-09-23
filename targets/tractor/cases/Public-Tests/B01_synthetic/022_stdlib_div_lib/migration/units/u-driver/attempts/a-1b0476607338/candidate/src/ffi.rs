extern "C" {
    fn printf(fmt: *const u8, ...) -> i32;
}

pub fn printf_division(quot: i32, rem: i32) {
    unsafe {
        let fmt = b"quotient: %d, remainder: %d\n\0".as_ptr();
        printf(fmt, quot, rem);
    }
}

#[no_mangle]
pub unsafe extern "C" fn driver(x: i32, y: i32) {
    crate::logic::driver(x, y);
}
