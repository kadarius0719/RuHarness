extern "C" {
    fn putchar(c: i32) -> i32;
}

pub fn put_byte(b: u8) {
    unsafe { putchar(b as i32); }
}

static mut INNER: i32 = 1;

pub fn call_static_alias_with_static(outer_val: i32) -> (i32, bool) {
    unsafe {
        if outer_val >= INNER {
            INNER = INNER.wrapping_add(outer_val);
            (INNER, true)
        } else {
            (outer_val.wrapping_add(INNER), false)
        }
    }
}

pub fn call_static_alias_with_outer(outer: &mut i32) -> (i32, bool) {
    unsafe {
        if *outer >= INNER {
            INNER = INNER.wrapping_add(*outer);
            (*outer, false)
        } else {
            *outer = (*outer).wrapping_add(INNER);
            (*outer, false)
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn static_alias(outer: *mut i32) -> *mut i32 {
    if *outer >= INNER {
        INNER = INNER.wrapping_add(*outer);
        &mut INNER
    } else {
        *outer = (*outer).wrapping_add(INNER);
        outer
    }
}

#[no_mangle]
pub extern "C" fn driver(initial_value: i32, iterations: i32) {
    crate::logic::driver(initial_value, iterations);
}
