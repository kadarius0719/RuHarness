fn hdr_valid(h: &[u8]) -> i32 {
    if h.len() < 3 {
        return 0;
    }

    let h0 = h[0];
    let h1 = h[1];
    let h2 = h[2];

    let cond1 = h0 == 0xff;
    let cond2 = ((h1 & 0xF0) == 0xf0) || ((h1 & 0xFE) == 0xe2);
    let cond3 = ((h1 >> 1) & 3) != 0;
    let cond4 = (h2 >> 4) != 15;
    let cond5 = ((h2 >> 2) & 3) != 3;

    if cond1 && cond2 && cond3 && cond4 && cond5 {
        1
    } else {
        0
    }
}

pub fn hdr_compare(h1: &[u8], h2: &[u8]) -> i32 {
    if h1.len() < 3 || h2.len() < 3 {
        return 0;
    }

    let h1_1 = h1[1];
    let h1_2 = h1[2];
    let h2_1 = h2[1];
    let h2_2 = h2[2];

    let cond1 = hdr_valid(h2) != 0;
    let cond2 = ((h1_1 ^ h2_1) & 0xFE) == 0;
    let cond3 = ((h1_2 ^ h2_2) & 0x0C) == 0;
    let cond4 = !((((h1_2) & 0xF0) == 0) ^ (((h2_2) & 0xF0) == 0));

    if cond1 && cond2 && cond3 && cond4 {
        1
    } else {
        0
    }
}
