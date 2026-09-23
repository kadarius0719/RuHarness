pub fn gaussian_kernel(dest: &mut [f32], size: i32, radius: f32) {
    let sigma = 1.6f32;
    let theta = 2.25f32;
    let hsize = size / 2;

    let s2 = 1.0f32 / (sigma * sigma * theta).exp();
    let rs = sigma / radius;

    let mut sum = 0.0f32;

    for r in -hsize..=hsize {
        let idx = (r + hsize) as usize;
        let x = r as f32 * rs;
        let v = (1.0f32 / (x * x).exp()) - s2;
        let v = if v > 0.0f32 { v } else { 0.0f32 };
        if idx < dest.len() {
            dest[idx] = v;
            sum += v;
        }
    }

    if sum > 0.0f32 {
        let isum = 1.0f32 / sum;
        for i in 0..dest.len() {
            dest[i] *= isum;
        }
    }
}
