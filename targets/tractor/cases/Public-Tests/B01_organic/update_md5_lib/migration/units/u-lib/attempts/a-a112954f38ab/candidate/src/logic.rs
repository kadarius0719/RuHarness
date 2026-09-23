#[repr(C)]
pub struct tflac_md5 {
    pub pos: u32,
    pub total: u64,
    pub buffer: [u8; 72],
}

#[repr(C)]
pub struct tflac {
    pub md5_ctx: tflac_md5,
    pub cur_blocksize: u32,
    pub channels: u32,
}

pub fn tflac_pack_u64le(d: &mut [u8], n: u64) {
    d[0] = (n as u8);
    d[1] = ((n >> 8) as u8);
    d[2] = ((n >> 16) as u8);
    d[3] = ((n >> 24) as u8);
    d[4] = ((n >> 32) as u8);
    d[5] = ((n >> 40) as u8);
    d[6] = ((n >> 48) as u8);
    d[7] = ((n >> 56) as u8);
}

pub fn tflac_md5_addsample(m: &mut tflac_md5, bits: u32, val: u64) {
    m.total = m.total.wrapping_add(bits as u64);
    let bytes = bits / 8;
    let pos2 = m.pos % 64;
    let pos2_usize = pos2 as usize;

    if pos2_usize + 8 <= m.buffer.len() {
        tflac_pack_u64le(&mut m.buffer[pos2_usize..pos2_usize + 8], val);
    }

    m.pos = m.pos.wrapping_add(bytes);

    if m.pos >= 64 {
        m.pos %= 64;
        let bytes_to_copy = m.pos as usize;
        if bytes_to_copy > 0 {
            let mut i = bytes_to_copy;
            while i > 0 {
                i = i.wrapping_sub(1);
                m.buffer[i] = m.buffer[64 + i];
            }
        }
    }
}

pub fn update_md5(t: &mut tflac, samples: &[i32]) -> u32 {
    let mut b = t.cur_blocksize.wrapping_mul(t.channels);
    let step = std::mem::size_of::<u64>() as u32;
    let mut offset = 0usize;

    for _ in 0..=4 {
        if offset + 8 > samples.len() {
            break;
        }

        let mut v: u64 = 0;
        v |= ((samples[offset] as u64) & 0xFF) << 0;
        v |= ((samples[offset + 1] as u64) & 0xFF) << 8;
        v |= ((samples[offset + 2] as u64) & 0xFF) << 16;
        v |= ((samples[offset + 3] as u64) & 0xFF) << 24;
        v |= ((samples[offset + 4] as u64) & 0xFF) << 32;
        v |= ((samples[offset + 5] as u64) & 0xFF) << 40;
        v |= ((samples[offset + 6] as u64) & 0xFF) << 48;
        v |= ((samples[offset + 7] as u64) & 0xFF) << 56;

        tflac_md5_addsample(&mut t.md5_ctx, 8u32.wrapping_mul(std::mem::size_of::<u64>() as u32), v);

        b = b.wrapping_sub(step);
        offset = offset.wrapping_add(8);
    }

    b
}
