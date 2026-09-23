use crate::logic;

extern "C" {
    fn putchar(c: i32) -> i32;
}

pub fn put_byte(b: u8) {
    unsafe { putchar(b as i32); }
}

#[no_mangle]
pub unsafe extern "C" fn fma_array(out: *mut i32, mul1: *const i32, mul2: *const i32, add: *const i32, len: i32) {
    if out.is_null() || mul1.is_null() || mul2.is_null() || add.is_null() || len <= 0 {
        return;
    }

    let len = len as usize;
    let out_slice = std::slice::from_raw_parts_mut(out, len);
    let mul1_slice = std::slice::from_raw_parts(mul1, len);
    let mul2_slice = std::slice::from_raw_parts(mul2, len);
    let add_slice = std::slice::from_raw_parts(add, len);

    logic::fma_array(out_slice, mul1_slice, mul2_slice, add_slice);
}

#[no_mangle]
pub unsafe extern "C" fn driver(data: *const i32, len: i32) {
    if data.is_null() || len <= 0 {
        return;
    }

    let len = len as usize;
    let data_slice = std::slice::from_raw_parts(data, len);
    logic::driver(data_slice);
}
