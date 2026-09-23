use crate::logic::{Tflac, flac_validate as flac_validate_logic, tflac_size_memory as tflac_size_memory_logic};

#[no_mangle]
pub unsafe extern "C" fn flac_validate(t: *mut Tflac) -> i32 {
    flac_validate_logic(&mut *t)
}

#[no_mangle]
pub extern "C" fn tflac_size_memory(blocksize: u32) -> u32 {
    tflac_size_memory_logic(blocksize)
}
