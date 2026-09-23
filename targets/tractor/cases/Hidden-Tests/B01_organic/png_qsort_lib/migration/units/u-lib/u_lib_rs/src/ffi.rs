use crate::logic::CpIntegerImage;

#[no_mangle]
pub unsafe extern "C" fn qsort(
    items: *mut CpIntegerImage,
    count: i32,
) {
    if count <= 0 {
        return;
    }
    let count_usize = count as usize;
    let items_slice = core::slice::from_raw_parts_mut(items, count_usize);
    crate::logic::qsort(items_slice);
}
