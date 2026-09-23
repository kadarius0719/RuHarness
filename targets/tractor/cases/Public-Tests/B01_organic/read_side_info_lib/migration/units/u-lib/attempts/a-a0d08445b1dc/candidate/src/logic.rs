#[repr(C)]
pub struct BsT {
    pub buf: *const u8,
    pub pos: i32,
    pub limit: i32,
}

#[repr(C)]
pub struct L3GrInfoT {
    pub sfbtab: *const u8,
    pub part_23_length: u16,
    pub big_values: u16,
    pub scalefac_compress: u16,
    pub global_gain: u8,
    pub block_type: u8,
    pub mixed_block_flag: u8,
    pub n_long_sfb: u8,
    pub n_short_sfb: u8,
    pub table_select: [u8; 3],
    pub region_count: [u8; 3],
    pub subblock_gain: [u8; 3],
    pub preflag: u8,
    pub scalefac_scale: u8,
    pub count1_table: u8,
    pub scfsi: u8,
}

const G_SCF_LONG: [[u8; 23]; 8] = [
    [6,  6,  6,  6,  6,  6,  8,  10, 12, 14, 16, 20,
     24, 28, 32, 38, 46, 52, 60, 68, 58, 54, 0],
    [12, 12, 12, 12, 12, 12, 16, 20, 24, 28, 32, 40,
     48, 56, 64, 76, 90, 2,  2,  2,  2,  2,  0],
    [6,  6,  6,  6,  6,  6,  8,  10, 12, 14, 16, 20,
     24, 28, 32, 38, 46, 52, 60, 68, 58, 54, 0],
    [6,  6,  6,  6,  6,  6,  8,  10, 12, 14, 16, 18,
     22, 26, 32, 38, 46, 54, 62, 70, 76, 36, 0],
    [6,  6,  6,  6,  6,  6,  8,  10, 12, 14, 16, 20,
     24, 28, 32, 38, 46, 52, 60, 68, 58, 54, 0],
    [4,  4,  4,  4,  4,  4,  6,  6,  8,  8,   10, 12,
     16, 20, 24, 28, 34, 42, 50, 54, 76, 158, 0],
    [4,  4,  4,  4,  4,  4,  6,  6,  6,  8,   10, 12,
     16, 18, 22, 28, 34, 40, 46, 54, 54, 192, 0],
    [4,  4,  4,  4,  4,  4,  6,  6,  8,   10, 12, 16,
     20, 24, 30, 38, 46, 56, 68, 84, 102, 26, 0],
];

const G_SCF_SHORT: [[u8; 40]; 8] = [
    [4,  4,  4,  4,  4,  4,  4,  4,  4,  6,  6,  6,  8,  8,
     8,  10, 10, 10, 12, 12, 12, 14, 14, 14, 18, 18, 18, 24,
     24, 24, 30, 30, 30, 40, 40, 40, 18, 18, 18, 0],
    [8,  8,  8,  8,  8,  8,  8,  8,  8,  12, 12, 12, 16, 16,
     16, 20, 20, 20, 24, 24, 24, 28, 28, 28, 36, 36, 36, 2,
     2,  2,  2,  2,  2,  2,  2,  2,  26, 26, 26, 0],
    [4,  4,  4,  4,  4,  4,  4,  4,  4,  6,  6,  6,  6,  6,
     6,  8,  8,  8,  10, 10, 10, 14, 14, 14, 18, 18, 18, 26,
     26, 26, 32, 32, 32, 42, 42, 42, 18, 18, 18, 0],
    [4,  4,  4,  4,  4,  4,  4,  4,  4,  6,  6,  6,  8,  8,
     8,  10, 10, 10, 12, 12, 12, 14, 14, 14, 18, 18, 18, 24,
     24, 24, 32, 32, 32, 44, 44, 44, 12, 12, 12, 0],
    [4,  4,  4,  4,  4,  4,  4,  4,  4,  6,  6,  6,  8,  8,
     8,  10, 10, 10, 12, 12, 12, 14, 14, 14, 18, 18, 18, 24,
     24, 24, 30, 30, 30, 40, 40, 40, 18, 18, 18, 0],
    [4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  6,  6,
     6,  8,  8,  8,  10, 10, 10, 12, 12, 12, 14, 14, 14, 18,
     18, 18, 22, 22, 22, 30, 30, 30, 56, 56, 56, 0],
    [4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  6,  6,
     6,  6,  6,  6,  10, 10, 10, 12, 12, 12, 14, 14, 14, 16,
     16, 16, 20, 20, 20, 26, 26, 26, 66, 66, 66, 0],
    [4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  6,  6,
     6,  8,  8,  8,  12, 12, 12, 16, 16, 16, 20, 20, 20, 26,
     26, 26, 34, 34, 34, 42, 42, 42, 12, 12, 12, 0],
];

