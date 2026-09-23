use crate::logic::{BsT, L12ScaleInfo, dequantize_granule as dequantize_granule_logic};

#[no_mangle]
pub unsafe extern "C" fn dequantize_granule(
    grbuf: *mut f32,
    bs: *mut BsT,
    sci: *mut L12ScaleInfo,
    group_size: i32,
) -> i32 {
    let grbuf_len = (group_size * 4 * 576) as usize;
    let grbuf_slice = core::slice::from_raw_parts_mut(grbuf, grbuf_len);
    let bs_ref = &mut *bs;
    let sci_ref = &mut *sci;

    let buf_len = ((bs_ref.limit + 7) >> 3) as usize;
    let buf_slice = core::slice::from_raw_parts(bs_ref.buf, buf_len);

    dequantize_granule_logic(grbuf_slice, buf_slice, &mut bs_ref.pos, bs_ref.limit, sci_ref, group_size)
}
