use crate::logic;

#[repr(C)]
pub struct bs_t {
    pub buf: *const u8,
    pub pos: i32,
    pub limit: i32,
}

#[no_mangle]
pub unsafe extern "C" fn read_scalefactors(
    bs: *mut bs_t,
    pba: *mut u8,
    scfcod: *mut u8,
    bands: i32,
    scf: *mut f32,
) {
    let bs_ref: &mut bs_t = &mut *bs;

    let limit: i32 = bs_ref.limit;
    let buf_len: usize = if limit > 0 {
        ((limit as i64 + 7) / 8) as usize
    } else {
        0
    };
    let buf_slice: &[u8] = if buf_len == 0 || bs_ref.buf.is_null() {
        &[]
    } else {
        std::slice::from_raw_parts(bs_ref.buf, buf_len)
    };

    let n: usize = if bands > 0 { bands as usize } else { 0 };

    let pba_slice: &[u8] = if n == 0 || pba.is_null() {
        &[]
    } else {
        std::slice::from_raw_parts(pba as *const u8, n)
    };

    let scfcod_slice: &[u8] = if n == 0 || scfcod.is_null() {
        &[]
    } else {
        std::slice::from_raw_parts(scfcod as *const u8, n)
    };

    let scf_len: usize = n.saturating_mul(3);
    let scf_slice: &mut [f32] = if scf_len == 0 || scf.is_null() {
        &mut []
    } else {
        std::slice::from_raw_parts_mut(scf, scf_len)
    };

    let mut bitstream = logic::BitStream {
        buf: buf_slice,
        pos: bs_ref.pos,
        limit: bs_ref.limit,
    };

    logic::read_scalefactors(&mut bitstream, pba_slice, scfcod_slice, bands, scf_slice);

    bs_ref.pos = bitstream.pos;
}
