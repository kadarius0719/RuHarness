use crate::logic::TflacMd5;

#[no_mangle]
pub unsafe extern "C" fn md5_transform(m: *mut TflacMd5) {
    let m_ref = &mut *m;
    crate::logic::md5_transform(m_ref);
}

#[no_mangle]
pub unsafe extern "C" fn tflac_unpack_u32le(d: *const u8) -> u32 {
    let d_slice = core::slice::from_raw_parts(d, 4);
    crate::logic::tflac_unpack_u32le(d_slice)
}
