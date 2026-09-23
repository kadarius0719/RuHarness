use std::sync::Mutex;

static SUM: Mutex<i32> = Mutex::new(0);

pub fn static_sum(update: i32) -> i32 {
    let mut sum_guard = SUM.lock().unwrap();
    *sum_guard = (*sum_guard).wrapping_add(update);
    *sum_guard
}

pub fn driver(stride: i32) {
    for i in 0i32..10 {
        let result = static_sum(i.wrapping_mul(stride));
        output_i32(result);
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
