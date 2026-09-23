#[repr(C)]
pub struct Btac1cIdxstate {
    pub idx: u16,
    pub lpred: i16,
    pub rpred: i16,
    pub tag: u8,
    pub bcfcn: u8,
    pub bsfcn: u8,
    pub usefx: u8,
    pub firfx: [[i16; 8]; 4],
}

pub fn predict_sample(psamp: &[i32], idx: i32, mut pfcn: i32, ridx: &Btac1cIdxstate) -> i32 {
    pfcn %= 17;
    let i = idx;

    match pfcn {
        0 => psamp[((i - 1) & 7) as usize],
        1 => 2 * psamp[((i - 1) & 7) as usize] - psamp[((i - 2) & 7) as usize],
        2 => (3 * psamp[((i - 1) & 7) as usize] - psamp[((i - 2) & 7) as usize]) >> 1,
        3 => (5 * psamp[((i - 1) & 7) as usize] - psamp[((i - 2) & 7) as usize]) >> 2,
        4 => {
            let p0 = psamp[((i - 1) & 7) as usize] + psamp[((i - 2) & 7) as usize];
            let p1 = psamp[((i - 2) & 7) as usize] + psamp[((i - 3) & 7) as usize];
            p0 - (p1 >> 1)
        }
        5 => {
            let p0 = psamp[((i - 1) & 7) as usize] + psamp[((i - 2) & 7) as usize];
            let p1 = psamp[((i - 2) & 7) as usize] + psamp[((i - 3) & 7) as usize];
            (3 * p0 - p1) >> 2
        }
        6 => {
            let p0 = psamp[((i - 1) & 7) as usize] + psamp[((i - 2) & 7) as usize];
            let p1 = psamp[((i - 2) & 7) as usize] + psamp[((i - 3) & 7) as usize];
            (5 * p0 - p1) >> 3
        }
        7 => {
            (18 * psamp[((i - 1) & 7) as usize] - 4 * psamp[((i - 2) & 7) as usize] +
                3 * psamp[((i - 3) & 7) as usize] - 2 * psamp[((i - 4) & 7) as usize] +
                1 * psamp[((i - 5) & 7) as usize]) / 16
        }
        8 => {
            (72 * psamp[((i - 1) & 7) as usize] - 16 * psamp[((i - 2) & 7) as usize] +
                12 * psamp[((i - 3) & 7) as usize] - 8 * psamp[((i - 4) & 7) as usize] +
                5 * psamp[((i - 5) & 7) as usize] - 3 * psamp[((i - 6) & 7) as usize] +
                3 * psamp[((i - 7) & 7) as usize] - 1 * psamp[((i - 8) & 7) as usize]) / 64
        }
        9 => {
            (76 * psamp[((i - 1) & 7) as usize] - 17 * psamp[((i - 2) & 7) as usize] +
                10 * psamp[((i - 3) & 7) as usize] - 7 * psamp[((i - 4) & 7) as usize] +
                5 * psamp[((i - 5) & 7) as usize] - 4 * psamp[((i - 6) & 7) as usize] +
                4 * psamp[((i - 7) & 7) as usize] - 3 * psamp[((i - 8) & 7) as usize]) / 64
        }
        10 => {
            let p0 = psamp[((i - 1) & 7) as usize] + psamp[((i - 2) & 7) as usize] + psamp[((i - 3) & 7) as usize] +
                psamp[((i - 4) & 7) as usize];
            let p1 = psamp[((i - 5) & 7) as usize] + psamp[((i - 6) & 7) as usize] + psamp[((i - 7) & 7) as usize] +
                psamp[((i - 8) & 7) as usize];
            (5 * p0 - p1) >> 4
        }
        11 => {
            let p0 = psamp[((i - 1) & 7) as usize] + psamp[((i - 2) & 7) as usize] + psamp[((i - 3) & 7) as usize] +
                psamp[((i - 4) & 7) as usize];
            let p1 = psamp[((i - 5) & 7) as usize] + psamp[((i - 6) & 7) as usize] + psamp[((i - 7) & 7) as usize] +
                psamp[((i - 8) & 7) as usize];
            (p0 + p1) >> 3
        }
        12 | 13 | 14 | 15 => {
            (ridx.firfx[(pfcn - 12) as usize][0] as i32 * psamp[((i - 1) & 7) as usize] +
                ridx.firfx[(pfcn - 12) as usize][1] as i32 * psamp[((i - 2) & 7) as usize] +
                ridx.firfx[(pfcn - 12) as usize][2] as i32 * psamp[((i - 3) & 7) as usize] +
                ridx.firfx[(pfcn - 12) as usize][3] as i32 * psamp[((i - 4) & 7) as usize] +
                ridx.firfx[(pfcn - 12) as usize][4] as i32 * psamp[((i - 5) & 7) as usize] +
                ridx.firfx[(pfcn - 12) as usize][5] as i32 * psamp[((i - 6) & 7) as usize] +
                ridx.firfx[(pfcn - 12) as usize][6] as i32 * psamp[((i - 7) & 7) as usize] +
                ridx.firfx[(pfcn - 12) as usize][7] as i32 * psamp[((i - 8) & 7) as usize]) / 256
        }
        _ => 0,
    }
}
