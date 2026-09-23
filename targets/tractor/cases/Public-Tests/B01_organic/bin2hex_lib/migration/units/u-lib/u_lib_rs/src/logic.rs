pub fn bin2hex(hex: &mut [u8], bin: &[u8]) {
    const SIZE_MAX: u64 = 18446744073709551615;

    let bin_len = bin.len() as u64;
    let hex_maxlen = hex.len() as u64;

    if bin_len >= SIZE_MAX / 2 || hex_maxlen <= bin_len * 2 {
        panic!("bin2hex: buffer overflow");
    }

    for i in 0..bin.len() {
        let c = (bin[i] & 0xf) as u32;
        let b = ((bin[i] >> 4) & 0xf) as u32;

        let c_val = 87u32.wrapping_add(c).wrapping_add(
            ((c.wrapping_sub(10)) >> 8) & !38u32
        );
        let b_val = 87u32.wrapping_add(b).wrapping_add(
            ((b.wrapping_sub(10)) >> 8) & !38u32
        );

        let x = (((c_val as u8) as u32) << 8) | ((b_val as u8) as u32);

        hex[i * 2] = (x & 0xFF) as u8;
        hex[i * 2 + 1] = ((x >> 8) & 0xFF) as u8;
    }

    hex[bin.len() * 2] = 0;
}
