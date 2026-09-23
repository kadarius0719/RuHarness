pub fn mp3d_scale_pcm(sample: f32) -> i16 {
    if sample >= 32766.5 {
        return 32767i16;
    }
    if sample <= -32767.5 {
        return -32768i16;
    }
    let mut s: i16 = (sample + 0.5f32) as i16;
    if s < 0 {
        s -= 1;
    }
    s
}

pub fn synth_pair_inner(pcm: &mut [i16], nch: i32, z: &[f32]) {
    let nch_usize = nch as usize;

    let mut a: f32;
    a = (z[14 * 64] - z[0]) * 29.0;
    a += (z[1 * 64] + z[13 * 64]) * 213.0;
    a += (z[12 * 64] - z[2 * 64]) * 459.0;
    a += (z[3 * 64] + z[11 * 64]) * 2037.0;
    a += (z[10 * 64] - z[4 * 64]) * 5153.0;
    a += (z[5 * 64] + z[9 * 64]) * 6574.0;
    a += (z[8 * 64] - z[6 * 64]) * 37489.0;
    a += z[7 * 64] * 75038.0;
    pcm[0] = mp3d_scale_pcm(a);

    a = z[2 + 14 * 64] * 104.0;
    a += z[2 + 12 * 64] * 1567.0;
    a += z[2 + 10 * 64] * 9727.0;
    a += z[2 + 8 * 64] * 64019.0;
    a += z[2 + 6 * 64] * -9975.0;
    a += z[2 + 4 * 64] * -45.0;
    a += z[2 + 2 * 64] * 146.0;
    a += z[2 + 0 * 64] * -5.0;
    pcm[16 * nch_usize] = mp3d_scale_pcm(a);
}
