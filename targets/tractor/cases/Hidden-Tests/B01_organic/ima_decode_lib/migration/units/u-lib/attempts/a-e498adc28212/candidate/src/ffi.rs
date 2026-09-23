use crate::logic::{ImaBlock, ImaChannelState};

#[no_mangle]
pub unsafe extern "C" fn ima_decode(
    output: *mut f32,
    channel_count: u32,
    block: *const ImaBlock,
    decode_count: u64,
    state: *mut ImaChannelState,
) {
    let output_slice = core::slice::from_raw_parts_mut(output, 1);
    let block_ref = &*block;
    let state_ref = &mut *state;

    crate::logic::ima_decode(output_slice, channel_count, block_ref, decode_count, state_ref);
}
