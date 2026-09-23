extern "C" {
    fn putchar(c: i32) -> i32;
}

pub fn put_byte(b: u8) {
    unsafe {
        putchar(b as i32);
    }
}

#[no_mangle]
pub unsafe extern "C" fn perform_operations(a: u32, b: u32) -> u32 {
    crate::logic::perform_operations(a, b)
}

#[no_mangle]
pub unsafe extern "C" fn switch_arith(seed: u32) {
    crate::logic::switch_arith(seed);
}
