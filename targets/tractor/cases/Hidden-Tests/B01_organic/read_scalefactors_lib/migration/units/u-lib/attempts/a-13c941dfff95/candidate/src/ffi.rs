use crate::logic;

#[repr(C)]
pub struct BsT {
    pub buf: *const u8,
    pub pos: i32,
    pub limit: i32,
}

#[no_mangle]
pub unsafe extern "C" fn read_scalefactors(
    bs: *mut BsT,
    pba: *mut u8,
    scfcod: *mut u8,
    bands: i32,
    scf: *mut f32,
) {
    let bands_n: usize = if bands > 0 { bands as usize } else { 0 };

    let pba_slice: &[u8] = if pba.is_null() || bands_n == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(pba, bands_n)
    };

    let scfcod_slice: &[u8] = if scfcod.is_null() || bands_n == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(scfcod, bands_n)
    };

    let scf_len: usize = bands_n.checked_mul(3).unwrap_or(0);
    let scf_slice: &mut [f32] = if scf.is_null() || scf_len == 0 {
        &mut []
    } else {
        core::slice::from_raw_parts_mut(scf, scf_len)
    };

    // `bs` (the bs_t struct) must be touched only on the same path the C
    // touches it: inside get_bits, and only when the algorithm actually
    // calls get_bits (mask & m != 0 for some band). Defer every access to
    // *bs into this closure so it is dereferenced lazily, exactly when
    // logic::read_scalefactors would invoke the C's get_bits(bs, n).
    let mut new_pos: i32 = 0;
    let mut pos_touched = false;

    {
        let get_bits = |n: i32| -> u32 {
            if bs.is_null() {
                return 0;
            }
            unsafe {
                let cur_pos = if pos_touched { new_pos } else { (*bs).pos };
                let buf_ptr = (*bs).buf;
                let limit = (*bs).limit;

                let s = (cur_pos & 7) as u32;
                let shl0 = n + s as i32;
                let byte0 = cur_pos >> 3;
                let updated_pos = cur_pos.wrapping_add(n);

                new_pos = updated_pos;
                pos_touched = true;

                if updated_pos > limit {
                    return 0;
                }

                let read_byte = |idx: i32| -> u8 {
                    if idx < 0 || buf_ptr.is_null() {
                        0
                    } else {
                        unsafe { *buf_ptr.add(idx as usize) }
                    }
                };

                let mut next: u32 = (read_byte(byte0) as u32) & (255u32 >> s);
                let mut cache: u32 = 0;
                let mut shl = shl0;
                let mut off = byte0.wrapping_add(1);
                loop {
                    shl -= 8;
                    if shl <= 0 {
                        break;
                    }
                    cache |= next << (shl as u32);
                    next = read_byte(off) as u32;
                    off = off.wrapping_add(1);
                }
                cache | (next >> ((-shl) as u32))
            }
        };

        logic::read_scalefactors(pba_slice, scfcod_slice, bands, scf_slice, get_bits);
    }

    // Write bs->pos back only if get_bits actually ran at least once during
    // this call: if it never ran, the C never touched bs at all, and we must
    // not touch it either.
    if pos_touched && !bs.is_null() {
        unsafe {
            (*bs).pos = new_pos;
        }
    }
}
