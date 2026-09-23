extern "C" {
    fn putchar(c: i32) -> i32;
}

pub fn put_byte(b: u8) {
    unsafe { putchar(b as i32); }
}

#[no_mangle]
pub extern "C" fn printLine(line: *const i8) {
    if line.is_null() {
        return;
    }
    unsafe {
        let c_str = std::ffi::CStr::from_ptr(line);
        if let Ok(s) = c_str.to_str() {
            crate::logic::print_line(s);
        }
    }
}

#[no_mangle]
pub extern "C" fn printIntLine(int_number: i32) {
    crate::logic::print_int_line(int_number);
}

#[no_mangle]
pub extern "C" fn bad(data: i32) {
    crate::logic::bad(data);
}

#[no_mangle]
pub extern "C" fn good(data: i32) {
    crate::logic::good(data);
}

#[no_mangle]
pub extern "C" fn driver(good_data: i32, bad_data: i32) {
    crate::logic::driver(good_data, bad_data);
}
