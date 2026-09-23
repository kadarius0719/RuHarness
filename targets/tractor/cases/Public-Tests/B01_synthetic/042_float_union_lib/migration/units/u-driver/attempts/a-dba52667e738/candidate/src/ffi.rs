extern "C" {
    fn putchar(c: i32) -> i32;
}

pub fn put_char(c: u8) {
    unsafe {
        putchar(c as i32);
    }
}

#[no_mangle]
pub unsafe extern "C" fn driver(f: f64) {
    crate::logic::driver(f);
}
