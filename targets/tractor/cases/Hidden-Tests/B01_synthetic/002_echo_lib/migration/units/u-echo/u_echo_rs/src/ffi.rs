use crate::logic;

extern "C" {
    fn printf(fmt: *const u8, ...) -> i32;
    fn putchar(c: i32) -> i32;
}

pub fn print_string(s: &str) {
    for &byte in s.as_bytes() {
        unsafe { putchar(byte as i32); }
    }
}

pub fn print_newline() {
    unsafe { putchar(b'\n' as i32); }
}

#[no_mangle]
pub unsafe extern "C" fn echo(argc: i32, argv: *mut *mut u8) -> i32 {
    if argc <= 0 {
        return 0;
    }

    let args_slice = std::slice::from_raw_parts(argv, argc as usize);
    let mut args_vec: Vec<&str> = Vec::new();

    for i in 0..(argc as usize) {
        let ptr = args_slice[i];
        let mut len = 0;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let s = std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len));
        args_vec.push(s);
    }

    logic::echo(&args_vec)
}
