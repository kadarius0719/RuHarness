#[no_mangle]
pub unsafe extern "C" fn read_side_info(bs: *mut crate::logic::BsT, gr: *mut crate::logic::L3GrInfoT, hdr: *const u8) -> i32 {
    if !bs.is_null() && !gr.is_null() && !hdr.is_null() {
        let bs_ref = &mut *bs;
        let buf_len = ((bs_ref.limit + 7) >> 3) as usize;
        let buf = std::slice::from_raw_parts(bs_ref.buf, buf_len);
        let hdr_slice = std::slice::from_raw_parts(hdr, 4);

        let sr_idx = ((((hdr_slice[2]) >> 2) & 3) as usize +
                      (((hdr_slice[1] >> 3) & 1) + ((hdr_slice[1] >> 4) & 1)) as usize * 3) as usize;
        let mut gr_count = if ((hdr_slice[3]) & 0xC0) == 0xC0 { 1 } else { 2 };
        if ((hdr_slice[1]) & 0x8) != 0 {
            gr_count *= 2;
        }

        let gr_slice = std::slice::from_raw_parts_mut(gr, gr_count);
        let result = crate::logic::read_side_info(buf, &mut bs_ref.pos, bs_ref.limit, gr_slice, hdr_slice);
        result
    } else {
        -1
    }
}
