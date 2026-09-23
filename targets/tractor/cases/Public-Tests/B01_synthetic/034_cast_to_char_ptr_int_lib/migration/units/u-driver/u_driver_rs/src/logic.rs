pub fn driver_impl(x: i32) {
    let bytes = x.to_ne_bytes();
    print_hex(&bytes);
}

pub fn print_hex(bytes: &[u8]) {
    for &byte in bytes {
        let hex_str = format!("{:02x}", byte);
        for hex_char in hex_str.as_bytes() {
            crate::ffi::put_byte(*hex_char);
        }
    }
    crate::ffi::put_byte(b'\n');
}
