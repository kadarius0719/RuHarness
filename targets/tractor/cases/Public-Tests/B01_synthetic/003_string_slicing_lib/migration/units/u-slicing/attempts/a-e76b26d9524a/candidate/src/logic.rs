pub fn slice(mystr: &[u8], start_ptr: Option<&i32>, stop_ptr: Option<&i32>) -> i32 {
    let len = mystr.len() as i32;

    let start = if let Some(start_ref) = start_ptr {
        *start_ref
    } else {
        0
    };

    if start > len || start < 0 {
        output_string("Error: start is off the end of the string!\n");
        return 1;
    }

    let stop = if let Some(stop_ref) = stop_ptr {
        let s = *stop_ref;
        if s > len || s < 0 {
            output_string("Error: stop is off the end of the string!\n");
            return 1;
        }
        if s <= start {
            output_string("Error: stop must come after start!\n");
            return 1;
        }
        s
    } else {
        len
    };

    let start_usize = start as usize;
    let stop_usize = stop as usize;
    for i in start_usize..stop_usize {
        if i < mystr.len() {
            crate::ffi::put_byte(mystr[i]);
        }
    }
    crate::ffi::put_byte(b'\n');

    0
}

fn output_string(s: &str) {
    for byte in s.bytes() {
        crate::ffi::put_byte(byte);
    }
}
