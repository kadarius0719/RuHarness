#[repr(C)]
pub struct BsT {
    pub buf: *const u8,
    pub pos: i32,
    pub limit: i32,
}

#[repr(C)]
pub struct L12ScaleInfo {
    pub scf: [f32; 192],
    pub total_bands: u8,
    pub stereo_bands: u8,
    pub bitalloc: [u8; 64],
    pub scfcod: [u8; 64],
}

pub fn dequantize_granule(grbuf: &mut [f32], buf: &[u8], bs_pos: &mut i32, bs_limit: i32, sci: &L12ScaleInfo, group_size: i32) -> i32 {
    let mut choff = 576;

    for j in 0..4 {
        let dst_base = (group_size * j) as usize;
        let mut dst_offset = 0usize;

        for i in 0..(2 * sci.total_bands as i32) {
            let ba = sci.bitalloc[i as usize];

            if ba != 0 {
                if ba < 17 {
                    let half = ((1i32 << (ba - 1)) - 1) as i32;
                    for k in 0..group_size {
                        let bits = get_bits(buf, bs_pos, bs_limit, ba as i32) as i32;
                        let dst_idx = dst_base + dst_offset + (k as usize);
                        if dst_idx < grbuf.len() {
                            grbuf[dst_idx] = (bits - half) as f32;
                        }
                    }
                } else {
                    let mod_val = (2i32 << (ba - 17)) + 1;
                    let bits_to_read = (mod_val + 2 - (mod_val >> 3)) as i32;
                    let mut code = get_bits(buf, bs_pos, bs_limit, bits_to_read) as i32;

                    for k in 0..group_size {
                        let dst_idx = dst_base + dst_offset + (k as usize);
                        if dst_idx < grbuf.len() {
                            grbuf[dst_idx] = ((code % mod_val) - (mod_val / 2)) as f32;
                            code /= mod_val;
                        }
                    }
                }
            }

            dst_offset += choff as usize;
            choff = 18 - choff;
        }
    }

    group_size * 4
}

fn get_bits(buf: &[u8], pos: &mut i32, limit: i32, n: i32) -> u32 {
    let mut next: u32;
    let mut cache: u32 = 0;
    let s = (*pos & 7) as u32;
    let mut shl = (n + (*pos & 7)) as i32;

    let p_idx = (*pos >> 3) as usize;
    *pos += n;

    if *pos > limit {
        return 0;
    }

    if p_idx >= buf.len() {
        return 0;
    }

    next = (buf[p_idx] as u32) & (255u32 >> s);

    let mut p_idx = p_idx + 1;
    while shl > 8 {
        cache |= next << (shl - 8);
        if p_idx < buf.len() {
            next = buf[p_idx] as u32;
            p_idx += 1;
        } else {
            next = 0;
        }
        shl -= 8;
    }

    if shl > 0 {
        cache | (next >> (8 - shl))
    } else if shl < 0 {
        cache | (next >> (-shl))
    } else {
        cache
    }
}
