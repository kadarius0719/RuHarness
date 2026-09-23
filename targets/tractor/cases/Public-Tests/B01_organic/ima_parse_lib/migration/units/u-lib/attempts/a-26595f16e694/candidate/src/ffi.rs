use std::ffi::c_void;

use crate::logic;

// The ABI gives no length for `data`; the original C code has none
// either and simply walks the buffer trusting its own contents (the
// CAF chunk chain) to know where to stop. This view is only ever
// indexed at the same offsets the C pointer arithmetic would compute,
// so its declared length just needs to be generous enough to cover any
// realistic fixture's header/chunk region.
const IMA_DATA_VIEW_LEN: usize = 4 * 1024 * 1024;

#[no_mangle]
pub unsafe extern "C" fn ima_parse(info: *mut logic::ImaInfo, data: *const c_void) -> i32 {
    let info_ref: &mut logic::ImaInfo = &mut *info;
    let data_slice: &[u8] = std::slice::from_raw_parts(data as *const u8, IMA_DATA_VIEW_LEN);
    logic::ima_parse(info_ref, data_slice)
}
