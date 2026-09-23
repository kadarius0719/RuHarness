fn srgb_channel_to_linear(x: f32) -> f32 {
    let xd = x as f64;
    let result: f64 = if xd > 0.04045 {
        ((xd + 0.055) / 1.055).powf(2.4)
    } else {
        xd / 12.92
    };
    result as f32
}

fn cb_luminance(r: f32, g: f32, b: f32) -> f32 {
    let r2 = srgb_channel_to_linear(r);
    let g2 = srgb_channel_to_linear(g);
    let b2 = srgb_channel_to_linear(b);
    0.2126f32 * r2 + 0.7152f32 * g2 + 0.0722f32 * b2
}

fn cb_contrast_ratio(ra: f32, ga: f32, ba: f32, rb: f32, gb: f32, bb: f32) -> f32 {
    let lum_a = cb_luminance(ra, ga, ba);
    let lum_b = cb_luminance(rb, gb, bb);
    let mut high = lum_a;
    let mut low = lum_b;
    if high < low {
        high = lum_b;
        low = lum_a;
    }
    high / low
}

pub fn contrast_ratio(a_r: u8, a_g: u8, a_b: u8, b_r: u8, b_g: u8, b_b: u8) -> f32 {
    let ra = (a_r as f32) / 255.0f32;
    let ga = (a_g as f32) / 255.0f32;
    let ba = (a_b as f32) / 255.0f32;
    let rb = (b_r as f32) / 255.0f32;
    let gb = (b_g as f32) / 255.0f32;
    let bb = (b_b as f32) / 255.0f32;
    cb_contrast_ratio(ra, ga, ba, rb, gb, bb)
}
