use crate::logic;

#[no_mangle]
pub unsafe extern "C" fn md5_digest(m: *const logic::Md5, out: *mut u8) {
    let m_ref = &*m;
    let out_slice = std::slice::from_raw_parts_mut(out, 16);
    logic::md5_digest(m_ref, out_slice);
}
