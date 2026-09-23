use crate::logic::{ImaInfo, ImaBlock};

#[no_mangle]
pub unsafe extern "C" fn ima_parse(info: *mut ImaInfo, data: *const std::ffi::c_void) -> i32 {
    if info.is_null() {
        return -1;
    }

    let data_ptr = data as *const u8;
    let data_slice = std::slice::from_raw_parts(data_ptr, 65536);
    let (ret, size, sample_rate, frame_count, channel_count, blocks_offset) =
        crate::logic::ima_parse_internal(data_slice);

    if ret == 0 {
        (*info).blocks = data_ptr.add(blocks_offset) as *const ImaBlock;
        (*info).size = size;
        (*info).sample_rate = sample_rate;
        (*info).frame_count = frame_count;
        (*info).channel_count = channel_count;
    }

    ret
}
