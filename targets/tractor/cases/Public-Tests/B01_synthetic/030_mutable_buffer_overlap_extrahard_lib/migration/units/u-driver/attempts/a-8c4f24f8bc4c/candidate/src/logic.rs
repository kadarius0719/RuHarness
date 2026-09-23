pub fn fma_array(out: &mut [i32], mul1: &[i32], mul2: &[i32], add: &[i32]) {
    for i in 0..out.len() {
        out[i] = mul1[i].wrapping_mul(mul2[i]).wrapping_add(add[i]);
    }
}

pub fn fma_array_aliased(out: &mut [i32]) {
    for i in 0..out.len() {
        let val = out[i];
        out[i] = val.wrapping_mul(val).wrapping_add(val);
    }
}

fn inner(out: &mut [i32]) {
    fma_array_aliased(out);
    for &val in out.iter() {
        crate::ffi::print_int(val);
    }
}

pub fn driver_logic(data: &[i32]) {
    let mut out = data.to_vec();
    inner(&mut out);
}
