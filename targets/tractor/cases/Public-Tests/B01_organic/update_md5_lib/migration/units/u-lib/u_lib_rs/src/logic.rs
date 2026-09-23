pub fn pack_u64le(d: &mut [u8], n: u64) {
    d[0] = n as u8;
    d[1] = (n >> 8) as u8;
    d[2] = (n >> 16) as u8;
    d[3] = (n >> 24) as u8;
    d[4] = (n >> 32) as u8;
    d[5] = (n >> 40) as u8;
    d[6] = (n >> 48) as u8;
    d[7] = (n >> 56) as u8;
}

pub fn md5_addsample(pos: &mut u32, total: &mut u64, buffer: &mut [u8], bits: u32, val: u64) {
    *total = total.wrapping_add(bits as u64);
    let bytes = bits / 8;
    let pos2 = (*pos % 64) as usize;
    pack_u64le(&mut buffer[pos2..pos2 + 8], val);
    *pos = pos.wrapping_add(bytes);
    if *pos >= 64 {
        *pos %= 64;
        let mut idx = *pos as usize;
        // Mirrors C's `while (bytes--) { buffer[bytes] = buffer[64 + bytes]; }`:
        // copies buffer[idx-1..0] from buffer[64+idx-1..64]. The buffer's extra
        // 8 trailing bytes only ever need to cover idx in 0..=7 for well-formed
        // callers; the bounds guard avoids a panic without changing behavior
        // for that documented range.
        while idx != 0 {
            idx -= 1;
            if 64 + idx < buffer.len() {
                buffer[idx] = buffer[64 + idx];
            }
        }
    }
}

pub fn update_md5(
    pos: &mut u32,
    total: &mut u64,
    buffer: &mut [u8],
    cur_blocksize: u32,
    channels: u32,
    samples: &[i32],
) -> u32 {
    let mut b: u32 = cur_blocksize.wrapping_mul(channels);
    let step: u32 = 8; // sizeof(tflac_uint) == sizeof(u64)
    let mut offset: usize = 0;

    for _ in 0..5 {
        let mut v: u64 = 0;
        for k in 0..8usize {
            // (tflac_uint)samples[k] & 0xFF is just the low byte of samples[k].
            let byte = samples[offset + k] as u8 as u64;
            v |= byte << (k * 8);
        }
        md5_addsample(pos, total, buffer, 8 * 8, v);
        b = b.wrapping_sub(step);
        // Mirrors the C source's `samples += (8 * sizeof(tflac_s32))`, which
        // is pointer arithmetic in units of tflac_s32 elements: it advances
        // the pointer by 32 elements per iteration, not 8.
        offset += 32;
    }

    b
}
