pub fn rgb_to_hsv(dest: &mut [f32], src: &[f32]) {
    if dest.len() < 3 || src.len() < 3 {
        return;
    }

    let r = src[0];
    let g = src[1];
    let b = src[2];

    let min = if r < g { if r < b { r } else { b } } else { if g < b { g } else { b } };
    let max = if r > g { if r > b { r } else { b } } else { if g > b { g } else { b } };

    let delta = max - min;
    let v = max;

    if delta == 0.0 || max == 0.0 {
        dest[0] = 0.0;
        dest[1] = 0.0;
        dest[2] = v;
        return;
    }

    let s = delta / max;
    let mut h = if r == max {
        (g - b) / delta
    } else if g == max {
        2.0 + (b - r) / delta
    } else {
        4.0 + (r - g) / delta
    };

    h *= 60.0;
    if h < 0.0 {
        h += 360.0;
    }

    dest[0] = h;
    dest[1] = s;
    dest[2] = v;
}
