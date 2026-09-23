use crate::logic::Btac1cIdxstate;

#[no_mangle]
pub unsafe extern "C" fn predict_sample(
    psamp: *const i32,
    idx: i32,
    pfcn: i32,
    ridx: *const Btac1cIdxstate,
) -> i32 {
    let psamp_slice = core::slice::from_raw_parts(psamp, 8);
    let ridx_ref = &*ridx;

    crate::logic::predict_sample(psamp_slice, idx, pfcn, ridx_ref)
}
