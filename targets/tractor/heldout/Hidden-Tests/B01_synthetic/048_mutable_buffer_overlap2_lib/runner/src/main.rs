// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        data: Vec<c_int>,
        len: c_int
    },
    library: "driver",
    symbol: "driver",

    signature: unsafe extern "C" fn(*const c_int, c_int),

    fn run(&mut self) {
        unsafe {
            (*SYMBOL)(
                self.data.as_ptr(),
                self.len
            )
        };
    }

}
