pub fn normalize(dest: &mut [f32], src: &[f32]) {
    let size = src.len().min(dest.len());
    let mut sum = 0.0f32;

    for i in 0..size {
        sum += src[i] * src[i];
    }

    if sum > 0.0f32 {
        sum = 1.0f32 / sum.sqrt();
        for i in 0..size {
            dest[i] = src[i] * sum;
        }
    } else if dest.as_ptr() != src.as_ptr() {
        for i in 0..size {
            dest[i] = 0.0f32;
        }
    }
}
