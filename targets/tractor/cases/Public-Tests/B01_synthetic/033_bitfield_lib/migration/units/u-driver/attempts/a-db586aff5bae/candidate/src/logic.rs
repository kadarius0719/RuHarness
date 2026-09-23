pub fn print_foo_impl(x: u32, y: u32, b: bool, z: i32) {
    let b_int = if b { 1 } else { 0 };
    let line = format!("{} {} {} {}\n", x, y, b_int, z);
    for byte in line.as_bytes() {
        crate::ffi::put_byte(*byte);
    }
}

pub fn driver_impl(x: u32, y: u32, b: bool, z: i32) {
    let x_truncated = x & 0b11;
    let y_truncated = y & 0b111;
    print_foo_impl(x_truncated, y_truncated, b, z);
}
