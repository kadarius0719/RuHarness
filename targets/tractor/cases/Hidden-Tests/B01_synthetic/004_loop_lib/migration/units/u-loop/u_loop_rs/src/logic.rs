pub fn loop_fn(max_val: i32) {
    for i in 0..=max_val {
        crate::ffi::print_int(i);
    }
}
