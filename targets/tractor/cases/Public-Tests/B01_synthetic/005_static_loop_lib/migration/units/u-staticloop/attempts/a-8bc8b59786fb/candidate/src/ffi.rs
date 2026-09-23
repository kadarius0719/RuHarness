extern "C" {
    fn putchar(c: i32) -> i32;
}

pub fn put_byte(b: u8) {
    unsafe { putchar(b as i32); }
}

#[no_mangle]
pub extern "C" fn static_sum(update: i32) -> i32 {
    crate::logic::static_sum(update)
}

#[no_mangle]
pub extern "C" fn driver(stride: i32) {
    crate::logic::driver(stride);
}
