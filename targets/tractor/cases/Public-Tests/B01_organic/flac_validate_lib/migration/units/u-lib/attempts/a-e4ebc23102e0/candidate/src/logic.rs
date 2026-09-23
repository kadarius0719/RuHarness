#[repr(C)]
pub struct Tflac {
    pub blocksize: u32,
    pub samplerate: u32,
    pub channels: u32,
    pub bitdepth: u32,
    pub channel_mode: u8,
    pub max_rice_value: u8,
    pub min_partition_order: u8,
    pub max_partition_order: u8,
    pub partition_order: u8,
    pub cur_blocksize: u32,
}

pub fn tflac_size_memory(blocksize: u32) -> u32 {
    let val = (15u32).wrapping_add(blocksize.wrapping_mul(4u32)) & 0xFFFFFFF0u32;
    (15u32).wrapping_add((5u32).wrapping_mul(val))
}

pub fn flac_validate(t: &mut Tflac) -> i32 {
    if t.blocksize < 16 {
        return -1;
    }
    if t.blocksize > 65535 {
        return -1;
    }
    if t.samplerate == 0 {
        return -1;
    }
    if t.samplerate > 655350 {
        return -1;
    }
    if t.channels == 0 {
        return -1;
    }
    if t.channels > 8 {
        return -1;
    }
    if t.bitdepth == 0 {
        return -1;
    }
    if t.bitdepth > 32 {
        return -1;
    }
    if t.channel_mode != 0 {
        if t.channels != 2 || t.bitdepth == 32 {
            t.channel_mode = 0;
        }
    }
    if t.max_rice_value == 0 {
        if t.bitdepth <= 16 {
            t.max_rice_value = 14;
        } else {
            t.max_rice_value = 30;
        }
    } else if t.max_rice_value > 30 {
        return -1;
    }
    if t.max_partition_order > 15 {
        return -1;
    }
    if t.min_partition_order > t.max_partition_order {
        return -1;
    }
    t.partition_order = t.min_partition_order;
    while (t.blocksize % (1u32 << (t.partition_order as u32 + 1)) == 0) && (t.partition_order < t.max_partition_order) {
        t.partition_order += 1;
    }
    t.cur_blocksize = t.blocksize;
    0
}
