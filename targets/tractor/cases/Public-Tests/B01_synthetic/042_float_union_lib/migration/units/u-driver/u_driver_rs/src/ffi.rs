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
pub unsafe extern "C" fn driver(f: f64) {
    logic::driver(f);
}
