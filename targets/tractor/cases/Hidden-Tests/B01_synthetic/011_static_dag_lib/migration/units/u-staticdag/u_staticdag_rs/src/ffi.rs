use crate::logic;

extern "C" {
    fn putchar(c: i32) -> i32;
}

pub fn output_int(n: i32) {
    let s = format!("{}\n", n);
    for &b in s.as_bytes() {
        unsafe { putchar(b as i32); }
    }
}

#[no_mangle]
pub unsafe extern "C" fn static_update(update: bool, new_value: i32) -> i32 {
    logic::static_update(update, new_value)
}

#[no_mangle]
pub unsafe extern "C" fn path_mult(update: i32) {
    logic::path_mult(update);
}

#[no_mangle]
pub unsafe extern "C" fn path_add(update: i32) {
    logic::path_add(update);
}

#[no_mangle]
pub unsafe extern "C" fn path_subtract(update: i32) {
    logic::path_subtract(update);
}

#[no_mangle]
pub unsafe extern "C" fn driver(val: i32, iterations: i32) {
    logic::driver(val, iterations);
}
