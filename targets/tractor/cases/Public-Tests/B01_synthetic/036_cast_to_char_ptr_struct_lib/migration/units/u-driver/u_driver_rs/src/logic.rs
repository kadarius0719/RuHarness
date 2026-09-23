pub fn driver_impl(floors: i32) {
    let bedrooms = 3i32;
    let bathrooms = 2.0f64;

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&floors.to_ne_bytes());
    bytes.extend_from_slice(&bedrooms.to_ne_bytes());
    bytes.extend_from_slice(&bathrooms.to_ne_bytes());

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
