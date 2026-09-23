// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        x: c_int,
        y: c_int,
    },
    library: "driver",
    symbol: "driver",
    signature: unsafe extern "C" fn(c_int, c_int),

    fn run(&mut self) {
        unsafe {
            (*SYMBOL)(
                self.x,
                self.y
            )
        };
    }
}

