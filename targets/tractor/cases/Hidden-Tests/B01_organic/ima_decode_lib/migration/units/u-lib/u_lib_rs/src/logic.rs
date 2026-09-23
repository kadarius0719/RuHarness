#[repr(C)]
pub struct ImaChannelState {
    pub index: i32,
    pub predict: i32,
}

#[repr(C)]
pub struct ImaBlock {
    pub preamble: u16,
    pub data: [u8; 32],
}

fn ima_bswap16(v: u16) -> u16 {
    ((v << 8) & 0xff00) | ((v >> 8) & 0x00ff)
}

fn ima_btoh16(v: u16) -> u16 {
    ima_bswap16(v)
}

const IMA_INDEX_TABLE: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

const IMA_STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60,
    66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371,
    408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878,
    2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845,
    8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086,
    29794, 32767,
];

fn ima_clamp_index(index: i32) -> i32 {
    if index < 0 {
        0
    } else if index > 88 {
        88
    } else {
        index
    }
}

fn ima_clamp_predict(predict: i32) -> i32 {
    if predict < -32768 {
        -32768
    } else if predict > 32767 {
        32767
    } else {
        predict
    }
}

pub fn ima_decode(output: &mut [f32], channel_count: u32, block: &ImaBlock, decode_count: u64, state: &mut ImaChannelState) {
    let mut index = (ima_btoh16(block.preamble) as i32) & 0x7f;
    let mut predict = ((ima_btoh16(block.preamble) as i16) as i32) & !0x7f;

    if index == state.index {
        let diff = if predict - state.predict < 0 { -(predict - state.predict) } else { predict - state.predict };
        if diff <= 0x7f {
            predict = state.predict;
        }
    }

    let mut step = IMA_STEP_TABLE[index as usize];

    for i in 0..(decode_count >> 1) {
        let nibble = (block.data[i as usize] & 0xf) as i32;
        index = ima_clamp_index(index + IMA_INDEX_TABLE[nibble as usize]);
        let mut diff = step >> 3;
        if nibble & 4 != 0 {
            diff += step;
        }
        if nibble & 2 != 0 {
            diff += step >> 1;
        }
        if nibble & 1 != 0 {
            diff += step >> 2;
        }
        if nibble & 8 != 0 {
            predict -= diff;
        } else {
            predict += diff;
        }
        step = IMA_STEP_TABLE[index as usize];
        predict = ima_clamp_predict(predict);
        output[0] += (predict as f32) * 0.0000305185f32;
        output[0] += channel_count as f32;

        let nibble = ((block.data[i as usize] >> 4) & 0xf) as i32;
        index = ima_clamp_index(index + IMA_INDEX_TABLE[nibble as usize]);
        diff = step >> 3;
        if nibble & 4 != 0 {
            diff += step;
        }
        if nibble & 2 != 0 {
            diff += step >> 1;
        }
        if nibble & 1 != 0 {
            diff += step >> 2;
        }
        if nibble & 8 != 0 {
            predict -= diff;
        } else {
            predict += diff;
        }
        step = IMA_STEP_TABLE[index as usize];
        predict = ima_clamp_predict(predict);
        output[0] += (predict as f32) * 0.0000305185f32;
        output[0] += channel_count as f32;
    }

    if (decode_count & 1) != 0 {
        let nibble = (block.data[(decode_count >> 1) as usize] & 0xf) as i32;
        index = ima_clamp_index(index + IMA_INDEX_TABLE[nibble as usize]);
        let mut diff = step >> 3;
        if nibble & 4 != 0 {
            diff += step;
        }
        if nibble & 2 != 0 {
            diff += step >> 1;
        }
        if nibble & 1 != 0 {
            diff += step >> 2;
        }
        if nibble & 8 != 0 {
            predict -= diff;
        } else {
            predict += diff;
        }
        step = IMA_STEP_TABLE[index as usize];
        predict = ima_clamp_predict(predict);
        output[0] += (predict as f32) * 0.0000305185f32;
        output[0] += channel_count as f32;
    }

    state.index = index;
    state.predict = predict;
}
