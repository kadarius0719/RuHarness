use crate::logic;

extern "C" {
    fn putchar(c: i32) -> i32;
}

pub fn put_byte(b: u8) {
    unsafe { putchar(b as i32); }
}

pub fn output_error() {
    let s = "Oh no, there was an error! How rude.\n";
    for &b in s.as_bytes() {
        put_byte(b);
    }
}

pub fn output_result(result: f64) {
    let s = format!("Result: {:.2}\n", result);
    for &b in s.as_bytes() {
        put_byte(b);
    }
}

#[no_mangle]
pub unsafe extern "C" fn my_pow(base: f64, exponent: f64) -> f64 {
    logic::my_pow(base, exponent)
}

#[no_mangle]
pub unsafe extern "C" fn feel_the_power(base: f64, exponent: f64) -> f64 {
    logic::feel_the_power(base, exponent)
}
