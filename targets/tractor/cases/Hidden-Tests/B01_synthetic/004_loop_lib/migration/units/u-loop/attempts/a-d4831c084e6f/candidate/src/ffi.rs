use crate::logic;

extern "C" {
    fn putchar(c: i32) -> i32;
}

pub fn print_int(n: i32) {
    let s = format!("{}\n", n);
    for &byte in s.as_bytes() {
        unsafe { putchar(byte as i32); }
    }
}

#[no_mangle]
pub unsafe extern "C" fn r#loop(max_val: i32) {
    logic::loop_fn(max_val);
}
