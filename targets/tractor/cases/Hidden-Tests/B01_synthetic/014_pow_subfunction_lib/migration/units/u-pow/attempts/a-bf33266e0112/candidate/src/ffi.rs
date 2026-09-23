extern "C" {
    fn putchar(c: i32) -> i32;
}

pub fn write_string(s: &str) {
    for byte in s.bytes() {
        unsafe { putchar(byte as i32); }
    }
}

#[no_mangle]
pub unsafe extern "C" fn feel_the_power(base: f64, exponent: f64) -> f64 {
    crate::logic::feel_the_power(base, exponent)
}

#[no_mangle]
pub unsafe extern "C" fn my_pow(base: f64, exponent: f64) -> f64 {
    crate::logic::my_pow(base, exponent)
}
