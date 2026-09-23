extern "C" {
    fn putchar(c: i32) -> i32;
}

pub fn print_int(val: i32) {
    let s = val.to_string();
    for byte in s.as_bytes() {
        unsafe { putchar(*byte as i32); }
    }
    unsafe { putchar(b'\n' as i32); }
}

#[no_mangle]
pub unsafe extern "C" fn fma_array(
    out: *mut i32,
    mul1: *const i32,
    mul2: *const i32,
    add: *const i32,
    len: i32,
) {
    if len < 0 {
        return;
    }
    let len = len as usize;

    let out_const = out as *const i32;
    if out_const == mul1 && mul1 == mul2 as *const i32 && mul2 as *const i32 == add {
        let out_slice = std::slice::from_raw_parts_mut(out, len);
        crate::logic::fma_array_aliased(out_slice);
    } else {
        let out_slice = std::slice::from_raw_parts_mut(out, len);
        let mul1_slice = std::slice::from_raw_parts(mul1, len);
        let mul2_slice = std::slice::from_raw_parts(mul2, len);
        let add_slice = std::slice::from_raw_parts(add, len);
        crate::logic::fma_array(out_slice, mul1_slice, mul2_slice, add_slice);
    }
}

#[no_mangle]
pub unsafe extern "C" fn driver(data: *const i32, len: i32) {
    if len < 0 {
        return;
    }
    let len = len as usize;
    let data_slice = std::slice::from_raw_parts(data, len);
    crate::logic::driver_logic(data_slice);
}
