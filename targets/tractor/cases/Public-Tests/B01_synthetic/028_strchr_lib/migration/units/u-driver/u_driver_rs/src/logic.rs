pub fn foo(in_bytes: &[u8], c: u8) -> i32 {
    let mut res = 0;
    let mut s = in_bytes;
    while let Some(pos) = s.iter().position(|&b| b == c) {
        res += 1;
        s = &s[pos + 1..];
    }
    res
}

pub fn driver_impl(in_bytes: &[u8]) {
    let count_a = foo(in_bytes, b'A');
    let count_x = foo(in_bytes, b'x');
    crate::ffi::printf_results(count_a, count_x);
}
