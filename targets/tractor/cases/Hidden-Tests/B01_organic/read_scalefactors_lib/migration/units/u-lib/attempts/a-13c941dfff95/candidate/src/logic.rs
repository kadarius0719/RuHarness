const G_DEQ_L12: [f32; 54] = [
    9.53674316e-07_f32 / 3.0, 7.56931807e-07_f32 / 3.0, 6.00777173e-07_f32 / 3.0,
    9.53674316e-07_f32 / 7.0, 7.56931807e-07_f32 / 7.0, 6.00777173e-07_f32 / 7.0,
    9.53674316e-07_f32 / 15.0, 7.56931807e-07_f32 / 15.0, 6.00777173e-07_f32 / 15.0,
    9.53674316e-07_f32 / 31.0, 7.56931807e-07_f32 / 31.0, 6.00777173e-07_f32 / 31.0,
    9.53674316e-07_f32 / 63.0, 7.56931807e-07_f32 / 63.0, 6.00777173e-07_f32 / 63.0,
    9.53674316e-07_f32 / 127.0, 7.56931807e-07_f32 / 127.0, 6.00777173e-07_f32 / 127.0,
    9.53674316e-07_f32 / 255.0, 7.56931807e-07_f32 / 255.0, 6.00777173e-07_f32 / 255.0,
    9.53674316e-07_f32 / 511.0, 7.56931807e-07_f32 / 511.0, 6.00777173e-07_f32 / 511.0,
    9.53674316e-07_f32 / 1023.0, 7.56931807e-07_f32 / 1023.0, 6.00777173e-07_f32 / 1023.0,
    9.53674316e-07_f32 / 2047.0, 7.56931807e-07_f32 / 2047.0, 6.00777173e-07_f32 / 2047.0,
    9.53674316e-07_f32 / 4095.0, 7.56931807e-07_f32 / 4095.0, 6.00777173e-07_f32 / 4095.0,
    9.53674316e-07_f32 / 8191.0, 7.56931807e-07_f32 / 8191.0, 6.00777173e-07_f32 / 8191.0,
    9.53674316e-07_f32 / 16383.0, 7.56931807e-07_f32 / 16383.0, 6.00777173e-07_f32 / 16383.0,
    9.53674316e-07_f32 / 32767.0, 7.56931807e-07_f32 / 32767.0, 6.00777173e-07_f32 / 32767.0,
    9.53674316e-07_f32 / 65535.0, 7.56931807e-07_f32 / 65535.0, 6.00777173e-07_f32 / 65535.0,
    9.53674316e-07_f32 / 3.0, 7.56931807e-07_f32 / 3.0, 6.00777173e-07_f32 / 3.0,
    9.53674316e-07_f32 / 5.0, 7.56931807e-07_f32 / 5.0, 6.00777173e-07_f32 / 5.0,
    9.53674316e-07_f32 / 9.0, 7.56931807e-07_f32 / 9.0, 6.00777173e-07_f32 / 9.0,
];

pub fn read_scalefactors<F: FnMut(i32) -> u32>(
    pba: &[u8],
    scfcod: &[u8],
    bands: i32,
    scf: &mut [f32],
    mut get_bits: F,
) {
    let mut i: i32 = 0;
    let mut scf_idx: usize = 0;
    while i < bands {
        let mut s: f32 = 0.0;
        let ba_byte = pba.get(i as usize).copied().unwrap_or(0);
        let ba: i32 = ba_byte as i32;
        // The C computes `mask` with `ba ? 4 + ((19 >> scfcod[i]) & 3) : 0`,
        // a ternary that short-circuits: scfcod[i] is read ONLY when ba != 0.
        // Mirror that laziness exactly so we never touch scfcod at indices
        // the C leaves alone (e.g. a run of trailing bands with ba == 0).
        let mask: i32 = if ba != 0 {
            let scfcod_val = scfcod.get(i as usize).copied().unwrap_or(0);
            4 + (19i32.wrapping_shr(scfcod_val as u32) & 3)
        } else {
            0
        };
        let mut m: i32 = 4;
        while m != 0 {
            if (mask & m) != 0 {
                let b: u32 = get_bits(6);
                let idx = ba.wrapping_mul(3).wrapping_sub(6).wrapping_add((b % 3) as i32);
                s = if idx >= 0 && (idx as usize) < G_DEQ_L12.len() {
                    G_DEQ_L12[idx as usize] * ((1i32 << 21).wrapping_shr(b / 3) as f32)
                } else {
                    0.0
                };
            }
            if let Some(slot) = scf.get_mut(scf_idx) {
                *slot = s;
            }
            scf_idx += 1;
            m >>= 1;
        }
        i = i.wrapping_add(1);
    }
}
