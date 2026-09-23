pub fn driver(buffer: &mut [u8]) {
    let mut pos = 0;
    let sep_chars = &[b':', b'/', b'\n'];

    loop {
        while pos < buffer.len() && sep_chars.contains(&buffer[pos]) {
            pos += 1;
        }

        if pos >= buffer.len() {
            break;
        }

        let start = pos;
        while pos < buffer.len() && !sep_chars.contains(&buffer[pos]) {
            pos += 1;
        }

        let token = &buffer[start..pos];
        crate::ffi::put_bytes(b"line ");
        crate::ffi::put_bytes(token);
        crate::ffi::put_byte(b'\n');

        if pos < buffer.len() {
            pos += 1;
        }
    }
}
