use std::ffi::CStr;
use crate::logic::{HouseT, run_logic, parse_val};

extern "C" {
    fn printf(fmt: *const u8, ...) -> i32;
}

pub fn printf_house(floors: i32, bedrooms: i32, bathrooms: f64) {
    unsafe {
        let fmt = b"The house has %d floors, %d bedrooms, and %.1f bathrooms\n\0".as_ptr();
        printf(fmt, floors, bedrooms, bathrooms);
    }
}

pub fn printf_error() {
    unsafe {
        let fmt = b"An error occurred\n\0".as_ptr();
        printf(fmt);
    }
}

static mut THE_HOUSE: HouseT = HouseT {
    floors: 2,
    bedrooms: 5,
    bathrooms: 2.5,
};

#[no_mangle]
pub unsafe extern "C" fn run(extra_bedrooms: i32) {
    run_logic(&mut THE_HOUSE, extra_bedrooms);
}

#[no_mangle]
pub unsafe extern "C" fn driver(in_ptr: *const i8) {
    if !in_ptr.is_null() {
        if let Ok(input_str) = CStr::from_ptr(in_ptr).to_str() {
            if let Some(x) = parse_val(input_str) {
                run(x);
                run(x);
            } else {
                printf_error();
            }
        } else {
            printf_error();
        }
    } else {
        printf_error();
    }
}
