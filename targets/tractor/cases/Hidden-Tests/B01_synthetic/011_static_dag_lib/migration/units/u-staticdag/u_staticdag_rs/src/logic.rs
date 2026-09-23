use std::sync::atomic::{AtomicI32, Ordering};

static RUN: AtomicI32 = AtomicI32::new(0);

pub fn static_update(update: bool, new_value: i32) -> i32 {
    if update {
        RUN.store(new_value, Ordering::SeqCst);
    }
    RUN.load(Ordering::SeqCst)
}

pub fn path_mult(update: i32) {
    let run = static_update(false, 0);
    let run = run.wrapping_mul(update);
    let _updated = static_update(true, run);
}

pub fn path_add(update: i32) {
    let run = static_update(false, 0);
    let run = run.wrapping_add(update);
    let _updated = static_update(true, run);
}

pub fn path_subtract(update: i32) {
    let run = static_update(false, 0);
    let run = run.wrapping_sub(update);
    let _updated = static_update(true, run);
}

pub fn driver(val: i32, iterations: i32) {
    if iterations > 0 {
        for _i in 0..(iterations as usize) {
            path_add(val);
            crate::ffi::output_int(static_update(false, 0));
            path_mult(val);
            crate::ffi::output_int(static_update(false, 0));
            path_subtract(val);
            crate::ffi::output_int(static_update(false, 0));
        }
    }
}
