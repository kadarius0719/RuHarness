pub fn wcscat(dst: &mut [u32], src: Option<&[u32]>) -> i32 {
    if dst.is_empty() {
        return 22;
    }

    if src.is_none() {
        dst[0] = 0;
        return 22;
    }

    let src_slice = src.unwrap();

    let mut dst_idx = 0;
    while dst_idx < dst.len() && dst[dst_idx] != 0 {
        dst_idx += 1;
    }

    let mut src_idx = 0;
    while dst_idx < dst.len() && src_idx < src_slice.len() {
        dst[dst_idx] = src_slice[src_idx];
        if src_slice[src_idx] == 0 {
            return 0;
        }
        dst_idx += 1;
        src_idx += 1;
    }

    if !dst.is_empty() {
        dst[0] = 0;
    }
    34
}
