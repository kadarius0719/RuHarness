extern "C" {
    fn putchar(c: i32) -> i32;
}

pub fn put_byte(b: u8) {
    unsafe { putchar(b as i32); }
}

#[no_mangle]
pub extern "C" fn sieve(val: i32) {
    crate::logic::sieve(val);
}
