pub fn driver(initial_value: i32, iterations: i32) {
    let mut running_sum = initial_value;
    let mut points_to_static = false;

    for _ in 0..iterations {
        if points_to_static {
            let result = crate::ffi::call_static_alias_with_static(running_sum);
            running_sum = result.0;
            points_to_static = result.1;
        } else {
            let result = crate::ffi::call_static_alias_with_outer(&mut running_sum);
            running_sum = result.0;
            points_to_static = result.1;
        }
        output_i32(running_sum);
        crate::ffi::put_byte(b'\n');
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
