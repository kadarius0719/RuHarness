extern "C" {
    fn printf(fmt: *const u8, ...) -> i32;
}

pub fn printf_hex_byte(byte: i8) {
    unsafe {
        let fmt = b"%02x\n\0".as_ptr();
        printf(fmt, byte as i32);
    }
}

#[no_mangle]
pub extern "C" fn printHexCharLine(char_hex: i8) {
    crate::logic::print_hex_char_line(char_hex);
}

#[no_mangle]
pub extern "C" fn driver(data: i8) {
    crate::logic::driver(data);
}
