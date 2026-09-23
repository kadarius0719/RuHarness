// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        x: c_uint,
        y: c_uint,
        b: bool,
        z: c_int,
    },
    library: "driver",
    symbol: "driver",
    signature: unsafe extern "C" fn(c_uint, c_uint, bool, c_int),

    fn run(&mut self) {
        unsafe {
            (*SYMBOL)(
                self.x,
                self.y,
                self.b,
                self.z
            )
        };
    }
}

