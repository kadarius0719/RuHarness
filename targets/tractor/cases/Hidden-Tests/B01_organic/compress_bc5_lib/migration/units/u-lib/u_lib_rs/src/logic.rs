pub fn compress_alpha_block(dest: &mut [u8], src: &[u8], stride: usize) {
    let mut mn = src[0] as i32;
    let mut mx = src[0] as i32;

    for i in 1..16 {
        let val = src[i * stride] as i32;
        if val < mn {
            mn = val;
        } else if val > mx {
            mx = val;
        }
    }

    dest[0] = mx as u8;
    dest[1] = mn as u8;

    let mut dest_offset = 2;
    let dist = mx - mn;
    let dist4 = dist * 4;
    let dist2 = dist * 2;

    let bias = if dist < 8 {
        dist - 1
    } else {
        dist / 2 + 2
    };
    let bias = bias - mn * 7;

    let mut bits = 0;
    let mut mask = 0i32;

    for i in 0..16 {
        let mut a = (src[i * stride] as i32) * 7 + bias;

        let t = if a >= dist4 { -1i32 } else { 0i32 };
        let mut ind = t & 4;
        a = a - (dist4 & t);

        let t = if a >= dist2 { -1i32 } else { 0i32 };
        ind = ind + (t & 2);
        a = a - (dist2 & t);

        ind = ind + (if a >= dist { 1 } else { 0 });
        ind = -ind & 7;
        ind = ind ^ (if 2 > ind { 1 } else { 0 });

        mask = mask | (ind << bits);
        bits += 3;
        if bits >= 8 {
            dest[dest_offset] = (mask & 0xFF) as u8;
            dest_offset += 1;
            mask = mask >> 8;
            bits -= 8;
        }
    }
}

pub fn compress_bc5(dest: &mut [u8], src: &[u8]) {
    compress_alpha_block(dest, src, 2);
    compress_alpha_block(&mut dest[8..], &src[1..], 2);
}
