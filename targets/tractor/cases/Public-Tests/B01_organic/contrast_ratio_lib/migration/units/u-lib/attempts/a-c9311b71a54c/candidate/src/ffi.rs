use crate::logic::{CbRgb255, contrast_ratio as contrast_ratio_logic};

#[no_mangle]
pub extern "C" fn contrast_ratio(a: CbRgb255, b: CbRgb255) -> f32 {
    contrast_ratio_logic(a, b)
}
