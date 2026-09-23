#[repr(C)]
pub struct ImaBlock {
    pub preamble: u16,
    pub data: [u8; 32],
}

#[repr(C)]
pub struct ImaInfo {
    pub blocks: *const ImaBlock,
    pub size: u64,
    pub sample_rate: f64,
    pub frame_count: u64,
    pub channel_count: u32,
}

fn ima_bswap16(v: u16) -> u16 {
    ((v << 8) & 0xff00) | ((v >> 8) & 0x00ff)
}

fn ima_bswap32(v: u32) -> u32 {
    ((v << 24) & 0xff000000) | ((v << 8) & 0x00ff0000) |
    ((v >> 8) & 0x0000ff00) | ((v >> 24) & 0x000000ff)
}

fn ima_bswap64(v: u64) -> u64 {
    ((v << 56) & 0xff00000000000000) |
    ((v << 40) & 0x00ff000000000000) |
    ((v << 24) & 0x0000ff0000000000) |
    ((v << 8) & 0x000000ff00000000) |
    ((v >> 8) & 0x00000000ff000000) |
    ((v >> 24) & 0x0000000000ff0000) |
    ((v >> 40) & 0x000000000000ff00) |
    ((v >> 56) & 0x00000000000000ff)
}

fn ima_btoh16(v: u16) -> u16 {
    ima_bswap16(v)
}

fn ima_btoh32(v: u32) -> u32 {
    ima_bswap32(v)
}

fn ima_btoh64(v: u64) -> u64 {
    ima_bswap64(v)
}

pub fn ima_parse_internal(data: &[u8]) -> (i32, u64, f64, u64, u32, usize) {
    if data.len() < 4 {
        return (-1, 0, 0.0, 0, 0, 0);
    }

    let header_type = u32::from_be_bytes([
        data[0], data[1], data[2], data[3],
    ]);
    let header_type = ima_btoh32(header_type);

    let fac_type = ('f' as u32) | (('f' as u32) << 8) | (('a' as u32) << 16) | (('c' as u32) << 24);

    if header_type != fac_type {
        return (-1, 0, 0.0, 0, 0, 0);
    }

    if data.len() < 6 {
        return (-2, 0, 0.0, 0, 0, 0);
    }

    let header_version = u16::from_be_bytes([data[4], data[5]]);
    let header_version = ima_btoh16(header_version);

    if header_version != 1 {
        return (-2, 0, 0.0, 0, 0, 0);
    }

    let mut chunk_offset = 8;
    let mut desc_sample_rate = 0.0f64;
    let mut desc_channels = 0u32;
    let mut pakt_frame_count = 0i64;
    let mut blocks_offset = 0usize;
    let mut chunk_size: i64 = 0;
    let mut found_atad = false;

    loop {
        if chunk_offset + 12 > data.len() {
            break;
        }

        let chunk_type = u32::from_be_bytes([
            data[chunk_offset],
            data[chunk_offset + 1],
            data[chunk_offset + 2],
            data[chunk_offset + 3],
        ]);
        let chunk_type = ima_btoh32(chunk_type);

        let chunk_size_raw = u64::from_be_bytes([
            data[chunk_offset + 4],
            data[chunk_offset + 5],
            data[chunk_offset + 6],
            data[chunk_offset + 7],
            data[chunk_offset + 8],
            data[chunk_offset + 9],
            data[chunk_offset + 10],
            data[chunk_offset + 11],
        ]);
        chunk_size = ima_btoh64(chunk_size_raw) as i64;

        let csd_type = ('c' as u32) | (('s' as u32) << 8) | (('e' as u32) << 16) | (('d' as u32) << 24);
        let pakt_type = ('t' as u32) | (('k' as u32) << 8) | (('a' as u32) << 16) | (('p' as u32) << 24);
        let atad_type = ('a' as u32) | (('t' as u32) << 8) | (('a' as u32) << 16) | (('d' as u32) << 24);

        if chunk_type == csd_type {
            if chunk_offset + 12 + 32 <= data.len() {
                let sample_rate_bits = u64::from_be_bytes([
                    data[chunk_offset + 12],
                    data[chunk_offset + 13],
                    data[chunk_offset + 14],
                    data[chunk_offset + 15],
                    data[chunk_offset + 16],
                    data[chunk_offset + 17],
                    data[chunk_offset + 18],
                    data[chunk_offset + 19],
                ]);
                desc_sample_rate = f64::from_bits(ima_btoh64(sample_rate_bits));

                desc_channels = u32::from_be_bytes([
                    data[chunk_offset + 32],
                    data[chunk_offset + 33],
                    data[chunk_offset + 34],
                    data[chunk_offset + 35],
                ]);
                desc_channels = ima_btoh32(desc_channels);
            }
        } else if chunk_type == pakt_type {
            if chunk_offset + 12 + 16 <= data.len() {
                let frame_count_raw = u64::from_be_bytes([
                    data[chunk_offset + 20],
                    data[chunk_offset + 21],
                    data[chunk_offset + 22],
                    data[chunk_offset + 23],
                    data[chunk_offset + 24],
                    data[chunk_offset + 25],
                    data[chunk_offset + 26],
                    data[chunk_offset + 27],
                ]);
                pakt_frame_count = ima_btoh64(frame_count_raw) as i64;
            }
        } else if chunk_type == atad_type {
            blocks_offset = chunk_offset + 12 + 4;
            found_atad = true;
            break;
        }

        chunk_offset += 12 + chunk_size as usize;
    }

    if !found_atad {
        return (-1, 0, 0.0, 0, 0, 0);
    }

    let ima4_type = ('4' as u32) | (('a' as u32) << 8) | (('m' as u32) << 16) | (('i' as u32) << 24);

    if blocks_offset + 4 > data.len() {
        return (-3, 0, 0.0, 0, 0, 0);
    }

    let format_id = u32::from_be_bytes([
        data[blocks_offset - 4],
        data[blocks_offset - 3],
        data[blocks_offset - 2],
        data[blocks_offset - 1],
    ]);
    let format_id = ima_btoh32(format_id);

    if format_id != ima4_type {
        return (-3, 0, 0.0, 0, 0, 0);
    }

    (0, chunk_size as u64, desc_sample_rate, pakt_frame_count as u64, desc_channels, blocks_offset)
}
