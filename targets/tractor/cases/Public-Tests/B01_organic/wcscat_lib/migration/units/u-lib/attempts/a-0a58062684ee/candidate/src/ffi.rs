#[no_mangle]
pub unsafe extern "C" fn wcscat(dst: *mut u32, numElem: usize, src: *const u32) -> i32 {
    if dst.is_null() || numElem == 0 {
        return 22;
    }

    let dst_slice = std::slice::from_raw_parts_mut(dst, numElem);

    if src.is_null() {
        crate::logic::wcscat(dst_slice, None)
    } else {
        let src_len = {
            let mut len = 0;
            while *src.add(len) != 0 {
                len += 1;
            }
            len + 1
        };

        let src_slice = std::slice::from_raw_parts(src, src_len);
        crate::logic::wcscat(dst_slice, Some(src_slice))
    }
}
