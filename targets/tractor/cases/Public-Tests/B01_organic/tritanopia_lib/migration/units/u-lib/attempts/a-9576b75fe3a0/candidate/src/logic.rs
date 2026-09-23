#[repr(C)]
pub struct cb_rgb_255 {
    pub R: u8,
    pub G: u8,
    pub B: u8,
}

pub fn tritanopia(rgb: cb_rgb_255) -> cb_rgb_255 {
    let r_norm = (rgb.R as f32) / 255.0;
    let g_norm = (rgb.G as f32) / 255.0;
    let b_norm = (rgb.B as f32) / 255.0;

    let r_removed = if r_norm > 0.04045 {
        ((r_norm + 0.055) / 1.055).powf(2.4)
    } else {
        r_norm / 12.92
    };
    let g_removed = if g_norm > 0.04045 {
        ((g_norm + 0.055) / 1.055).powf(2.4)
    } else {
        g_norm / 12.92
    };
    let b_removed = if b_norm > 0.04045 {
        ((b_norm + 0.055) / 1.055).powf(2.4)
    } else {
        b_norm / 12.92
    };

    let mut r_tri = r_removed;
    let mut g_tri = g_removed;
    let mut b_tri = b_removed;

    let old_r = r_tri;
    let old_g = g_tri;
    let old_b = b_tri;

    r_tri = old_r + 0.12739886310880 * old_g - 0.12739886341072 * old_b;
    g_tri = -4.486E-11 * old_r + 0.87390929928361 * old_g + 0.12609070101523 * old_b;
    b_tri = 3.1113E-10 * old_r + 0.87390929725848 * old_g + 0.12609070067115 * old_b;

    let r_applied = if r_tri > 0.00313080495356037151702786377709 {
        1.055 * r_tri.powf(0.4166666666) - 0.055
    } else {
        r_tri * 12.92
    };
    let g_applied = if g_tri > 0.00313080495356037151702786377709 {
        1.055 * g_tri.powf(0.4166666666) - 0.055
    } else {
        g_tri * 12.92
    };
    let b_applied = if b_tri > 0.00313080495356037151702786377709 {
        1.055 * b_tri.powf(0.4166666666) - 0.055
    } else {
        b_tri * 12.92
    };

    let r_denorm = ((r_applied * 255.0 + 0.5) as u8);
    let g_denorm = ((g_applied * 255.0 + 0.5) as u8);
    let b_denorm = ((b_applied * 255.0 + 0.5) as u8);

    cb_rgb_255 {
        R: r_denorm,
        G: g_denorm,
        B: b_denorm,
    }
}
