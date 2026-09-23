#[repr(C)]
pub struct Tflac {
    pub bitdepth: u32,
    pub cur_blocksize: u32,
    pub subframe_bitdepth: u32,
    pub constant: u8,
    pub residual_errors: [u64; 5],
    pub residuals: [i32; 5],
}

pub fn decorrelate(t: &mut Tflac, channel: u32, stride: u32) {
    let mut i = 0u32;
    let mut l = 0u32;
    let mut r = 0u32;
    let mut non_constant = 0u32;
    let mut min_found = 0u32;

    if channel == 0 {
        t.subframe_bitdepth = t.bitdepth;
        while i < t.cur_blocksize && i <= 5 {
            t.residuals[i as usize] = l as i32;
            non_constant |= (t.residuals[i as usize] as u32) ^ (t.residuals[0] as u32);
            min_found |= if t.residuals[i as usize] == i32::MIN { 1 } else { 0 };
            i = i.wrapping_add(1);
            l = l.wrapping_add(stride);
        }
    } else {
        t.subframe_bitdepth = t.bitdepth.wrapping_add(1);
        while i < t.cur_blocksize && i <= 5 {
            t.residuals[i as usize] = (l as i32).wrapping_sub(r as i32);
            non_constant |= (t.residuals[i as usize] as u32) ^ (t.residuals[0] as u32);
            min_found |= if t.residuals[i as usize] == i32::MIN { 1 } else { 0 };
            i = i.wrapping_add(1);
            l = l.wrapping_add(stride);
            r = r.wrapping_add(stride);
        }
    }
    t.constant = if non_constant == 0 { 1 } else { 0 };
    t.residual_errors[0] = if min_found != 0 { u64::MAX } else { 0 };
}
