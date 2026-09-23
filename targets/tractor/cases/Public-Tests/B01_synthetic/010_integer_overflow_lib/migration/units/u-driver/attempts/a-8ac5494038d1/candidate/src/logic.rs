pub fn print_hex_char_line(char_hex: i8) {
    crate::ffi::printf_hex_byte(char_hex);
}

pub fn driver(data: i8) {
    let result = data.wrapping_add(1);
    print_hex_char_line(result);
}
