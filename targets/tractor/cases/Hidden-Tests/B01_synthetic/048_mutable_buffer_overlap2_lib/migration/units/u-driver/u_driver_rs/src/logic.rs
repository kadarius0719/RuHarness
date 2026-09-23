pub fn fma_array(out: &mut [i32], mul1: &[i32], mul2: &[i32], add: &[i32]) {
    for i in 0..out.len() {
        out[i] = mul1[i].wrapping_mul(mul2[i]).wrapping_add(add[i]);
    }
}

pub fn driver(data: &[i32]) {
    let mut out = vec![0i32; data.len()];
    fma_array(&mut out, data, data, data);
    for val in out {
        print_int(val);
    }
}

pub fn print_int(val: i32) {
    let s = format!("{}\n", val);
    for &b in s.as_bytes() {
        crate::ffi::put_byte(b);
    }
}
