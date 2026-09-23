// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

state_member! {
    struct cp_v2i_t {
        x: c_int,
        y: c_int,
    }
}

state_member! {
    struct cp_integer_image_t {
        img_index: c_int,
        size: cp_v2i_t,
        min: cp_v2i_t,
        max: cp_v2i_t,
        fit: c_int,
    }
}

harness! {
    state: {
        items: Vec<cp_integer_image_t>,
        count: c_int,
    },

    library: "png_qsort_lib",
    symbol: "qsort",

    signature: unsafe extern "C" fn(*mut cp_integer_image_t, c_int),

    fn run(&mut self) {
        self.count = self.items.len() as i32;
        unsafe {
            (*SYMBOL)(
                self.items.as_mut_ptr(),
                self.count,
            )
        }

    }
}
