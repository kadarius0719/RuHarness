use std::ffi::CStr;
use crate::logic::HouseT;

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

#[no_mangle]
pub unsafe extern "C" fn driver(in_ptr: *const i8) {
    if !in_ptr.is_null() {
        if let Ok(input_str) = CStr::from_ptr(in_ptr).to_str() {
            crate::logic::driver_logic(input_str);
        } else {
            printf_error();
        }
    } else {
        printf_error();
    }
}

#[no_mangle]
pub unsafe extern "C" fn run(the_house: *mut HouseT, extra_bedrooms: i32) {
    if !the_house.is_null() {
        let house_ref = &mut *the_house;
        crate::logic::run(house_ref, extra_bedrooms);
    }
}
