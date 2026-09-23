use crate::logic;

extern "C" {
    fn putchar(c: i32) -> i32;
}

pub fn put_byte(b: u8) {
    unsafe { putchar(b as i32); }
}

pub fn put_bytes(bytes: &[u8]) {
    for &b in bytes {
        put_byte(b);
    }
}

#[no_mangle]
pub unsafe extern "C" fn driver(s_in: *mut u8) {
    if s_in.is_null() {
        return;
    }

    let mut len = 0;
    while *s_in.add(len) != 0 {
        len += 1;
    }

    let slice = std::slice::from_raw_parts_mut(s_in, len);
    logic::driver(slice);
}
