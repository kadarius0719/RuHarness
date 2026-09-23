pub fn print_line(line: &[u8]) {
    for &b in line {
        crate::ffi::put_byte(b);
    }
    crate::ffi::put_byte(b'\n');
}

pub fn print_hex_char_line(char_hex: i8) {
    let extended = char_hex as i32 as u32;
    let hex_str = format!("{:02x}\n", extended);
    for &b in hex_str.as_bytes() {
        crate::ffi::put_byte(b);
    }
}

pub fn bad() {
    let data = i8::MIN;
    if data < 0 {
        let result = data.wrapping_mul(2);
        print_hex_char_line(result);
    }
}

pub fn good_g2b() {
    let data = -2i8;
    if data < 0 {
        let result = data.wrapping_mul(2);
        print_hex_char_line(result);
    }
}

pub fn good_b2g() {
    let mut data = ' ' as i8;
    data = i8::MIN;
    if data < 0 {
        if data > (i8::MIN / 2) {
            let result = data.wrapping_mul(2);
            print_hex_char_line(result);
        } else {
            print_line(b"data value is too small to perform multiplication.");
        }
    }
}

pub fn good() {
    good_g2b();
    good_b2g();
}
