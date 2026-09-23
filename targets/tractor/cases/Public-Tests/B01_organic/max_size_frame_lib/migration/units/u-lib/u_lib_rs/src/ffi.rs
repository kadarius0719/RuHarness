use crate::logic;

#[no_mangle]
pub extern "C" fn max_size_frame(blocksize: u32, channels: u32, bitdepth: u32) -> u32 {
    logic::max_size_frame(blocksize, channels, bitdepth)
}
