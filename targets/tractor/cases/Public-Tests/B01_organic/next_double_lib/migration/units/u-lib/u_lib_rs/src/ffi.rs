use crate::logic;

#[no_mangle]
pub unsafe extern "C" fn next_double(rnd: *mut logic::Rnd) -> f64 {
    logic::next_double(&mut *rnd)
}
