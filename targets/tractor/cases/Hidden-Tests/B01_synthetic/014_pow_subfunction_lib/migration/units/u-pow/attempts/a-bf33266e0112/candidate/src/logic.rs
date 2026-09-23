use std::cell::RefCell;

thread_local! {
    static MY_ERR: RefCell<i32> = RefCell::new(0);
}

fn format_f64(val: f64) -> String {
    if val.is_nan() {
        "nan".to_string()
    } else if val.is_infinite() {
        if val.is_sign_positive() {
            "inf".to_string()
        } else {
            "-inf".to_string()
        }
    } else {
        format!("{:.2}", val)
    }
}

pub fn my_pow(base: f64, exponent: f64) -> f64 {
    let result = base.powf(exponent);
    if result.is_nan() {
        eprint!("Domain error: pow({}, {}) is undefined in the real number domain.\n", format_f64(base), format_f64(exponent));
        MY_ERR.with(|err| *err.borrow_mut() = -1);
        0.0
    } else if result.is_infinite() {
        eprint!("Range error: pow({}, {}) caused overflow or underflow.\n", format_f64(base), format_f64(exponent));
        MY_ERR.with(|err| *err.borrow_mut() = -1);
        0.0
    } else {
        result
    }
}

pub fn feel_the_power(base: f64, exponent: f64) -> f64 {
    MY_ERR.with(|err| *err.borrow_mut() = 0);
    let result = my_pow(base, exponent);

    MY_ERR.with(|err| {
        let error_flag = *err.borrow();
        if error_flag != 0 {
            crate::ffi::write_string("Oh no, there was an error! How rude.\n");
        } else {
            let output = format!("Result: {:.2}\n", result);
            crate::ffi::write_string(&output);
        }
    });

    result
}
