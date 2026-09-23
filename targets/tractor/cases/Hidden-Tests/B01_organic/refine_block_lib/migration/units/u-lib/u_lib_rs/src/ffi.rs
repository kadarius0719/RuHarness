use crate::logic;

#[no_mangle]
pub unsafe extern "C" fn refine_block(
    block: *mut u8,
    pmax16: *mut u16,
    pmin16: *mut u16,
    mask: u32,
) -> i32 {
    let block_slice = std::slice::from_raw_parts(block, 64);
    logic::refine_block(block_slice, &mut *pmax16, &mut *pmin16, mask)
}
