#[repr(C)]
pub struct CbRgb255 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

fn cb_luminance(r: f32, g: f32, b: f32) -> f32 {
    let r = if r > 0.04045 {
        ((r + 0.055) / 1.055_f32).powf(2.4_f32)
    } else {
        r / 12.92_f32
    };
    let g = if g > 0.04045 {
        ((g + 0.055) / 1.055_f32).powf(2.4_f32)
    } else {
        g / 12.92_f32
    };
    let b = if b > 0.04045 {
        ((b + 0.055) / 1.055_f32).powf(2.4_f32)
    } else {
        b / 12.92_f32
    };
    0.2126_f32 * r + 0.7152_f32 * g + 0.0722_f32 * b
}

fn cb_contrast_ratio(ra: f32, ga: f32, ba: f32, rb: f32, gb: f32, bb: f32) -> f32 {
    let lum_a = cb_luminance(ra, ga, ba);
    let lum_b = cb_luminance(rb, gb, bb);
    let (high, low) = if lum_a > lum_b {
        (lum_a, lum_b)
    } else {
        (lum_b, lum_a)
    };
    high / low
}

pub fn contrast_ratio(a: CbRgb255, b: CbRgb255) -> f32 {
    let ra = (a.r as f32) / 255.0_f32;
    let ga = (a.g as f32) / 255.0_f32;
    let ba = (a.b as f32) / 255.0_f32;
    let rb = (b.r as f32) / 255.0_f32;
    let gb = (b.g as f32) / 255.0_f32;
    let bb = (b.b as f32) / 255.0_f32;

    cb_contrast_ratio(ra, ga, ba, rb, gb, bb)
}
