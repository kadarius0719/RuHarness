use crate::logic::{HouseT, run_logic};

extern "C" {
    fn printf(fmt: *const u8, ...) -> i32;
}

pub fn printf_house(floors: i32, bedrooms: i32, bathrooms: f64) {
    unsafe {
        let fmt = b"The house has %d floors, %d bedrooms, and %.1f bathrooms\n\0".as_ptr();
        printf(fmt, floors, bedrooms, bathrooms);
    }
}

static mut THE_HOUSE: HouseT = HouseT {
    floors: 2,
    bedrooms: 5,
    bathrooms: 2.5,
};

#[no_mangle]
pub unsafe extern "C" fn run(extra_bedrooms: i32) {
    run_logic(&mut THE_HOUSE, extra_bedrooms);
}

#[no_mangle]
pub unsafe extern "C" fn driver(x: i32) {
    run(x);
    run(x);
}
