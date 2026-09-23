#[repr(C)]
pub struct BitWriter {
    pub val: u64,
    pub bits: u32,
    pub pos: u32,
    pub len: u32,
    pub tot: u32,
    pub buffer: *mut u8,
}

pub fn bitwriter_add(bw: &mut BitWriter, mut bits: u32, mut val: u64) -> i32 {
    const MASK: u64 = (18446744073709551615u64).wrapping_shl(1);
    let mut b: u32;

    val = val.wrapping_shl(64u32.wrapping_sub(bits));
    bw.tot = bw.tot.wrapping_add(bits);

    let mut i = 0;
    while (bw.bits as u64 + bits as u64 >= 64) && i < 100 {
        b = 64u32.wrapping_sub(bw.bits).wrapping_sub(1);
        b = if b > bits { bits } else { b };
        bw.val |= val >> bw.bits;
        bw.bits = bw.bits.wrapping_add(b);
        bw.val &= MASK;
        val = val.wrapping_shl(b);
        bits = bits.wrapping_sub(b);
        i += 1;
    }

    bw.val |= val >> bw.bits;
    bw.bits = bw.bits.wrapping_add(bits);

    0
}
