fn fakesqrt(val: i64) -> i64 {
    if val < 0 {
        return -fakesqrt(-val);
    }

    let mut v = val;
    while v > (1i64 << 30) {
        v >>= 1;
    }

    while v.wrapping_mul(v) > val {
        v >>= 1;
    }

    for i in 1..17 {
        let mut v1 = v + (v >> i);
        let mut j = 16;

        if v1 == v {
            break;
        }

        while v1.wrapping_mul(v1) <= val && j > 0 {
            v = v1;
            v1 = v + (v >> i);
            j -= 1;
        }
    }

    v
}

pub fn stereo_samples(ibuf0: &[i16], ibuf1: &[i16], len: i32) -> i32 {
    let len_mod = len % 16;
    let len = if len_mod < 0 { 0 } else { len_mod as usize };
    let mut e: i64 = 0;

    for i in 0..len {
        let p0 = ibuf0[i * 2 + 0] as i64;
        let p1 = ibuf0[i * 2 + 1] as i64;
        let p2 = ibuf1[i * 2 + 0] as i64;
        let p3 = ibuf1[i * 2 + 1] as i64;

        let pc0 = (p0 + p1) >> 1;
        let ps0 = p0 - p1;
        let pc1 = (p2 + p3) >> 1;
        let ps1 = p2 - p3;

        let dc = pc0 - pc1;
        let mut ds = ps0 - ps1;
        ds >>= 2;

        let d = dc.wrapping_mul(dc).wrapping_add(ds.wrapping_mul(ds));
        e = e.wrapping_add(d);
    }

    e = fakesqrt(e);
    if e < 0 || e > (1i64 << 30) {
        e = 1i64 << 30;
    }
    e as i32
}
