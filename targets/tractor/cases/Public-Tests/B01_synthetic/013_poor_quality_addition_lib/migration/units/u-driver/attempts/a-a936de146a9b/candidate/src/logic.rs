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

pub fn bad() {
    let int_one = 1;
    let int_two = 1;
    let mut int_sum = 0;
    print_int_line(int_sum);
    let _ = int_one + int_two;
    print_int_line(int_sum);
}

pub fn good() {
    let int_one = 1;
    let int_two = 1;
    let mut int_sum = 0;
    print_int_line(int_sum);
    int_sum = int_one + int_two;
    print_int_line(int_sum);
}

pub fn driver() {
    print_line("Calling good()...");
    good();
    print_line("Finished good()");
    print_line("Calling bad()...");
    bad();
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
