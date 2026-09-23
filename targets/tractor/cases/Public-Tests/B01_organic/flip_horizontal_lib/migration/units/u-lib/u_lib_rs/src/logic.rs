#[repr(C)]
pub struct CpPixelT {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

#[repr(C)]
pub struct CpImageT {
    pub w: i32,
    pub h: i32,
    pub pix: *mut CpPixelT,
}

pub fn flip_horizontal_impl(pixels: &mut [CpPixelT], w: usize, h: usize) {
    let flips = h / 2;

    for i in 0..flips {
        let a_offset = w * i;
        let b_offset = w * (h - i - 1);

        for j in 0..w {
            let idx_a = a_offset + j;
            let idx_b = b_offset + j;

            if idx_a < pixels.len() && idx_b < pixels.len() {
                pixels.swap(idx_a, idx_b);
            }
        }
    }
}
