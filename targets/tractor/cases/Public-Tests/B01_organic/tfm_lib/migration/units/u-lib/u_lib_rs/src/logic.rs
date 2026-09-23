pub fn tfm_inner(count: i32, dest: &mut [f32], src: &[f32]) {
    let count_usize = count as usize;
    let mut dest_idx = 0;
    let mut src_idx = 0;

    for _ in 0..count_usize {
        if src_idx + 2 >= src.len() || dest_idx + 1 >= dest.len() {
            break;
        }

        if src[src_idx] < src[src_idx + 1] {
            let dx2 = src[src_idx];
            let dy2 = src[src_idx + 1];
            let dxy = src[src_idx + 2];
            let sqd = (dy2 * dy2) - (2.0 * dx2 * dy2) + (dx2 * dx2) + (4.0 * dxy * dxy);
            let lambda = 0.5 * (dy2 + dx2 + sqd.max(0.0).sqrt());
            dest[dest_idx] = dx2 - lambda;
            dest[dest_idx + 1] = dxy;
        } else {
            let dy2 = src[src_idx];
            let dx2 = src[src_idx + 1];
            let dxy = src[src_idx + 2];
            let sqd = (dy2 * dy2) - (2.0 * dx2 * dy2) + (dx2 * dx2) + (4.0 * dxy * dxy);
            let lambda = 0.5 * (dy2 + dx2 + sqd.max(0.0).sqrt());
            dest[dest_idx] = dxy;
            dest[dest_idx + 1] = dx2 - lambda;
        }
        src_idx += 3;
        dest_idx += 2;
    }
}
