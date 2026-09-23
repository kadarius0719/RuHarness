#[no_mangle]
pub unsafe extern "C" fn synth_pair(pcm: *mut i16, nch: i32, z: *const f32) {
    if !pcm.is_null() && !z.is_null() {
        let pcm_len = if nch > 0 { (16 * nch) as usize + 1 } else { 1 };
        let z_len = 15 * 64 + 1;
        let pcm_slice = std::slice::from_raw_parts_mut(pcm, pcm_len);
        let z_slice = std::slice::from_raw_parts(z, z_len);
        crate::logic::synth_pair_inner(pcm_slice, nch, z_slice);
    }
}
