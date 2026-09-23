pub fn sieve(mut val: i32) {
    loop {
        output_i32(val);
        crate::ffi::put_byte(b'\n');
        if val % 10 == 9 {
            break;
        }
        val = val.wrapping_add(1);
    }
}

fn output_i32(val: i32) {
    if val < 0 {
        crate::ffi::put_byte(b'-');
        output_u32((val.wrapping_neg()) as u32);
    } else {
        output_u32(val as u32);
    }
}

fn output_u32(mut val: u32) {
    if val == 0 {
        crate::ffi::put_byte(b'0');
        return;
    }

    let mut digits = [0u8; 10];
    let mut count = 0;
    while val > 0 {
        digits[count] = (val % 10) as u8 + b'0';
        val /= 10;
        count += 1;
    }

    for i in (0..count).rev() {
        crate::ffi::put_byte(digits[i]);
    }
}
