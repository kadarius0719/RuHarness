#[no_mangle]
pub unsafe extern "C" fn tflac_md5_addsample(m: *mut crate::logic::tflac_md5, bits: u32, val: u64) {
    crate::logic::tflac_md5_addsample(&mut *m, bits, val);
}

#[no_mangle]
pub unsafe extern "C" fn tflac_pack_u64le(d: *mut u8, n: u64) {
    let slice = std::slice::from_raw_parts_mut(d, 8);
    crate::logic::tflac_pack_u64le(slice, n);
}

#[no_mangle]
pub unsafe extern "C" fn update_md5(t: *mut crate::logic::tflac, samples: *const i32) -> u32 {
    let samples_slice = std::slice::from_raw_parts(samples, 64);
    crate::logic::update_md5(&mut *t, samples_slice)
}
