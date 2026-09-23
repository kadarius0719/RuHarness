pub fn driver(floors: i32) {
    let bedrooms = 3i32;
    let bathrooms = 2.0f64;

    for &b in &floors.to_ne_bytes() {
        print_hex_byte(b);
    }
    for &b in &bedrooms.to_ne_bytes() {
        print_hex_byte(b);
    }
    for &b in &bathrooms.to_ne_bytes() {
        print_hex_byte(b);
    }
    print_newline();
}

fn print_hex_byte(b: u8) {
    let high = (b >> 4) & 0xfu8;
    let low = b & 0xfu8;

    let high_char = if high < 10u8 { b'0' + high } else { b'a' + high - 10u8 };
    let low_char = if low < 10u8 { b'0' + low } else { b'a' + low - 10u8 };

    crate::ffi::put_char(high_char);
    crate::ffi::put_char(low_char);
}

fn print_newline() {
    crate::ffi::put_char(b'\n');
}
