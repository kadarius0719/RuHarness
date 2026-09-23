use crate::logic;

#[no_mangle]
pub unsafe extern "C" fn merge_sort(a: *mut logic::Sprite, b: *mut logic::Sprite, size: i32) {
    if size < 0 {
        return;
    }

    let size_usize = size as usize;
    let a_slice = std::slice::from_raw_parts_mut(a, size_usize);
    let b_slice = std::slice::from_raw_parts_mut(b, size_usize);

    logic::merge_sort(a_slice, b_slice);
}
