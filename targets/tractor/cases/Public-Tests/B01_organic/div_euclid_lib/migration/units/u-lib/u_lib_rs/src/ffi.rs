use crate::logic::div_euclid as div_euclid_logic;

#[no_mangle]
pub unsafe extern "C" fn div_euclid(v1: i32, v2: i32) -> i32 {
    div_euclid_logic(v1, v2)
}
