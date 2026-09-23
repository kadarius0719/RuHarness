use crate::logic;

#[no_mangle]
pub unsafe extern "C" fn bitwriter_add(
    bw: *mut logic::BitWriter,
    bits: u32,
    val: u64,
) -> i32 {
    if bw.is_null() {
        return 0;
    }

    logic::bitwriter_add(&mut *bw, bits, val)
}
