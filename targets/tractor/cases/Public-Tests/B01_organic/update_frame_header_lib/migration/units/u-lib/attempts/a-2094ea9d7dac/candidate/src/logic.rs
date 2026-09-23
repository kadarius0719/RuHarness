pub fn update_frame_header(t: &mut tflac) {
    t.frame_header = 0xFFF8u32.wrapping_shl(16);

    match t.cur_blocksize {
        192 => t.frame_header |= 0x01u32.wrapping_shl(12),
        576 => t.frame_header |= 0x02u32.wrapping_shl(12),
        1152 => t.frame_header |= 0x03u32.wrapping_shl(12),
        2304 => t.frame_header |= 0x04u32.wrapping_shl(12),
        4608 => t.frame_header |= 0x05u32.wrapping_shl(12),
        256 => t.frame_header |= 0x08u32.wrapping_shl(12),
        512 => t.frame_header |= 0x09u32.wrapping_shl(12),
        1024 => t.frame_header |= 0x0Au32.wrapping_shl(12),
        2048 => t.frame_header |= 0x0Bu32.wrapping_shl(12),
        4096 => t.frame_header |= 0x0Cu32.wrapping_shl(12),
        8192 => t.frame_header |= 0x0Du32.wrapping_shl(12),
        16384 => t.frame_header |= 0x0Eu32.wrapping_shl(12),
        32768 => t.frame_header |= 0x0Fu32.wrapping_shl(12),
        _ => {
            if t.cur_blocksize <= 256 {
                t.frame_header |= 0x06u32.wrapping_shl(12);
            } else {
                t.frame_header |= 0x07u32.wrapping_shl(12);
            }
        }
    }

    match t.samplerate {
        882000 => t.frame_header |= 0x01u32.wrapping_shl(8),
        176400 => t.frame_header |= 0x02u32.wrapping_shl(8),
        192000 => t.frame_header |= 0x03u32.wrapping_shl(8),
        8000 => t.frame_header |= 0x04u32.wrapping_shl(8),
        16000 => t.frame_header |= 0x05u32.wrapping_shl(8),
        22050 => t.frame_header |= 0x06u32.wrapping_shl(8),
        24000 => t.frame_header |= 0x07u32.wrapping_shl(8),
        32000 => t.frame_header |= 0x08u32.wrapping_shl(8),
        44100 => t.frame_header |= 0x09u32.wrapping_shl(8),
        48000 => t.frame_header |= 0x0Au32.wrapping_shl(8),
        96000 => t.frame_header |= 0x0Bu32.wrapping_shl(8),
        _ => {
            if t.samplerate % 1000 == 0 {
                if t.samplerate / 1000 < 256 {
                    t.frame_header |= 0x0Cu32.wrapping_shl(8);
                }
            } else if t.samplerate < 65536 {
                t.frame_header |= 0x0Du32.wrapping_shl(8);
            } else if t.samplerate % 10 == 0 {
                if t.samplerate / 10 < 65536 {
                    t.frame_header |= 0x0Eu32.wrapping_shl(8);
                }
            }
        }
    }

    let mode = (t.channel_mode % 4) as u32;
    match mode {
        0 => t.frame_header |= (t.channels - 1).wrapping_shl(4),
        1 => t.frame_header |= 0x08u32.wrapping_shl(4),
        2 => t.frame_header |= 0x09u32.wrapping_shl(4),
        3 => t.frame_header |= 0x0Au32.wrapping_shl(4),
        _ => {}
    }

    match t.bitdepth {
        8 => t.frame_header |= 1u32.wrapping_shl(1),
        12 => t.frame_header |= 2u32.wrapping_shl(1),
        16 => t.frame_header |= 4u32.wrapping_shl(1),
        20 => t.frame_header |= 5u32.wrapping_shl(1),
        24 => t.frame_header |= 6u32.wrapping_shl(1),
        32 => t.frame_header |= 7u32.wrapping_shl(1),
        _ => {}
    }
}

#[repr(C)]
pub struct tflac {
    pub samplerate: u32,
    pub channels: u32,
    pub bitdepth: u32,
    pub channel_mode: u8,
    pub frame_header: u32,
    pub cur_blocksize: u32,
}
