#[repr(C)]
pub struct BsT {
    pub buf: *const u8,
    pub pos: i32,
    pub limit: i32,
}

pub struct BitStream<'a> {
    pub buf: &'a [u8],
    pub pos: i32,
    pub limit: i32,
}

fn get_bits(bs: &mut BitStream, n: i32) -> u32 {
    let s = (bs.pos & 7) as u32;
    let mut shl = n + (s as i32);
    let byte_pos = (bs.pos >> 3) as usize;

    bs.pos += n;
    if bs.pos > bs.limit {
        return 0;
    }

    let mut next = (bs.buf[byte_pos] as u32) & (255u32 >> s);
    let mut cache = 0u32;
    let mut byte_offset = byte_pos + 1;

    while shl > 8 {
        shl -= 8;
        cache |= next << shl;
        next = bs.buf[byte_offset] as u32;
        byte_offset += 1;
    }

    cache | if shl > 0 { next << (shl as u32) } else { next >> (((-shl) as u32)) }
}

pub fn read_scalefactors(bs: &mut BitStream, pba: &[u8], scfcod: &[u8], scf: &mut [f32]) {
    const G_DEQ_L12: [f32; 54] = [
        9.53674316e-07 / 3.0,     7.56931807e-07 / 3.0,     6.00777173e-07 / 3.0,
        9.53674316e-07 / 7.0,     7.56931807e-07 / 7.0,     6.00777173e-07 / 7.0,
        9.53674316e-07 / 15.0,    7.56931807e-07 / 15.0,    6.00777173e-07 / 15.0,
        9.53674316e-07 / 31.0,    7.56931807e-07 / 31.0,    6.00777173e-07 / 31.0,
        9.53674316e-07 / 63.0,    7.56931807e-07 / 63.0,    6.00777173e-07 / 63.0,
        9.53674316e-07 / 127.0,   7.56931807e-07 / 127.0,   6.00777173e-07 / 127.0,
        9.53674316e-07 / 255.0,   7.56931807e-07 / 255.0,   6.00777173e-07 / 255.0,
        9.53674316e-07 / 511.0,   7.56931807e-07 / 511.0,   6.00777173e-07 / 511.0,
        9.53674316e-07 / 1023.0,  7.56931807e-07 / 1023.0,  6.00777173e-07 / 1023.0,
        9.53674316e-07 / 2047.0,  7.56931807e-07 / 2047.0,  6.00777173e-07 / 2047.0,
        9.53674316e-07 / 4095.0,  7.56931807e-07 / 4095.0,  6.00777173e-07 / 4095.0,
        9.53674316e-07 / 8191.0,  7.56931807e-07 / 8191.0,  6.00777173e-07 / 8191.0,
        9.53674316e-07 / 16383.0, 7.56931807e-07 / 16383.0, 6.00777173e-07 / 16383.0,
        9.53674316e-07 / 32767.0, 7.56931807e-07 / 32767.0, 6.00777173e-07 / 32767.0,
        9.53674316e-07 / 65535.0, 7.56931807e-07 / 65535.0, 6.00777173e-07 / 65535.0,
        9.53674316e-07 / 3.0,     7.56931807e-07 / 3.0,     6.00777173e-07 / 3.0,
        9.53674316e-07 / 5.0,     7.56931807e-07 / 5.0,     6.00777173e-07 / 5.0,
        9.53674316e-07 / 9.0,     7.56931807e-07 / 9.0,     6.00777173e-07 / 9.0,
    ];

    let mut scf_idx = 0;
    for i in 0..pba.len() {
        let mut s = 0.0f32;
        let ba = pba[i] as usize;
        let mask = if ba != 0 { 4 + ((19 >> scfcod[i]) & 3) } else { 0 };

        let mut m = 4;
        loop {
            if (mask as i32) & m != 0 {
                let b = get_bits(bs, 6) as usize;
                let shift_val = (1u32 << 21) >> ((b / 3) as u32);
                s = G_DEQ_L12[ba * 3 - 6 + b % 3] * (shift_val as f32);
            }
            scf[scf_idx] = s;
            scf_idx += 1;
            m >>= 1;
            if m == 0 {
                break;
            }
        }
    }
}
