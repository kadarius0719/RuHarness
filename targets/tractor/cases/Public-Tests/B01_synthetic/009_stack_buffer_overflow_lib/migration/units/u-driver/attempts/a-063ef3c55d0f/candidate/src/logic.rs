pub fn print_line(line: &str) {
    for byte in line.bytes() {
        crate::ffi::put_byte(byte);
    }
    crate::ffi::put_byte(b'\n');
}

pub fn print_int_line(int_number: i32) {
    output_i32(int_number);
    crate::ffi::put_byte(b'\n');
}

pub fn bad(data: i32) {
    let mut buffer = [0i32; 10];
    if data >= 0 {
        if (data as usize) < buffer.len() {
            buffer[data as usize] = 1;
        }
        for i in 0..10 {
            print_int_line(buffer[i]);
        }
    } else {
        print_line("ERROR: Array index is negative.");
    }
}

fn good_g2b() {
    let data = 7;
    let mut buffer = [0i32; 10];
    if data >= 0 {
        buffer[data as usize] = 1;
        for i in 0..10 {
            print_int_line(buffer[i]);
        }
    } else {
        print_line("ERROR: Array index is negative.");
    }
}

fn good_b2g(data: i32) {
    let mut buffer = [0i32; 10];
    if data >= 0 && data < 10 {
        buffer[data as usize] = 1;
        for i in 0..10 {
            print_int_line(buffer[i]);
        }
    } else {
        print_line("ERROR: Array index is out-of-bounds");
    }
}

pub fn good(data: i32) {
    good_g2b();
    good_b2g(data);
}

pub fn driver(good_data: i32, bad_data: i32) {
    print_line("Calling good()...");
    good(good_data);
    print_line("Finished good()");
    print_line("Calling bad()...");
    bad(bad_data);
    print_line("Finished bad()");
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
