pub fn hex2bin(
    bin: &mut [u8],
    hex: &[u8],
    ignore: Option<&[u8]>,
) -> (i32, usize, usize) {
    let bin_maxlen = bin.len();
    let hex_len = hex.len();

    let mut bin_pos: usize = 0;
    let mut hex_pos: usize = 0;
    let mut ret: i32 = 0;
    let mut c: u8;
    let mut c_alpha0: u8;
    let mut c_alpha: u8;
    let mut c_num0: u8;
    let mut c_num: u8;
    let mut c_acc: u8 = 0;
    let mut c_val: u8;
    let mut state: u8 = 0;

    while hex_pos < hex_len {
        c = hex[hex_pos];
        c_num = c ^ 48;
        c_num0 = (((c_num as i32).wrapping_sub(10)) >> 8) as u8;
        c_alpha = ((c & !32).wrapping_sub(55)) as u8;
        c_alpha0 = (((c_alpha as i32).wrapping_sub(10) ^ (c_alpha as i32).wrapping_sub(16)) >> 8) as u8;

        if (c_num0 | c_alpha0) == 0 {
            if let Some(ignore_bytes) = ignore {
                if state == 0 && ignore_bytes.contains(&c) {
                    hex_pos += 1;
                    continue;
                }
            }
            break;
        }

        c_val = (c_num0 & c_num) | (c_alpha0 & c_alpha);

        if bin_pos >= bin_maxlen {
            ret = -1;
            break;
        }

        if state == 0 {
            c_acc = c_val.wrapping_mul(16);
        } else {
            bin[bin_pos] = c_acc | c_val;
            bin_pos += 1;
        }

        state = !state;
        hex_pos += 1;
    }

    if state != 0 {
        hex_pos = hex_pos.saturating_sub(1);
        ret = -1;
    }

    (ret, bin_pos, hex_pos)
}
