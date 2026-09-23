extern "C" {
    fn printf(fmt: *const u8, ...) -> i32;
}

pub fn printf_results(count_a: i32, count_x: i32) {
    unsafe {
        let fmt = b"A: %d\n\0".as_ptr();
        printf(fmt, count_a);

        let fmt = b"x: %d\n\0".as_ptr();
        printf(fmt, count_x);
    }
}

#[no_mangle]
pub unsafe extern "C" fn foo(in_ptr: *const i8, c: i8) -> i32 {
    if in_ptr.is_null() {
        return 0;
    }
    let c_byte = c as u8;
    let mut len = 0;
    let mut ptr = in_ptr as *const u8;
    while *ptr != 0 {
        len += 1;
        ptr = ptr.offset(1);
    }
    let in_bytes = std::slice::from_raw_parts(in_ptr as *const u8, len);
    crate::logic::foo(in_bytes, c_byte)
}

#[no_mangle]
pub unsafe extern "C" fn driver(in_ptr: *const i8) {
    if in_ptr.is_null() {
        return;
    }
    let mut len = 0;
    let mut ptr = in_ptr as *const u8;
    while *ptr != 0 {
        len += 1;
        ptr = ptr.offset(1);
    }
    let in_bytes = std::slice::from_raw_parts(in_ptr as *const u8, len);
    crate::logic::driver_impl(in_bytes);
}
