use crate::logic;

#[no_mangle]
pub extern "C" fn ldexp_q2(y: f32, exp_q2: i32) -> f32 {
    logic::ldexp_q2(y, exp_q2)
}
