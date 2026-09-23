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

pub fn premultiply_inner(pixels: &mut [CpPixelT]) {
    for pixel in pixels {
        let a = pixel.a as f32 / 255.0;
        let r = pixel.r as f32 / 255.0;
        let g = pixel.g as f32 / 255.0;
        let b = pixel.b as f32 / 255.0;

        pixel.r = ((r * a) * 255.0) as u8;
        pixel.g = ((g * a) * 255.0) as u8;
        pixel.b = ((b * a) * 255.0) as u8;
    }
}
