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

pub fn printf_hex_byte(byte: i8) {
    unsafe {
        let fmt = b"%02x\n\0".as_ptr();
        printf(fmt, byte as i32);
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
pub unsafe extern "C" fn printHexCharLine(charHex: i8) {
    crate::logic::printHexCharLine(charHex);
}

#[no_mangle]
pub unsafe extern "C" fn bad() {
    crate::logic::bad();
}

#[no_mangle]
pub unsafe extern "C" fn good() {
    crate::logic::good();
}

#[no_mangle]
pub unsafe extern "C" fn driver(useGood: i32) {
    crate::logic::driver(useGood);
}
