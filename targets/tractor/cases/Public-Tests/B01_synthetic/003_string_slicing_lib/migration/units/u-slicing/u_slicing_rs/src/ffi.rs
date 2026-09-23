extern "C" {
    fn putchar(c: i32) -> i32;
}

pub fn put_byte(b: u8) {
    unsafe { putchar(b as i32); }
}

#[no_mangle]
pub unsafe extern "C" fn slice(mystr: *const i8, start_ptr: *const i32, stop_ptr: *const i32) -> i32 {
    if mystr.is_null() {
        return 1;
    }

    let c_str = std::ffi::CStr::from_ptr(mystr);
    let mystr_bytes = c_str.to_bytes();

    let start_ref = if start_ptr.is_null() {
        None
    } else {
        Some(&*start_ptr)
    };

    let stop_ref = if stop_ptr.is_null() {
        None
    } else {
        Some(&*stop_ptr)
    };

    crate::logic::slice(mystr_bytes, start_ref, stop_ref)
}
