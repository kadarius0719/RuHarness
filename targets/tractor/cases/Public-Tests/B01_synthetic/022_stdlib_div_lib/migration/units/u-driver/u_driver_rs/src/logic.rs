pub fn driver(x: i32, y: i32) {
    let quot = x / y;
    let rem = x % y;
    crate::ffi::printf_division(quot, rem);
}
