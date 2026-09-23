pub struct BitStream<'a> {
    pub buf: &'a [u8],
    pub pos: i32,
    pub limit: i32,
}

const G_DEQ_L12: [f32; 54] = [
    9.53674316e-07f32 / 3.0,
    7.56931807e-07f32 / 3.0,
    6.00777173e-07f32 / 3.0,
    9.53674316e-07f32 / 7.0,
    7.56931807e-07f32 / 7.0,
    6.00777173e-07f32 / 7.0,
    9.53674316e-07f32 / 15.0,
    7.56931807e-07f32 / 15.0,
    6.00777173e-07f32 / 15.0,
    9.53674316e-07f32 / 31.0,
    7.56931807e-07f32 / 31.0,
    6.00777173e-07f32 / 31.0,
    9.53674316e-07f32 / 63.0,
    7.56931807e-07f32 / 63.0,
    6.00777173e-07f32 / 63.0,
    9.53674316e-07f32 / 127.0,
    7.56931807e-07f32 / 127.0,
    6.00777173e-07f32 / 127.0,
    9.53674316e-07f32 / 255.0,
    7.56931807e-07f32 / 255.0,
    6.00777173e-07f32 / 255.0,
    9.53674316e-07f32 / 511.0,
    7.56931807e-07f32 / 511.0,
    6.00777173e-07f32 / 511.0,
    9.53674316e-07f32 / 1023.0,
    7.56931807e-07f32 / 1023.0,
    6.00777173e-07f32 / 1023.0,
    9.53674316e-07f32 / 2047.0,
    7.56931807e-07f32 / 2047.0,
    6.00777173e-07f32 / 2047.0,
    9.53674316e-07f32 / 4095.0,
    7.56931807e-07f32 / 4095.0,
    6.00777173e-07f32 / 4095.0,
    9.53674316e-07f32 / 8191.0,
    7.56931807e-07f32 / 8191.0,
    6.00777173e-07f32 / 8191.0,
    9.53674316e-07f32 / 16383.0,
    7.56931807e-07f32 / 16383.0,
    6.00777173e-07f32 / 16383.0,
    9.53674316e-07f32 / 32767.0,
    7.56931807e-07f32 / 32767.0,
    6.00777173e-07f32 / 32767.0,
    9.53674316e-07f32 / 65535.0,
    7.56931807e-07f32 / 65535.0,
    6.00777173e-07f32 / 65535.0,
    9.53674316e-07f32 / 3.0,
    7.56931807e-07f32 / 3.0,
    6.00777173e-07f32 / 3.0,
    9.53674316e-07f32 / 5.0,
    7.56931807e-07f32 / 5.0,
    6.00777173e-07f32 / 5.0,
    9.53674316e-07f32 / 9.0,
    7.56931807e-07f32 / 9.0,
    6.00777173e-07f32 / 9.0,
];

fn read_byte_at(buf: &[u8], idx: i64) -> u32 {
    if idx < 0 {
        0
    } else {
        match buf.get(idx as usize) {
            Some(v) => *v as u32,
            None => 0,
        }
    }
}

fn get_bits(bs: &mut BitStream<'_>, n: i32) -> u32 {
    let s: u32 = (bs.pos & 7) as u32;
    let mut shl: i32 = n + s as i32;
    let byte_pos: i64 = (bs.pos >> 3) as i64;
    bs.pos = bs.pos.wrapping_add(n);
    if bs.pos > bs.limit {
        return 0;
    }

    let mut idx: i64 = byte_pos;
    let mut next: u32 = read_byte_at(bs.buf, idx) & 255u32.wrapping_shr(s);
    idx = idx.wrapping_add(1);

    let mut cache: u32 = 0;
    loop {
        shl -= 8;
        if shl <= 0 {
            break;
        }
        cache |= next.wrapping_shl(shl as u32);
        next = read_byte_at(bs.buf, idx);
        idx = idx.wrapping_add(1);
    }

    cache | next.wrapping_shr((-shl) as u32)
}

pub fn read_scalefactors(
    bs: &mut BitStream<'_>,
    pba: &[u8],
    scfcod: &[u8],
    bands: i32,
    scf: &mut [f32],
) {
    let mut pba_idx: usize = 0;
    let mut scf_idx: usize = 0;
    let mut i: i32 = 0;

    while i < bands {
        let mut s: f32 = 0.0;

        let ba: i32 = if pba_idx < pba.len() {
            pba[pba_idx] as i32
        } else {
            0
        };
        pba_idx = pba_idx.wrapping_add(1);

        let scfcod_val: u8 = if (i as usize) < scfcod.len() {
            scfcod[i as usize]
        } else {
            0
        };

        let mask: i32 = if ba != 0 {
            4 + (19i32.wrapping_shr(scfcod_val as u32) & 3)
        } else {
            0
        };

        let mut m: i32 = 4;
        while m != 0 {
            if mask & m != 0 {
                let b: i32 = get_bits(bs, 6) as i32;
                let idx: i32 = ba.wrapping_mul(3).wrapping_sub(6).wrapping_add(b % 3);
                let table_val: f32 = if idx >= 0 && (idx as usize) < G_DEQ_L12.len() {
                    G_DEQ_L12[idx as usize]
                } else {
                    0.0
                };
                let factor: i32 = 1i32.wrapping_shl(21).wrapping_shr((b / 3) as u32);
                s = table_val * (factor as f32);
            }
            if scf_idx < scf.len() {
                scf[scf_idx] = s;
            }
            scf_idx = scf_idx.wrapping_add(1);
            m >>= 1;
        }

        i = i.wrapping_add(1);
    }
}
