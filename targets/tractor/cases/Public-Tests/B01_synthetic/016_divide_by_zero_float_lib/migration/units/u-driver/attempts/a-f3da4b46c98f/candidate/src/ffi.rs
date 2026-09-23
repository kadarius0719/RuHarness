use std::ffi::{CStr, CString};

extern "C" {
    fn printf(fmt: *const u8, ...) -> i32;
}

pub fn printf_string(s: &str) {
    unsafe {
        let fmt = b"%s\n\0".as_ptr();
        let c_str = CString::new(s).unwrap();
        printf(fmt, c_str.as_ptr());
    }
}

pub fn printf_int(i: i32) {
    unsafe {
        let fmt = b"%d\n\0".as_ptr();
        printf(fmt, i);
    }
}

#[no_mangle]
pub unsafe extern "C" fn printLine(line: *const i8) {
    if !line.is_null() {
        if let Ok(s) = CStr::from_ptr(line as *const _).to_str() {
            crate::logic::printLine(s);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn printIntLine(intNumber: i32) {
    crate::logic::printIntLine(intNumber);
}

#[no_mangle]
pub unsafe extern "C" fn bad(data: f32) {
    crate::logic::bad(data);
}

#[no_mangle]
pub unsafe extern "C" fn good(data: f32) {
    crate::logic::good(data);
}

#[no_mangle]
pub unsafe extern "C" fn driver(goodData: f32, badData: f32) {
    crate::logic::driver(goodData, badData);
}
