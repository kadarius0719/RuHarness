pub fn driver(f: f64) {
    print_hex_u64(f.to_bits());
    crate::ffi::put_char(b' ');
    print_hex_float(f);
    crate::ffi::put_char(b' ');
    print_decimal_fixed_4(f);
    crate::ffi::put_char(b'\n');
}

fn print_hex_u64(mut n: u64) {
    let mut digits = [0u8; 16];
    let mut count = 0;
    let mut temp = n;

    while temp > 0 {
        let digit = (temp & 0xF) as u8;
        digits[count] = if digit < 10 { b'0' + digit } else { b'a' + digit - 10 };
        temp >>= 4;
        count += 1;
    }

    if count == 0 {
        crate::ffi::put_char(b'0');
    } else {
        for i in (0..count).rev() {
            crate::ffi::put_char(digits[i]);
        }
    }
}

fn print_hex_float(f: f64) {
    if f.is_nan() {
        for &ch in b"nan" {
            crate::ffi::put_char(ch);
        }
        return;
    }

    let bits = f.to_bits();
    let sign_bit = bits >> 63;
    let exponent = ((bits >> 52) & 0x7FF) as i32;
    let mantissa = bits & 0xFFFFFFFFFFFFF;

    if sign_bit == 1 {
        crate::ffi::put_char(b'-');
    }

    if exponent == 0x7FF {
        for &ch in b"inf" {
            crate::ffi::put_char(ch);
        }
        return;
    }

    for &ch in b"0x" {
        crate::ffi::put_char(ch);
    }

    if exponent == 0 && mantissa == 0 {
        crate::ffi::put_char(b'0');
        for &ch in b"p+0" {
            crate::ffi::put_char(ch);
        }
        return;
    }

    if exponent == 0 {
        let msb_pos = 63i32 - (mantissa.leading_zeros() as i32);
        let actual_exp = -1074i32 + msb_pos;
        let shift_amount = (51 - msb_pos) as u32;
        let normalized_mantissa = mantissa << shift_amount;

        crate::ffi::put_char(b'1');

        let mut last_nonzero = -1i32;
        for i in (0..13).rev() {
            let shift = i * 4;
            let hex_digit = ((normalized_mantissa >> shift) & 0xF) as u8;
            if hex_digit != 0 {
                last_nonzero = i;
            }
        }

        if last_nonzero >= 0 {
            crate::ffi::put_char(b'.');
            for i in (0..=last_nonzero).rev() {
                let shift = i * 4;
                let hex_digit = ((normalized_mantissa >> shift) & 0xF) as u8;
                let ch = if hex_digit < 10 { b'0' + hex_digit } else { b'a' + hex_digit - 10 };
                crate::ffi::put_char(ch);
            }
        }

        crate::ffi::put_char(b'p');
        if actual_exp >= 0 {
            crate::ffi::put_char(b'+');
            print_i32_unsigned(actual_exp as u32);
        } else {
            crate::ffi::put_char(b'-');
            print_i32_unsigned((-actual_exp) as u32);
        }
        return;
    }

    crate::ffi::put_char(b'1');

    let mut last_nonzero = -1i32;
    for i in (0..13).rev() {
        let shift = i * 4;
        let hex_digit = ((mantissa >> shift) & 0xF) as u8;
        if hex_digit != 0 {
            last_nonzero = i;
        }
    }

    if last_nonzero >= 0 {
        crate::ffi::put_char(b'.');
        for i in (0..=last_nonzero).rev() {
            let shift = i * 4;
            let hex_digit = ((mantissa >> shift) & 0xF) as u8;
            let ch = if hex_digit < 10 { b'0' + hex_digit } else { b'a' + hex_digit - 10 };
            crate::ffi::put_char(ch);
        }
    }

    crate::ffi::put_char(b'p');
    let actual_exp = exponent - 1023;
    if actual_exp >= 0 {
        crate::ffi::put_char(b'+');
        print_i32_unsigned(actual_exp as u32);
    } else {
        crate::ffi::put_char(b'-');
        print_i32_unsigned((-actual_exp) as u32);
    }
}

fn print_i32_unsigned(mut n: u32) {
    if n == 0 {
        crate::ffi::put_char(b'0');
        return;
    }

    let mut digits = [0u8; 10];
    let mut count = 0;

    while n > 0 {
        digits[count] = ((n % 10) as u8);
        n /= 10;
        count += 1;
    }

    for i in (0..count).rev() {
        crate::ffi::put_char(b'0' + digits[i]);
    }
}

fn print_decimal_fixed_4(f: f64) {
    if f.is_nan() {
        for &ch in b"nan" {
            crate::ffi::put_char(ch);
        }
        return;
    }

    if f.is_infinite() {
        if f < 0.0 {
            crate::ffi::put_char(b'-');
        }
        for &ch in b"inf" {
            crate::ffi::put_char(ch);
        }
        return;
    }

    let bits = f.to_bits();
    let is_negative = (bits >> 63) != 0;

    if is_negative {
        crate::ffi::put_char(b'-');
        print_decimal_fixed_4(-f);
        return;
    }

    let scaled = (f * 10000.0 + 0.5).floor() as i64;
    let int_part = scaled / 10000;
    let frac_part = scaled % 10000;

    print_i64(int_part);
    crate::ffi::put_char(b'.');

    if frac_part < 1000 {
        crate::ffi::put_char(b'0');
    }
    if frac_part < 100 {
        crate::ffi::put_char(b'0');
    }
    if frac_part < 10 {
        crate::ffi::put_char(b'0');
    }

    print_i64(frac_part);
}

fn print_i64(mut n: i64) {
    if n == 0 {
        crate::ffi::put_char(b'0');
        return;
    }

    if n < 0 {
        crate::ffi::put_char(b'-');
        if n == i64::MIN {
            for &ch in b"9223372036854775808" {
                crate::ffi::put_char(ch);
            }
        } else {
            print_i64(-n);
        }
        return;
    }

    let mut digits = [0u8; 20];
    let mut count = 0;

    while n > 0 {
        digits[count] = ((n % 10) as u8);
        n /= 10;
        count += 1;
    }

    for i in (0..count).rev() {
        crate::ffi::put_char(b'0' + digits[i]);
    }
}
