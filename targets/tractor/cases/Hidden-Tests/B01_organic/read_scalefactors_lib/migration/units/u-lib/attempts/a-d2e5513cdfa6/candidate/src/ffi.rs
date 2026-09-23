use crate::logic::{BitStream, BsT};

#[no_mangle]
pub unsafe extern "C" fn read_scalefactors(
    bs: *mut BsT,
    pba: *const u8,
    scfcod: *const u8,
    bands: i32,
    scf: *mut f32,
) {
    if bands <= 0 {
        return;
    }

    let buf_len = (((*bs).limit as usize) + 15) >> 3;
    let buf_slice = core::slice::from_raw_parts((*bs).buf, buf_len);

    let mut bs_rust = BitStream {
        buf: buf_slice,
        pos: (*bs).pos,
        limit: (*bs).limit,
    };

    let pba_slice = core::slice::from_raw_parts(pba, bands as usize);
    let scfcod_slice = core::slice::from_raw_parts(scfcod, bands as usize);
    let scf_slice = core::slice::from_raw_parts_mut(scf, (bands as usize) * 4);

    crate::logic::read_scalefactors(&mut bs_rust, pba_slice, scfcod_slice, scf_slice);

    (*bs).pos = bs_rust.pos;
}