const G_SCF_MIXED: [[u8; 40]; 8] = [
    [6,  6,  6,  6,  6,  6,  6,  6,  6,  8,  8,  8,  10,
     10, 10, 12, 12, 12, 14, 14, 14, 18, 18, 18, 24, 24,
     24, 30, 30, 30, 40, 40, 40, 18, 18, 18, 0, 0, 0, 0],
    [12, 12, 12, 4,  4,  4,  8,  8,  8,  12, 12, 12, 16, 16,
     16, 20, 20, 20, 24, 24, 24, 28, 28, 28, 36, 36, 36, 2,
     2,  2,  2,  2,  2,  2,  2,  2,  26, 26, 26, 0],
    [6,  6,  6,  6,  6,  6,  6,  6,  6,  6,  6,  6,  8,
     8,  8,  10, 10, 10, 14, 14, 14, 18, 18, 18, 26, 26,
     26, 32, 32, 32, 42, 42, 42, 18, 18, 18, 0, 0, 0, 0],
    [6,  6,  6,  6,  6,  6,  6,  6,  6,  8,  8,  8,  10,
     10, 10, 12, 12, 12, 14, 14, 14, 18, 18, 18, 24, 24,
     24, 32, 32, 32, 44, 44, 44, 12, 12, 12, 0, 0, 0, 0],
    [6,  6,  6,  6,  6,  6,  6,  6,  6,  8,  8,  8,  10,
     10, 10, 12, 12, 12, 14, 14, 14, 18, 18, 18, 24, 24,
     24, 30, 30, 30, 40, 40, 40, 18, 18, 18, 0, 0, 0, 0],
    [4,  4,  4,  4,  4,  4,  6,  6,  4,  4,  4,  6,  6,
     6,  8,  8,  8,  10, 10, 10, 12, 12, 12, 14, 14, 14,
     18, 18, 18, 22, 22, 22, 30, 30, 30, 56, 56, 56, 0, 0],
    [4,  4,  4,  4,  4,  4,  6,  6,  4,  4,  4,  6,  6,
     6,  6,  6,  6,  10, 10, 10, 12, 12, 12, 14, 14, 14,
     16, 16, 16, 20, 20, 20, 26, 26, 26, 66, 66, 66, 0, 0],
    [4,  4,  4,  4,  4,  4,  6,  6,  4,  4,  4,  6,  6,
     6,  8,  8,  8,  12, 12, 12, 16, 16, 16, 20, 20, 20,
     26, 26, 26, 34, 34, 34, 42, 42, 42, 12, 12, 12, 0, 0],
];

pub fn get_bits(buf: &[u8], pos: &mut i32, limit: i32, n: i32) -> u32 {
    let s = (*pos & 7) as u32;
    let shl = n + (*pos & 7);

    let byte_pos = (*pos >> 3) as usize;
    *pos += n;

    if *pos > limit {
        return 0;
    }

    if byte_pos >= buf.len() {
        return 0;
    }

    let mut next: u32 = ((buf[byte_pos] as u32) & (255 >> s)) as u32;
    let mut cache: u32 = 0;
    let mut shl_mut = shl;
    let mut byte_idx = byte_pos + 1;

    while shl_mut - 8 > 0 {
        cache |= next << (shl_mut - 8);
        shl_mut -= 8;
        if byte_idx >= buf.len() {
            return 0;
        }
        next = buf[byte_idx] as u32;
        byte_idx += 1;
    }

    cache | (next >> (8 - shl_mut))
}

