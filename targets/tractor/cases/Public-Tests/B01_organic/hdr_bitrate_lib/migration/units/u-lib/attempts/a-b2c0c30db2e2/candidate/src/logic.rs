pub fn hdr_bitrate(h: &[u8]) -> u32 {
    const HALFRATE: &[[[u8; 15]; 3]; 2] = &[
        [
            [0, 4, 8, 12, 16, 20, 24, 28, 32, 40, 48, 56, 64, 72, 80],
            [0, 4, 8, 12, 16, 20, 24, 28, 32, 40, 48, 56, 64, 72, 80],
            [0, 16, 24, 28, 32, 40, 48, 56, 64, 72, 80, 88, 96, 112, 128],
        ],
        [
            [0, 16, 20, 24, 28, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160],
            [0, 16, 24, 28, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192],
            [0, 16, 32, 48, 64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224],
        ],
    ];

    if h.len() < 3 {
        return 0;
    }

    let bit_0 = if (h[1] & 0x8) != 0 { 1 } else { 0 };
    let bits_1_2 = ((h[1] >> 1) & 3) as usize;
    let bits_4_7 = (h[2] >> 4) as usize;

    let channel_idx = if bits_1_2 >= 1 { bits_1_2 - 1 } else { 0 };

    let rate = if bit_0 == 0 && bits_1_2 >= 1 && channel_idx < 3 && bits_4_7 < 15 {
        HALFRATE[bit_0][channel_idx][bits_4_7] as u32
    } else if bit_0 == 1 && bits_1_2 >= 1 && channel_idx < 3 && bits_4_7 < 15 {
        HALFRATE[bit_0][channel_idx][bits_4_7] as u32
    } else {
        0
    };

    2 * rate
}
