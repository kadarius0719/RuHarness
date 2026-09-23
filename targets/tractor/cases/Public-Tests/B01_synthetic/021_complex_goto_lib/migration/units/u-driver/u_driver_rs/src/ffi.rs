extern "C" {
    fn printf(format: *const u8, ...) -> i32;
}

pub fn put_byte(b: u8) {
    unsafe {
        printf(b"%c\0".as_ptr(), b as i32);
    }
}

#[no_mangle]
pub unsafe extern "C" fn driver(x: i32, y: i32) {
    crate::logic::driver(x, y);
}