pub fn read_side_info(buf: &[u8], pos: &mut i32, limit: i32, gr_slice: &mut [L3GrInfoT], hdr: &[u8]) -> i32 {
    if hdr.len() < 4 {
        return -1;
    }

    let sr_idx = ((((hdr[2]) >> 2) & 3) as usize +
                  (((hdr[1] >> 3) & 1) + ((hdr[1] >> 4) & 1)) as usize * 3) as usize;
    let sr_idx = if sr_idx != 0 { sr_idx - 1 } else { 0 };

    let mut gr_count = if ((hdr[3]) & 0xC0) == 0xC0 { 1 } else { 2 };

    let mut scfsi: u32 = 0;
    let main_data_begin: i32;

    if ((hdr[1]) & 0x8) != 0 {
        gr_count *= 2;
        main_data_begin = get_bits(buf, pos, limit, 9) as i32;
        scfsi = get_bits(buf, pos, limit, 7 + gr_count as i32);
    } else {
        let val = get_bits(buf, pos, limit, 8 + gr_count as i32);
        main_data_begin = (val >> gr_count) as i32;
    }

    let mut part_23_sum = 0;

    for gr_idx in 0..gr_count.min(gr_slice.len()) {
        let gr = &mut gr_slice[gr_idx];

        if ((hdr[3]) & 0xC0) == 0xC0 {
            scfsi <<= 4;
        }

        gr.part_23_length = get_bits(buf, pos, limit, 12) as u16;
        part_23_sum += gr.part_23_length as i32;
        gr.big_values = get_bits(buf, pos, limit, 9) as u16;
        if gr.big_values > 288 {
            return -1;
        }

        gr.global_gain = get_bits(buf, pos, limit, 8) as u8;
        gr.scalefac_compress = get_bits(buf, pos, limit, if ((hdr[1]) & 0x8) != 0 { 4 } else { 9 }) as u16;

        gr.sfbtab = if sr_idx < G_SCF_LONG.len() {
            &G_SCF_LONG[sr_idx][0] as *const u8
        } else {
            &G_SCF_LONG[0][0] as *const u8
        };
        gr.n_long_sfb = 22;
        gr.n_short_sfb = 0;

        if get_bits(buf, pos, limit, 1) != 0 {
            gr.block_type = get_bits(buf, pos, limit, 2) as u8;
            if gr.block_type == 0 {
                return -1;
            }
            gr.mixed_block_flag = get_bits(buf, pos, limit, 1) as u8;
            gr.region_count[0] = 7;
            gr.region_count[1] = 255;

            if gr.block_type == 2 {
                scfsi &= 0x0F0F;
                if gr.mixed_block_flag == 0 {
                    gr.region_count[0] = 8;
                    gr.sfbtab = if sr_idx < G_SCF_SHORT.len() {
                        &G_SCF_SHORT[sr_idx][0] as *const u8
                    } else {
                        &G_SCF_SHORT[0][0] as *const u8
                    };
                    gr.n_long_sfb = 0;
                    gr.n_short_sfb = 39;
                } else {
                    gr.sfbtab = if sr_idx < G_SCF_MIXED.len() {
                        &G_SCF_MIXED[sr_idx][0] as *const u8
                    } else {
                        &G_SCF_MIXED[0][0] as *const u8
                    };
                    gr.n_long_sfb = if ((hdr[1]) & 0x8) != 0 { 8 } else { 6 };
                    gr.n_short_sfb = 30;
                }
            }

            let mut tables = get_bits(buf, pos, limit, 10);
            tables <<= 5;
            gr.subblock_gain[0] = get_bits(buf, pos, limit, 3) as u8;
            gr.subblock_gain[1] = get_bits(buf, pos, limit, 3) as u8;
            gr.subblock_gain[2] = get_bits(buf, pos, limit, 3) as u8;

            gr.table_select[0] = (tables >> 10) as u8;
            gr.table_select[1] = ((tables >> 5) & 31) as u8;
            gr.table_select[2] = (tables & 31) as u8;
        } else {
            gr.block_type = 0;
            gr.mixed_block_flag = 0;
            let tables = get_bits(buf, pos, limit, 15);
            gr.region_count[0] = get_bits(buf, pos, limit, 4) as u8;
            gr.region_count[1] = get_bits(buf, pos, limit, 3) as u8;
            gr.region_count[2] = 255;

            gr.table_select[0] = (tables >> 10) as u8;
            gr.table_select[1] = ((tables >> 5) & 31) as u8;
            gr.table_select[2] = (tables & 31) as u8;
        }

        gr.preflag = if ((hdr[1]) & 0x8) != 0 {
            get_bits(buf, pos, limit, 1) as u8
        } else {
            if gr.scalefac_compress >= 500 { 1 } else { 0 }
        };
        gr.scalefac_scale = get_bits(buf, pos, limit, 1) as u8;
        gr.count1_table = get_bits(buf, pos, limit, 1) as u8;
        gr.scfsi = ((scfsi >> 12) & 15) as u8;
        scfsi <<= 4;
    }

    if part_23_sum + *pos > limit + main_data_begin * 8 {
        return -1;
    }

    main_data_begin
}
