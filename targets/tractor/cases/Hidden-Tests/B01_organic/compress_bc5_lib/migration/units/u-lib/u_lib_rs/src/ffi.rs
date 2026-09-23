use crate::logic::compress_alpha_block;

#[no_mangle]
pub unsafe extern "C" fn stb__CompressAlphaBlock(
    dest: *mut u8,
    src: *mut u8,
    stride: i32,
) {
    let stride_usize = stride as usize;
    let src_len = stride_usize.saturating_mul(15).saturating_add(1);
    let src_slice = core::slice::from_raw_parts(src, src_len);
    let dest_slice = core::slice::from_raw_parts_mut(dest, 8);
    compress_alpha_block(dest_slice, src_slice, stride_usize);
}

#[no_mangle]
pub unsafe extern "C" fn compress_bc5(
    dest: *mut u8,
    src: *const u8,
) {
    let src_slice = core::slice::from_raw_parts(src, 32);
    let dest_slice = core::slice::from_raw_parts_mut(dest, 16);
    crate::logic::compress_bc5(dest_slice, src_slice);
}
