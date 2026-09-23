use crate::logic;

extern "C" {
    fn putchar(c: i32) -> i32;
}

pub fn put_byte(b: u8) {
    unsafe { putchar(b as i32); }
}

#[no_mangle]
pub unsafe extern "C" fn printLine(line: *const u8) {
    if !line.is_null() {
        let mut len = 0;
        while *line.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(line, len);
        logic::print_line(slice);
    }
}

#[no_mangle]
pub unsafe extern "C" fn printHexCharLine(char_hex: i8) {
    logic::print_hex_char_line(char_hex);
}

#[no_mangle]
pub unsafe extern "C" fn bad() {
    logic::bad();
}

#[no_mangle]
pub unsafe extern "C" fn good() {
    logic::good();
}

#[no_mangle]
pub unsafe extern "C" fn driver(use_good: i32) {
    if use_good != 0 {
        logic::good();
    } else {
        logic::bad();
    }
}
