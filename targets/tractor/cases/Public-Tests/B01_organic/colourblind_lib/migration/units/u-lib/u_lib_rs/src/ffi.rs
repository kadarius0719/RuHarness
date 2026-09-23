use crate::logic::{CbImpairment, colourblind as colourblind_logic};

#[no_mangle]
pub unsafe extern "C" fn colourblind(impairment: CbImpairment, r: *mut f32, g: *mut f32, b: *mut f32) {
    colourblind_logic(impairment, &mut *r, &mut *g, &mut *b);
}
