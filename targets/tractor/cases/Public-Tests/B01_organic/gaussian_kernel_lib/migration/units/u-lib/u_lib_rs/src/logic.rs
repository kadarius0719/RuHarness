pub fn gaussian_kernel(dest: &mut [f32], size: i32, radius: f32) {
    let sigma: f32 = 1.6f32;
    let tetha: f32 = 2.25f32;
    let hsize: i32 = size / 2;
    let s2: f32 = 1.0f32 / (sigma * sigma * tetha).exp();
    let rs: f32 = sigma / radius;
    let mut sum: f32 = 0.0f32;

    if hsize >= 0 {
        let mut r: i32 = -hsize;
        while r <= hsize {
            let x: f32 = (r as f32) * rs;
            let mut v: f32 = (1.0f32 / (x * x).exp()) - s2;
            v = if v > 0.0f32 { v } else { 0.0f32 };
            let idx = (r + hsize) as usize;
            dest[idx] = v;
            sum += v;
            r += 1;
        }
    }

    if sum > 0.0f32 {
        let isum: f32 = 1.0f32 / sum;
        if size > 0 {
            let n = size as usize;
            let mut r: usize = 0;
            while r < n {
                dest[r] *= isum;
                r += 1;
            }
        }
    }
}
