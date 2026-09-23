use std::ffi::CString;

extern "C" {
    fn printf(fmt: *const u8, ...) -> i32;
}

pub fn printf_line(s: &str) {
    unsafe {
        let fmt = b"%s\n\0".as_ptr();
        let c_str = CString::new(s).unwrap();
        printf(fmt, c_str.as_ptr());
    }
}

pub fn printf_result(result: i32) {
    unsafe {
        let fmt = b"Result: %d\n\0".as_ptr();
        printf(fmt, result);
    }
}

#[no_mangle]
pub unsafe extern "C" fn driver(x: i32, local_y: i32, z: i32) {
    crate::logic::driver_impl(x, local_y, z);
}
