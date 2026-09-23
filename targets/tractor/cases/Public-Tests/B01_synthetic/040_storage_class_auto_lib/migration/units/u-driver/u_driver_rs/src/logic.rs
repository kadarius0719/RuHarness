pub fn driver(x: i32) {
    let y = x.wrapping_mul(2).wrapping_add(300);
    print_int(y);
    crate::ffi::put_char(b'\n');
}

fn print_int(n: i32) {
    if n == 0 {
        crate::ffi::put_char(b'0');
        return;
    }

    if n < 0 {
        crate::ffi::put_char(b'-');
        if n == i32::MIN {
            for &ch in b"2147483648" {
                crate::ffi::put_char(ch);
            }
        } else {
            print_positive(-n);
        }
    } else {
        print_positive(n);
    }
}

fn print_positive(mut n: i32) {
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
