use std::sync::atomic::{AtomicI32, Ordering};

static MY_ERR: AtomicI32 = AtomicI32::new(0);

pub fn my_pow(base: f64, exponent: f64) -> f64 {
    let result = base.powf(exponent);
    if result.is_nan() {
        MY_ERR.store(-1, Ordering::SeqCst);
        0.0
    } else if result.is_infinite() {
        MY_ERR.store(-1, Ordering::SeqCst);
        0.0
    } else {
        result
    }
}

pub fn feel_the_power(base: f64, exponent: f64) -> f64 {
    MY_ERR.store(0, Ordering::SeqCst);
    let result = my_pow(base, exponent);

    if MY_ERR.load(Ordering::SeqCst) != 0 {
        crate::ffi::output_error();
    } else {
        crate::ffi::output_result(result);
    }

    result
}
