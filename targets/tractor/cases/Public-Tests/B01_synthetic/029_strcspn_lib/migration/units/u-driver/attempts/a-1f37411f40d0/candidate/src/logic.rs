pub fn driver_impl(s1: &[u8], s2: &[u8]) {
    // Implement strcspn: find the length of the prefix of s1
    // that does not contain any character from s2
    let result = s1.iter().position(|&b| s2.contains(&b)).unwrap_or(s1.len());

    // Convert result to string and output each byte followed by newline
    let result_str = result.to_string();
    for &b in result_str.as_bytes() {
        crate::ffi::put_byte(b);
    }
    crate::ffi::put_byte(b'\n');
}
