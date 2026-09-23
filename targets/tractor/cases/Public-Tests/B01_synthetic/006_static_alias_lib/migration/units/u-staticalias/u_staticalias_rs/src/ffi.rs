use std::os::raw::c_int;

use crate::logic;

extern "C" {
    fn putchar(c: c_int) -> c_int;
}

/// The one sanctioned exception to the ban on foreign `extern` blocks:
/// a safe wrapper around C's `putchar`, used so this unit's stdout
/// output lands in the exact same C stdio stream as the differential
/// driver's own output.
pub fn put_byte(b: u8) {
    unsafe {
        putchar(b as c_int);
    }
}

#[no_mangle]
pub unsafe extern "C" fn static_alias(outer: *mut i32) -> *mut i32 {
    match logic::static_alias_step(*outer) {
        Some(new_val) => {
            *outer = new_val;
            outer
        }
        None => logic::inner_ptr(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn driver(initial_value: i32, iterations: i32) {
    logic::driver(initial_value, iterations);
}
