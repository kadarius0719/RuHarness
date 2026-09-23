use crate::logic::crc16 as crc16_logic;

#[no_mangle]
pub unsafe extern "C" fn crc16(d: *const u8, len: u32, crc16: u16) -> u16 {
    let slice = core::slice::from_raw_parts(d, len as usize);
    crc16_logic(slice, crc16)
}
